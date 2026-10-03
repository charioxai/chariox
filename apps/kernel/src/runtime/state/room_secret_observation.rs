//! MP-08/MP-10/MP-11: Room-scoped observation protection, below every client.
//! Values live only in zeroizing memory. Durable markers contain no values.
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::KernelRuntimeState;
use crate::error::DaemonError;
use crate::transport::room_browser_controller::{
    RoomBrowserControllerCommand as Command, RoomBrowserControllerResult as Response,
};

#[derive(Clone)]
pub(super) struct RoomSecretObservations {
    root: PathBuf,
    worker_room: Option<String>,
    recovered: Arc<BTreeSet<String>>,
    rooms: Arc<Mutex<BTreeMap<String, Protection>>>,
    barriers: Arc<Mutex<BTreeMap<String, Arc<tokio::sync::RwLock<()>>>>>,
    pub(super) epoch: u64,
}

#[derive(Default)]
struct Protection {
    values: Vec<Zeroizing<String>>,
    unknown: bool,
    pixels_withheld: bool,
    revision: u64,
    recovered_artifacts: bool,
    controller_generation: Option<u64>,
}

impl RoomSecretObservations {
    pub(super) fn new(root: PathBuf, recovered: BTreeSet<String>) -> Self {
        Self {
            root,
            worker_room: None,
            recovered: Arc::new(recovered),
            rooms: Default::default(),
            barriers: Default::default(),
            epoch: crate::session::unix_epoch_ms(),
        }
    }

    // A provisioner-bound slice executes only this home Room, even when its
    // provider runs and transcripts carry worker-local session identifiers.
    pub(super) fn with_worker_room(mut self, room: Option<String>) -> Self {
        if let Some(room) = &room {
            if !self.recovered.is_empty() {
                self.recovered = Arc::new(BTreeSet::from([room.clone()]));
            }
        }
        self.worker_room = room;
        self
    }

    fn room_key<'a>(&'a self, room: &'a str) -> &'a str {
        self.worker_room.as_deref().unwrap_or(room)
    }

    pub(super) fn barrier(&self, room: &str) -> Result<Arc<tokio::sync::RwLock<()>>, DaemonError> {
        let room = self.room_key(room);
        Ok(self
            .barriers
            .lock()
            .map_err(|_| protection_error())?
            .entry(room.into())
            .or_default()
            .clone())
    }

    fn marker(&self, room: &str) -> PathBuf {
        let room = self.room_key(room);
        self.root
            .join(format!("{:x}", Sha256::digest(room.as_bytes())))
    }

