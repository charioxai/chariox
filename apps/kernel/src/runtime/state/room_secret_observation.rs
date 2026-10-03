//! MP-08/MP-10/MP-11: Room-scoped observation protection, below every client.
//! Values live only in zeroizing memory. Durable markers contain no values.
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

mod storage;

use super::KernelRuntimeState;
use crate::error::DaemonError;
use crate::transport::room_browser_controller::{
    RoomBrowserControllerCommand as Command, RoomBrowserControllerResult as Response,
};

#[derive(Clone)]
pub(super) struct RoomSecretObservations {
    root: PathBuf,
    identity: Option<Arc<Zeroizing<String>>>,
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
    targets: Vec<serde_json::Value>,
    revision: u64,
    recovered_artifacts: bool,
}

impl RoomSecretObservations {
    pub(super) fn new(root: PathBuf, recovered: BTreeSet<String>) -> Self {
        Self {
            root,
            identity: None,
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
        if marked {
            if let Some(restored) = self.restore_registry(room)? {
                return Ok(restored);
            }
        }
        let unknown = marked || self.recovered.contains(room);
        Ok(Protection {
            unknown,

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
        self.persist_marker(room, protection)?;
        Ok(())
    }

    fn persist_marker(&self, room: &str, protection: &Protection) -> Result<(), DaemonError> {
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
                file.write_all(b"protected observations\n")
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
        self.persist_registry(room, protection)
    }

    pub(super) fn blocked(&self, room: &str, _pixels: bool) -> Result<bool, DaemonError> {
        let room = self.room_key(room);
        let mut rooms = self.rooms.lock().map_err(|_| protection_error())?;
        if !rooms.contains_key(room) {
            rooms.insert(room.into(), self.initial(room)?);
        }
        let protection = &rooms[room];
        Ok(protection.unknown)
    }

    pub(super) fn require(&self, room: &str, pixels: bool) -> Result<(), DaemonError> {
        if self.blocked(room, pixels)? && !pixels {
            return Err(protection_error());
        }
        Ok(())
    }

    pub(super) fn with_identity(mut self, key: &str) -> Self {
        self.identity = Some(Arc::new(Zeroizing::new(key.to_string())));
        self
    }

    // Called under the exclusive input barrier, before any physical insertion.
    pub(super) fn register_command(
        &self,
        room: &str,
        command: &Command,
    ) -> Result<(), DaemonError> {
        if let Some(secret) = command_secret(command) {
            self.register(room, secret)?;
            let target = match command {
                Command::Action {
                    target_id,
                    document_id,
                    node_ref,
                    ..
                }
                | Command::RecoverAction {
                    target_id,
                    document_id,
                    node_ref,
                    ..
                } => {
                    serde_json::json!({"kind":"browser", "target_id":target_id, "document_id":document_id, "node_ref":node_ref})
                }
                Command::ComputerInput {
                    action:
                        crate::transport::room_browser_controller::RoomComputerInputAction::SecretText {
                            expected_target,
                            ..
                        },
                    ..
                } => serde_json::json!({"kind":"native", "target":expected_target}),
                _ => return Err(protection_error()),
            };
            let room = self.room_key(room);
            let mut rooms = self.rooms.lock().map_err(|_| protection_error())?;
            let protection = rooms.get_mut(room).ok_or_else(protection_error)?;
            if !protection.targets.contains(&target) {
                if protection.targets.len() >= 256 {
                    return Err(protection_error());
                }
                protection.targets.push(target);
            }
            self.persist_registry(room, protection)?;
        }
        Ok(())
    }

    pub(super) fn capture_policy(&self, room: &str) -> Result<Zeroizing<String>, DaemonError> {
        let room = self.room_key(room);
        self.blocked(room, false)?;
        let rooms = self.rooms.lock().map_err(|_| protection_error())?;
        let protection = &rooms[room];
        serde_json::to_string(&serde_json::json!({
            "unknown": protection.unknown || (!protection.values.is_empty() && protection.targets.is_empty()),
            "targets": protection.targets,
            "values": protection.values.iter().map(|value| value.as_str()).collect::<Vec<_>>(),
        })).map(Zeroizing::new).map_err(|_| protection_error())
    }

    pub(super) fn controller_values(
        &self,
        room: &str,
    ) -> Result<Vec<Zeroizing<String>>, DaemonError> {
        let room = self.room_key(room);
        self.require(room, false)?;
        Ok(self.rooms.lock().map_err(|_| protection_error())?[room]
            .values
            .clone())
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
        // Recheck under the value lock: another insertion may follow the first require.
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
            // Retain the fence for the session lifetime, without a human clearance.
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
                // must not enter the durable Room registry without a known registry.
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
    DaemonError::LocalTransport {
        operation: "room.secret_observation",
        message: "observation redacted, retrying".into(),
    }
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

    // MP-08/MP-10/MP-11: compatibility preflight has no interaction or wait.
    pub(super) async fn ensure_room_observation_ready(
        &self,
        room: &str,
        _agent: &str,
        pixels: bool,
    ) -> Result<(), DaemonError> {
        self.owned.room_secret_observations.require(room, pixels)
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
        assert!(store.require("room", true).is_ok());
    }

    #[tokio::test]
    async fn observations_never_create_human_clearance_even_for_false_positives() {
        use super::super::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::TestRoom;
        let root = TestRoot::new();
        let mut room = TestRoom::new("vault-autonomous-observation");
        room.runtime.owned.room_secret_observations =
            RoomSecretObservations::new(root.path().to_path_buf(), BTreeSet::new());
        room.runtime
            .owned
            .room_secret_observations
            .register(&room.session_id, "synthetic-only")
            .unwrap();
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            room.runtime
                .ensure_room_observation_ready(&room.session_id, &room.agent_id, true),
        )
        .await;
        assert!(
            result.is_ok(),
            "MP-08/MP-10/MP-11: observation must never wait for a human"
        );
        assert!(room
            .runtime
            .owned
            .session_store
            .get_session(&room.session_id)
            .unwrap()
            .active_interactions()
            .is_empty());
        assert_eq!(
            room.runtime
                .owned
                .room_secret_observations
                .scrub_text_or_withhold(&room.session_id, "benign field: synthetic-onl"),
            "benign field: synthetic-onl"
        );
        assert_eq!(
            room.runtime
                .owned
                .room_secret_observations
                .scrub_text_or_withhold(&room.session_id, "synthetic-only"),
            "[redacted]"
        );
    }

    #[test]
    fn sealed_registry_recovers_scrubbing_and_masks_without_human_clearance() {
        let root = TestRoot::new();
        let path = root.path().join("observations");
        let key = crate::transport::relay_crypto::generate_private_key_base64();
        let store = RoomSecretObservations::new(path.clone(), BTreeSet::new()).with_identity(&key);
        let command = Command::Action {
            execution_id: "input".into(),
            target_id: "target".into(),
            document_id: "document".into(),
            node_ref: "backend:42".into(),
            action: crate::runtime::browser_controller_action::BrowserLocatorAction::Fill {
                text: "synthetic-only".into(),
                append: false,
                submit: false,
                expected_document_url: Some("https://fixture.test".into()),
            },
            timeout_ms: 1000,
        };
        store.register_command("room", &command).unwrap();
        let bytes = std::fs::read(store.registry_path("room")).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("synthetic-only"));
        let recovered =
            RoomSecretObservations::new(path, BTreeSet::from(["room".into()])).with_identity(&key);
        assert_eq!(
            recovered.scrub_text_or_withhold("room", "synthetic-only and benign"),
            "[redacted] and benign"
        );
        let policy: serde_json::Value =
            serde_json::from_str(&recovered.capture_policy("room").unwrap()).unwrap();
        assert_eq!(policy["unknown"], false);
        assert_eq!(policy["targets"][0]["node_ref"], "backend:42");
        assert!(recovered
            .scrub_cached_result("room", "old observation".to_string())
            .is_err());
        assert!(recovered.require("room", true).is_ok());
    }

    #[test]
    fn registry_cannot_be_replayed_in_another_room_or_runtime_identity() {
        let root = TestRoot::new();
        let key = crate::transport::relay_crypto::generate_private_key_base64();
        let store = RoomSecretObservations::new(root.path().to_path_buf(), BTreeSet::new())
            .with_identity(&key);
        store.register("room", "synthetic-only").unwrap();
        std::fs::copy(store.marker("room"), store.marker("other")).unwrap();
        std::fs::copy(store.registry_path("room"), store.registry_path("other")).unwrap();
        assert!(store.require("other", false).is_err());
        let other_key = crate::transport::relay_crypto::generate_private_key_base64();
        let other = RoomSecretObservations::new(root.path().to_path_buf(), BTreeSet::new())
            .with_identity(&other_key);
        assert!(other.require("room", false).is_err());
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
        assert!(store.require("worker-run-session", true).is_ok());
        assert!(Arc::ptr_eq(
            &store.barrier("home-room").unwrap(),
            &store.barrier("worker-run-session").unwrap()
        ));

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

        assert!(restarted
            .scrub_cached_result("worker-run-session", "old data".to_string())
            .is_err());
    }

    #[test]
    fn split_terminal_chunks_are_withheld_without_human_clearance() {
        let root = TestRoot::new();
        let store = RoomSecretObservations::new(root.path().join("observations"), BTreeSet::new());
        assert_eq!(store.protect_unframed_bytes("room", b"normal"), b"normal");
        store.register("room", "synthetic-only").unwrap();

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
        assert!(store.require("room", true).is_ok());

        assert!(store.require("room", true).is_ok());
        assert!(!store
            .scrub("room", input)
            .unwrap()
            .to_string()
            .contains(secret));
        store.register("room", "another-input").unwrap();
        assert!(store.require("room", true).is_ok());
    }

    #[test]
    fn legacy_unknown_rooms_drop_observations_without_human_clearance() {
        let root = TestRoot::new();
        let path = root.path().join("observations");
        let store = RoomSecretObservations::new(path.clone(), BTreeSet::new());
        store.register("room", "synthetic-only").unwrap();

        let restarted = RoomSecretObservations::new(path, BTreeSet::from(["old-G-room".into()]));
        for room in ["room", "old-G-room"] {
            assert!(restarted.require(room, false).is_err());
            assert!(restarted.require(room, true).is_ok());

            assert!(restarted.require(room, true).is_ok());
        }
        assert!(restarted.require("fresh", true).is_ok());
    }

    #[test]
    fn fresh_capture_never_reauthorizes_unknown_recovered_history() {
        let root = TestRoot::new();
        let store = RoomSecretObservations::new(
            root.path().join("observations"),
            BTreeSet::from(["old-room".into()]),
        );

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