    fn initial(&self, room: &str) -> Result<Protection, DaemonError> {
        // Missing is the only clean disk state. Permission and I/O failures close observations.
        let marked = match std::fs::symlink_metadata(self.marker(room)) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => return Err(protection_error()),
        };
        let unknown = marked || self.recovered.contains(room);
        Ok(Protection {
            unknown,
            pixels_withheld: unknown,
            recovered_artifacts: unknown,
            ..Default::default()
        })
    }

    pub(super) fn register(&self, room: &str, value: &str) -> Result<(), DaemonError> {
        let room = self.room_key(room);
        let mut rooms = self.rooms.lock().map_err(|_| protection_error())?;
        if !rooms.contains_key(room) {
            rooms.insert(room.into(), self.initial(room)?);
        }
        let protection = rooms.get_mut(room).ok_or_else(protection_error)?;
        // Set the memory fence even if persisting the marker fails. Input must then abort.
        protection.pixels_withheld = true;
        protection.revision = protection.revision.saturating_add(1);
        let known = protection
            .values
            .iter()
            .any(|existing| existing.as_str() == value);
        if value.is_empty() || (!known && protection.values.len() >= 256) {
            protection.unknown = true;
            return Err(protection_error());
        }
        if !known {
            protection.values.push(Zeroizing::new(value.to_string()));
        }
        self.persist_marker(room)?;
        Ok(())
    }

    fn persist_marker(&self, room: &str) -> Result<(), DaemonError> {
        use std::io::Write;
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.root)
            .map_err(|_| protection_error())?;
        let marker = self.marker(room);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&marker)
        {
            Ok(mut file) => {
                file.write_all(b"observation clearance required\n")
                    .map_err(|_| protection_error())?;
                file.sync_all().map_err(|_| protection_error())?;
                std::fs::File::open(&self.root)
                    .and_then(|dir| dir.sync_all())
                    .map_err(|_| protection_error())?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let file = std::fs::OpenOptions::new()
                    .write(true)
                    .custom_flags(libc::O_NOFOLLOW)
                    .open(&marker)
                    .map_err(|_| protection_error())?;
                if !file.metadata().map_err(|_| protection_error())?.is_file() {
                    return Err(protection_error());
                }
                file.sync_all().map_err(|_| protection_error())?;
                std::fs::File::open(&self.root)
                    .and_then(|dir| dir.sync_all())
                    .map_err(|_| protection_error())?;
            }
            Err(_) => return Err(protection_error()),
        }
        Ok(())
    }

    pub(super) fn quarantine(&self, room: &str) -> Result<(), DaemonError> {
        let room = self.room_key(room);
        self.blocked(room, false)?;
        let mut rooms = self.rooms.lock().map_err(|_| protection_error())?;
        let protection = rooms.get_mut(room).ok_or_else(protection_error)?;
        protection.unknown = true;
        protection.pixels_withheld = true;
        protection.recovered_artifacts = true;
        self.persist_marker(room)
    }

    pub(super) fn blocked(&self, room: &str, pixels: bool) -> Result<bool, DaemonError> {
        let room = self.room_key(room);
        let mut rooms = self.rooms.lock().map_err(|_| protection_error())?;
        if !rooms.contains_key(room) {
            rooms.insert(room.into(), self.initial(room)?);
        }
        let protection = &rooms[room];
        Ok(protection.unknown || (pixels && protection.pixels_withheld))
    }

    pub(super) fn require(&self, room: &str, pixels: bool) -> Result<(), DaemonError> {
        if self.blocked(room, pixels)? {
            return Err(protection_error());
        }
        Ok(())
    }

    // Caller holds the exclusive barrier after the home-owned human interaction.
    pub(super) fn clear(&self, room: &str) -> Result<(), DaemonError> {
        let room = self.room_key(room);
        let mut rooms = self.rooms.lock().map_err(|_| protection_error())?;
        if !rooms.contains_key(room) {
            rooms.insert(room.into(), self.initial(room)?);
        }
        let protection = rooms.get_mut(room).ok_or_else(protection_error)?;
        self.persist_marker(room)?;
        protection.unknown = false;
        protection.pixels_withheld = false;
        protection.revision = protection.revision.saturating_add(1);
        // Keep marker and known values: restart/restore needs another clearance,
        // and navigation/clearance cannot allow delayed text echoes to escape.
        Ok(())
    }

    pub(super) fn revision(&self, room: &str) -> Result<u64, DaemonError> {
        let room = self.room_key(room);
        self.blocked(room, false)?;
        Ok(self.rooms.lock().map_err(|_| protection_error())?[room].revision)
    }

    pub(super) fn scrub_error(&self, room: &str, error: DaemonError) -> DaemonError {
        let original = error.to_string();
        match self.scrub(room, original.clone()) {
            Ok(message) if message == original => error,
            Ok(message) => DaemonError::LocalTransport {
                operation: "room.observation",
                message,
            },
            Err(_) => protection_error(),
        }
    }

    pub(super) fn scrub_text_or_withhold(&self, room: &str, text: &str) -> String {
        self.scrub(room, text.to_string())
            .unwrap_or_else(|_| "[sensitive Room observation withheld]".into())
    }

    pub(super) fn protects_bytes(&self, room: &str) -> bool {
        let room = self.room_key(room);
        if self.blocked(room, false).is_err() {
            return true;
        }
        self.rooms
            .lock()
            .map(|rooms| {
                let protection = &rooms[room];
                protection.unknown || !protection.values.is_empty()
            })
            .unwrap_or(true)
    }

    pub(super) fn scrub<T: Serialize + DeserializeOwned>(
        &self,
        room: &str,
        input: T,
    ) -> Result<T, DaemonError> {
        let room = self.room_key(room);
        self.require(room, false)?;
        let rooms = self.rooms.lock().map_err(|_| protection_error())?;
        let protection = &rooms[room];
        // Recheck under the value lock: quarantine may follow the first require.
        if protection.unknown {
            return Err(protection_error());
        }
        if protection.values.is_empty() {
            return Ok(input);
        }
        let mut value = serde_json::to_value(input).map_err(|_| protection_error())?;
        scrub_value(&mut value, &protection.values);
        serde_json::from_value(value).map_err(|_| protection_error())
    }

    pub(super) fn protect_transcript_entry(
        &self,
        mut entry: crate::history::SessionHistoryEntry,
    ) -> crate::history::SessionHistoryEntry {
        use crate::history::SessionHistoryEntryKind;
        if matches!(
            entry.kind,
            SessionHistoryEntryKind::ProviderOutput | SessionHistoryEntryKind::ProviderReasoning
        ) && self.protects_bytes(&entry.session_id)
        {
            entry.text = "[sensitive Room terminal stream withheld]".into();
        }
        match self.scrub(&entry.session_id, entry.clone()) {
            Ok(entry) => entry,
            Err(_) => {
                entry.text = "[sensitive Room observation withheld]".into();
                entry.attachments.clear();
                entry
            }
        }
    }

    pub(super) fn scrub_cached_result<T: Serialize + DeserializeOwned>(
        &self,
        room: &str,
        value: T,
    ) -> Result<T, DaemonError> {
        let room = self.room_key(room);
        self.require(room, false)?;
        if self.rooms.lock().map_err(|_| protection_error())?[room].recovered_artifacts {
            return Err(DaemonError::LocalTransport {
                operation: "room.observation.replay",
                message:
                    "Recovered observation caches cannot be replayed. Request a fresh invocation."
                        .into(),
            });
        }
        self.scrub(room, value)
    }

    pub(super) fn protect_unframed_bytes(&self, room: &str, bytes: &[u8]) -> Vec<u8> {
        if self.protects_bytes(room) {
            // Matching individual chunks cannot catch a secret split across chunks.
            // Retain the fence for the session lifetime, including after clearance.
            b"[sensitive Room terminal stream withheld]".to_vec()
        } else {
            bytes.to_vec()
        }
    }

    pub(super) fn scrub_response(
        &self,
        room: &str,
        mut response: Response,
    ) -> Result<Response, DaemonError> {
        let room = self.room_key(room);
        // A replacement controller has lost its pre-compaction value registry.
        // Do not accept bounded page strings until the human clears that view.
        self.blocked(room, false)?;
        {
            let mut rooms = self.rooms.lock().map_err(|_| protection_error())?;
            let protection = rooms.get_mut(room).ok_or_else(protection_error)?;
            match &response {
                Response::RecoveryRequired { .. } if !protection.values.is_empty() => {
                    protection.unknown = true;
                    protection.pixels_withheld = true;
                }
                Response::Reconciled {
                    reconciliation: Some(reconciliation),
                } => {
                    let generation = reconciliation.process.runtime_generation;
                    if protection
                        .controller_generation
                        .is_some_and(|old| old != generation)
                        && !protection.values.is_empty()
                    {
                        protection.unknown = true;
                        protection.pixels_withheld = true;
                    }
                    protection.controller_generation = Some(generation);
                }
                _ => {}
            }
        }
        match &mut response {
            Response::ComputerInputApplied { .. }
            | Response::ComputerSecretTarget { .. }
            | Response::SecretObservationCleared
            | Response::Process { .. }
            | Response::RecoveryRequired { .. }
            | Response::ActionCancelled { .. }
            | Response::CancellationRequested { .. }
            | Response::CookieImportRecovered
            | Response::CookieImportRolledBack => Ok(response),
            Response::Reconciled { reconciliation } if self.blocked(room, false)? => {
                // Recovery still needs physical identities; page-controlled metadata
                // must not enter the durable Room registry without clearance.
                if let Some(reconciliation) = reconciliation {
                    for tab in &mut reconciliation.browser.tabs {
                        tab.url.clear();
                        tab.title = "[withheld]".into();
                    }
                }
                Ok(response)
            }
            _ => self.scrub(room, response),
        }
    }

    pub(super) fn protect_history_events(
        &self,
        mut events: Vec<crate::history::HistoryEvent>,
    ) -> Vec<crate::history::HistoryEvent> {
        for event in &mut events {
            let Some(room) = event.session_id.clone() else {
                continue;
            };
            let room = self.room_key(&room);
            let recovered = self.blocked(room, false).is_err()
                || self
                    .rooms
                    .lock()
                    .map(|rooms| rooms[room].recovered_artifacts)
                    .unwrap_or(true);
            if recovered && event.timestamp_ms <= self.epoch {
                event.content = Some("[recovered sensitive Room history withheld]".into());
                event.content_ref = None;
                event.metadata.clear();
            } else if matches!(
                event.kind,
                crate::history::HistoryEventKind::ProviderOutput
                    | crate::history::HistoryEventKind::ProviderReasoning
            ) && self.protects_bytes(room)
            {
                event.content = Some("[sensitive Room terminal stream withheld]".into());
                event.content_ref = None;
                event.metadata.clear();
            } else if let Ok(scrubbed) = self.scrub(room, event.clone()) {
                *event = scrubbed;
            } else {
                event.content = Some("[sensitive Room history withheld]".into());
                event.content_ref = None;
                event.metadata.clear();
            }
        }
        events
    }
}

fn scrub_value(value: &mut serde_json::Value, secrets: &[Zeroizing<String>]) {
    match value {
        serde_json::Value::String(text) => {
            let mut variants = Vec::new();
            for secret in secrets {
                variants.extend(secret_variants(secret));
            }
            variants.sort_by_key(|variant| std::cmp::Reverse(variant.len()));
            for variant in variants {
                *text = text.replace(variant.as_str(), "[redacted]");
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                scrub_value(value, secrets);
            }
        }
        serde_json::Value::Object(values) => {
            let entries = std::mem::take(values);
            for (key, mut value) in entries {
                let mut key = serde_json::Value::String(key);
                scrub_value(&mut key, secrets);
                scrub_value(&mut value, secrets);
                values.insert(key.as_str().unwrap_or("[redacted]").into(), value);
            }
        }
        _ => {}
    }
}

fn secret_variants(secret: &str) -> Vec<Zeroizing<String>> {
    let percent = secret
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect::<String>();
    let hex = secret
        .bytes()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let json = serde_json::to_string(secret).unwrap_or_default();
    let html = secret
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;");
    vec![
        secret.to_string(),
        secret.to_lowercase(),
        secret.to_uppercase(),
        percent.clone(),
        percent.to_lowercase(),
        hex.clone(),
        hex.to_uppercase(),
        html,
        json[1..json.len().saturating_sub(1)].to_string(),
        base64::engine::general_purpose::STANDARD.encode(secret),
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(secret),
    ]
    .into_iter()
    .filter(|value| !value.is_empty())
    .map(Zeroizing::new)
    .collect()
}

pub(super) fn protection_error() -> DaemonError {
    DaemonError::LocalTransport { operation: "room.secret_observation",
        message: "Agent observation withheld: Vault input may remain in this view or recovered artifacts. Clear the sensitive content and approve the Room observation clearance interaction.".into() }
}

pub(super) fn command_secret(command: &Command) -> Option<&str> {
    match command {
        Command::Action {
            action:
                crate::runtime::browser_controller_action::BrowserLocatorAction::Fill {
                    text,
                    expected_document_url: Some(_),
                    ..
                },
            ..
        }
        | Command::RecoverAction {
            action:
                crate::runtime::browser_controller_action::BrowserLocatorAction::Fill {
                    text,
                    expected_document_url: Some(_),
                    ..
                },
            ..
        } => Some(text),
        Command::ComputerInput {
            action:
                crate::transport::room_browser_controller::RoomComputerInputAction::SecretText {
                    input,
                    ..
                },
            ..
        } => Some(input.as_str()),
        _ => None,
    }
}

impl KernelRuntimeState {
    pub(crate) fn protect_semantic_recall_candidates(
        &self,
        mut candidates: Vec<crate::local::SemanticRecallMatch>,
    ) -> Vec<crate::local::SemanticRecallMatch> {
        for candidate in &mut candidates {
            let protected = self
                .owned
                .room_secret_observations
                .protect_history_events(vec![candidate.event.clone()]);
            if protected[0] != candidate.event {
                candidate.chunk_text = None;
                candidate.reason = None;
            }
            candidate.event = protected[0].clone();
            if let Some(room) = candidate.event.session_id.as_deref() {
                candidate.chunk_text = candidate.chunk_text.as_deref().map(|text| {
                    self.owned
                        .room_secret_observations
                        .scrub_text_or_withhold(room, text)
                });
                candidate.reason = candidate.reason.as_deref().map(|text| {
                    self.owned
                        .room_secret_observations
                        .scrub_text_or_withhold(room, text)
                });
            }
        }
        candidates
    }

    pub(super) async fn ensure_room_observation_clearance(
        &self,
        room: &str,
        agent: &str,
        pixels: bool,
    ) -> Result<(), DaemonError> {
        if !self.owned.room_secret_observations.blocked(room, pixels)? {
            return Ok(());
        }
        let _barrier = self
            .owned
            .room_secret_observations
            .barrier(room)?
            .write_owned()
            .await;
        if !self.owned.room_secret_observations.blocked(room, pixels)? {
            return Ok(());
        }
        let interaction = crate::session::RuntimeInteraction::new(
            format!("room-observation-clearance-{}", crate::session::unix_epoch_ms()), agent,
            crate::session::RuntimeInteractionKind::Permission, crate::session::RuntimeInteractionLevel::Critical,
            Some("Clear sensitive Room observations".into()),
            "Vault input may be visible in this Room, including copied page text, images and restored content. Remove all sensitive content from every tab and desktop window before resuming agent observations. Existing image artifacts remain withheld. Approve only after checking the view.",
            vec![crate::session::RuntimeInteractionChoice::new("clear", "Content cleared; resume observations", "clear", Some(crate::session::RuntimeInteractionChoiceStyle::Primary)),
                crate::session::RuntimeInteractionChoice::new("deny", "Keep observations withheld", "deny", Some(crate::session::RuntimeInteractionChoiceStyle::Danger))],
            None, Some(30), Some("deny".into()));
        let id = interaction.id().to_string();
        let resolution = self.create_runtime_interaction(room, interaction).await?;
        let resolution =
            match tokio::time::timeout(std::time::Duration::from_secs(30), resolution).await {
                Ok(Ok(resolution)) => resolution,
                _ => {
                    self.timeout_runtime_interaction(room, &id).await?;
                    return Err(protection_error());
                }
            };
        if resolution.choice_id.as_deref() != Some("clear") {
            return Err(protection_error());
        }
        if let Some(slice) = self.owned.slice_store.environment_slice(room) {
            let response = self
                .route_room_browser_controller_command(room, slice, Command::ClearSecretObservation)
                .await?;
            if !matches!(response, Response::SecretObservationCleared) {
                return Err(protection_error());
            }
        }
        self.owned.room_secret_observations.clear(room)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestRoot(PathBuf);
    impl TestRoot {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static SEQUENCE: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "chariox-vaultredact-{}-{}-{}",
                std::process::id(),
                crate::session::unix_epoch_ms(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn room_capture_fence_settles_input_without_blocking_another_room() {
        let root = TestRoot::new();
        let store = RoomSecretObservations::new(root.path().to_path_buf(), BTreeSet::new());
        let capture = store.barrier("room").unwrap().read_owned().await;
        let pending = store.barrier("room").unwrap().write_owned();
        tokio::pin!(pending);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut pending)
                .await
                .is_err()
        );
        assert!(store
            .barrier("other-room")
            .unwrap()
            .try_write_owned()
            .is_ok());
        drop(capture);
        let input = pending.await;
        store.register("room", "synthetic-only").unwrap();
        assert!(store.barrier("room").unwrap().try_read_owned().is_err());
        drop(input);
        let _after_input = store.barrier("room").unwrap().read_owned().await;
        assert!(store.require("room", true).is_err());
    }

    #[tokio::test]
    async fn only_human_room_interaction_can_clear_and_known_values_remain_scrubbed() {
        use super::super::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::TestRoom;
        let root = TestRoot::new();
        let mut room = TestRoom::new("vault-observation-clearance");
        room.runtime.owned.room_secret_observations =
            RoomSecretObservations::new(root.path().to_path_buf(), BTreeSet::new());
        room.runtime
            .owned
            .room_secret_observations
            .register(&room.session_id, "synthetic-only")
            .unwrap();
        for choice in ["deny", "clear"] {
            let runtime = room.runtime.clone();
            let session_id = room.session_id.clone();
            let agent_id = room.agent_id.clone();
            let task = tokio::spawn(async move {
                runtime
                    .ensure_room_observation_clearance(&session_id, &agent_id, true)
                    .await
            });
            let id = tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    let session = room
                        .runtime
                        .owned
                        .session_store
                        .get_session(&room.session_id)
                        .unwrap();
                    if let Some(interaction) = session
                        .active_interactions()
                        .iter()
                        .find(|i| i.id().starts_with("room-observation-clearance-"))
                    {
                        assert!(!format!("{interaction:?}").contains("synthetic-only"));
                        break interaction.id().to_string();
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            room.runtime
                .resolve_runtime_interaction(&room.session_id, &id, choice, None)
                .await
                .unwrap();
            let result = task.await.unwrap();
            assert_eq!(result.is_ok(), choice == "clear");
        }
        assert!(room
            .runtime
            .owned
            .room_secret_observations
            .require(&room.session_id, true)
            .is_ok());
        assert_eq!(
            room.runtime
                .owned
                .room_secret_observations
                .scrub_text_or_withhold(&room.session_id, "synthetic-only"),
            "[redacted]"
        );
        room.runtime.owned.append_history_entries(&room.session_id, vec![
            crate::history::SessionHistoryEntry::provider_output(&room.session_id, "test-run",
                Some(&room.agent_id), crate::terminal::TerminalOutputKind::ProviderTool,
                None, serde_json::json!({"id": "tool", "status": "completed", "output": "synthetic-only"}).to_string()),
            crate::history::SessionHistoryEntry::provider_output(&room.session_id, "test-run",
                Some(&room.agent_id), crate::terminal::TerminalOutputKind::ProviderOutput,
                None, "synthetic"),
            crate::history::SessionHistoryEntry::provider_output(&room.session_id, "test-run",
                Some(&room.agent_id), crate::terminal::TerminalOutputKind::ProviderOutput,
                None, "-only"),
        ]);
        let events = room
            .runtime
            .owned
            .operational_history_store
            .load_session_events(&room.session_id, Some(&room.agent_id))
            .unwrap();
        let tools = events
            .iter()
            .filter(|event| event.kind == crate::history::HistoryEventKind::ProviderTool)
            .collect::<Vec<_>>();
        assert_eq!(tools.len(), 1);
        assert!(tools[0].content.as_ref().unwrap().contains("[redacted]"));
        let outputs = events
            .iter()
            .filter(|event| event.kind == crate::history::HistoryEventKind::ProviderOutput)
            .collect::<Vec<_>>();
        assert!(!outputs.is_empty());
        assert!(outputs
            .iter()
            .all(|event| event.content.as_deref()
                == Some("[sensitive Room terminal stream withheld]")));
        assert!(events.iter().all(|event| !event
            .content
            .as_deref()
            .unwrap_or_default()
            .contains("synthetic-only")));
    }

    #[test]
    fn worker_local_transcripts_share_home_room_registry_and_recovery_fence() {
        let root = TestRoot::new();
        let store = RoomSecretObservations::new(root.path().join("observations"), BTreeSet::new())
            .with_worker_room(Some("home-room".into()));
        store.register("home-room", "synthetic-only").unwrap();
        assert_eq!(
            store.scrub_text_or_withhold("worker-run-session", "synthetic-only"),
            "[redacted]"
        );
        assert!(store.require("worker-run-session", true).is_err());
        assert!(Arc::ptr_eq(
            &store.barrier("home-room").unwrap(),
            &store.barrier("worker-run-session").unwrap()
        ));
        store.clear("home-room").unwrap();
        assert!(store.require("worker-run-session", true).is_ok());
        assert_eq!(
            store
                .scrub_cached_result("worker-run-session", "synthetic-only".to_string())
                .unwrap(),
            "[redacted]"
        );
        let restarted =
            RoomSecretObservations::new(root.path().join("observations"), BTreeSet::new())
                .with_worker_room(Some("home-room".into()));
        assert!(restarted.require("worker-run-session", false).is_err());
        restarted.clear("home-room").unwrap();
        assert!(restarted
            .scrub_cached_result("worker-run-session", "old data".to_string())
            .is_err());
    }

    #[test]
    fn split_terminal_chunks_are_withheld_even_after_live_view_clearance() {
        let root = TestRoot::new();
        let store = RoomSecretObservations::new(root.path().join("observations"), BTreeSet::new());
        assert_eq!(store.protect_unframed_bytes("room", b"normal"), b"normal");
        store.register("room", "synthetic-only").unwrap();
        store.clear("room").unwrap();
        for bytes in [b"synthetic".as_slice(), b"-only".as_slice()] {
            let protected = store.protect_unframed_bytes("room", bytes);
            assert_eq!(protected, b"[sensitive Room terminal stream withheld]");
            let entry = crate::history::SessionHistoryEntry::provider_output(
                "room",
                "run",
                None,
                crate::terminal::TerminalOutputKind::ProviderOutput,
                None,
                String::from_utf8_lossy(bytes),
            );
            assert_eq!(
                store.protect_transcript_entry(entry).text,
                "[sensitive Room terminal stream withheld]"
            );
        }
    }

    #[test]
    fn vault_echo_scrubs_nested_tool_text_and_simple_transforms_and_room_isolation() {
        let root = TestRoot::new();
        let store = RoomSecretObservations::new(root.path().join("observations"), BTreeSet::new());
        let secret = "vault-page-copy-regression-value";
        store.register("room", secret).unwrap();
        let variants = secret_variants(secret);
        let input = serde_json::json!({"accessibility": variants.iter().map(|s| s.as_str()).collect::<Vec<_>>(), "dom": {secret: secret}, "ocr": secret});
        let scrubbed = store.scrub("room", input.clone()).unwrap();
        assert!(!scrubbed.to_string().contains(secret));
        assert!(scrubbed["accessibility"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s == "[redacted]"));
        assert_eq!(store.scrub("other-room", input.clone()).unwrap(), input);
        assert!(store.require("room", true).is_err());
        store.clear("room").unwrap();
        assert!(store.require("room", true).is_ok());
        assert!(!store
            .scrub("room", input)
            .unwrap()
            .to_string()
            .contains(secret));
        store.register("room", "another-input").unwrap();
        assert!(store.require("room", true).is_err());
    }

    #[test]
    fn restart_and_restore_withhold_text_pixels_and_preexisting_rooms() {
        let root = TestRoot::new();
        let path = root.path().join("observations");
        let store = RoomSecretObservations::new(path.clone(), BTreeSet::new());
        store.register("room", "synthetic-only").unwrap();
        store.clear("room").unwrap();
        let restarted = RoomSecretObservations::new(path, BTreeSet::from(["old-G-room".into()]));
        for room in ["room", "old-G-room"] {
            assert!(restarted.require(room, false).is_err());
            assert!(restarted.require(room, true).is_err());
            restarted.clear(room).unwrap();
            assert!(restarted.require(room, true).is_ok());
        }
        assert!(restarted.require("fresh", true).is_ok());
    }

    #[test]
    fn clearance_never_reauthorizes_unknown_recovered_history() {
        let root = TestRoot::new();
        let store = RoomSecretObservations::new(
            root.path().join("observations"),
            BTreeSet::from(["old-room".into()]),
        );
        store.clear("old-room").unwrap();
        let entry = crate::history::SessionHistoryEntry::provider_output(
            "old-room",
            "run",
            Some("agent"),
            crate::terminal::TerminalOutputKind::ProviderOutput,
            None,
            "synthetic old observation",
        );
        let mut event = crate::history::HistoryEvent::transcript(1, &entry, Default::default());
        event.timestamp_ms = 0;
        event.content_ref = Some("old-image".into());
        event
            .metadata
            .insert("observation".into(), "synthetic old observation".into());
        let protected = store.protect_history_events(vec![event]);
        assert_eq!(
            protected[0].content.as_deref(),
            Some("[recovered sensitive Room history withheld]")
        );
        assert!(protected[0].metadata.is_empty());
        assert!(protected[0].content_ref.is_none());
    }

    #[test]
    fn marker_io_failure_aborts_input_and_withholds_capture() {
        let root = TestRoot::new();
        let path = root.path().join("file");
        std::fs::write(&path, b"not a directory").unwrap();
        let store = RoomSecretObservations::new(path, BTreeSet::new());
        assert!(store.register("room", "synthetic-only").is_err());
        assert!(store.require("room", true).is_err());
    }
}
