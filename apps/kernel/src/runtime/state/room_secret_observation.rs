//! MP-08/MP-10/MP-11: Room-scoped observation protection, below every client.
//! Values use zeroizing memory and a private runtime-identity-sealed registry.
//! Retired values are scrub-only for the Room lifetime; deletion wipes them.
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

mod lifecycle;
mod revocation;
mod storage;

use super::KernelRuntimeState;
use crate::error::DaemonError;
use crate::transport::room_browser_controller::{
    RoomBrowserControllerCommand as Command, RoomBrowserControllerResult as Response,
    SecretObservationDisposition as Disposition,
};

#[derive(Clone)]
pub(super) struct RoomSecretObservations {
    root: PathBuf,
    identity: Option<Arc<Zeroizing<String>>>,
    worker_room: Option<String>,
    migration_failed: bool,
    rooms: Arc<Mutex<BTreeMap<String, Protection>>>,
    barriers: Arc<Mutex<BTreeMap<String, Arc<tokio::sync::RwLock<()>>>>>,
    vault_lifecycle: Arc<tokio::sync::RwLock<()>>,
    revocation_delivery: Arc<Mutex<BTreeMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>>,
    pub(super) epoch: u64,
}

#[derive(Default)]
struct Protection {
    values: Vec<Zeroizing<String>>,
    retired_values: Vec<Zeroizing<String>>,
    vault_keys: BTreeSet<String>,
    provenance_known: bool,
    unknown: bool,
    targets: Vec<serde_json::Value>,
    revision: u64,
    recovered_artifacts: bool,
    history_before_ms: u64,
}

impl RoomSecretObservations {
    pub(super) fn new(root: PathBuf, recovered: BTreeSet<String>) -> Self {
        let mut store = Self {
            root,
            identity: None,
            worker_room: None,
            migration_failed: false,
            rooms: Default::default(),
            barriers: Default::default(),
            vault_lifecycle: Default::default(),
            revocation_delivery: Default::default(),
            epoch: crate::session::unix_epoch_ms(),
        };
        store.migration_failed = store.migrate_legacy_rooms(recovered).is_err()
            || store.migrate_flat_revocations().is_err();
        store
    }

    // A provisioner-bound slice executes only this home Room, even when its
    // provider runs and transcripts carry worker-local session identifiers.
    pub(super) fn with_worker_room(mut self, room: Option<String>) -> Self {
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
        if self.migration_failed {
            return Err(protection_error());
        }
        // Missing is the only clean disk state. Permission and I/O failures close observations.
        let marked = match std::fs::symlink_metadata(self.marker(room)) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => return Err(protection_error()),
        };
        if marked {
            match self.restore_registry(room) {
                Ok(Some(restored)) => return Ok(restored),
                Ok(None) => {}
                Err(_) => return Ok(self.retire_unreadable_registry(room)),
            }
        } else if let Some(history_before_ms) = self.legacy_history_cutoff(room)? {
            // Migration withholds old artifacts, without blocking fresh observations.
            return Ok(Protection {
                recovered_artifacts: true,
                history_before_ms,
                ..Default::default()
            });
        }
        let unknown = marked;
        Ok(Protection {
            unknown,
            recovered_artifacts: unknown,
            history_before_ms: if unknown { self.epoch } else { 0 },
            ..Default::default()
        })
    }

    pub(super) fn register_credential_source(
        &self,
        room: &str,
        key: Option<&str>,
    ) -> Result<(), DaemonError> {
        let room = self.room_key(room);
        self.blocked(room, false)?;
        let mut rooms = self.rooms.lock().map_err(|_| protection_error())?;
        let protection = rooms.get_mut(room).ok_or_else(protection_error)?;
        if let Some(key) = key.map(str::trim).filter(|key| !key.is_empty()) {
            if protection.vault_keys.len() >= 256 && !protection.vault_keys.contains(key) {
                return Err(protection_error());
            }
            protection.vault_keys.insert(key.to_string());
        }
        if protection.values.is_empty() && !protection.unknown {
            protection.provenance_known = true;
        }
        self.persist_marker(room, protection)
    }

    pub(super) fn uses_vault_key(&self, room: &str, key: &str) -> Result<bool, DaemonError> {
        let key = key.trim();
        if key.is_empty() {
            return Ok(false);
        }
        let room = self.room_key(room);
        self.blocked(room, false)?;
        let rooms = self.rooms.lock().map_err(|_| protection_error())?;
        let protection = &rooms[room];
        // Registries from the unshipped predecessor lack provenance. Revoke them
        // conservatively rather than leaving an untracked value on disk.
        Ok(!protection.values.is_empty()
            && (!protection.provenance_known || protection.vault_keys.contains(key)))
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
        let retired = protection
            .retired_values
            .iter()
            .any(|existing| existing.as_str() == value);
        if value.is_empty()
            || (!known
                && !retired
                && protection.values.len() + protection.retired_values.len() >= 256)
        {
            protection.unknown = true;
            return Err(protection_error());
        }
        if !known {
            protection
                .retired_values
                .retain(|existing| existing.as_str() != value);
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
        if self.blocked(room, pixels)? {
            return Err(fenced_observation_error());
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
            let mut target = match command {
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
            target["value_hash"] =
                serde_json::json!(format!("{:x}", Sha256::digest(secret.as_bytes())));
            target["fill_revision"] = serde_json::json!(protection.revision);
            protection.targets.retain(|previous| {
                if previous["kind"] != target["kind"] {
                    return true;
                }
                if target["kind"] == "browser" {
                    previous["target_id"] != target["target_id"]
                        || (previous["document_id"] == target["document_id"]
                            && previous["node_ref"] != target["node_ref"])
                } else {
                    previous["target"] != target["target"]
                }
            });
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
            "unknown": protection.unknown,
            "targets": protection.targets,
            "values": protection.values.iter().chain(&protection.retired_values).map(|value| value.as_str()).collect::<Vec<_>>(),
        })).map(Zeroizing::new).map_err(|_| protection_error())
    }

    // Called with the capture/input barrier held; the private helper reports
    // only confirmed dead native XIDs, never page text or secret values.
    pub(super) fn prune_native_targets(&self, room: &str, ids: &[u64]) -> Result<(), DaemonError> {
        let room = self.room_key(room);
        let mut rooms = self.rooms.lock().map_err(|_| protection_error())?;
        let protection = rooms.get_mut(room).ok_or_else(protection_error)?;
        let before = protection.targets.len();
        protection.targets.retain(|target| {
            target["kind"] != "native"
                || target["target"]["focus_window"]
                    .as_u64()
                    .is_none_or(|id| !ids.contains(&id))
        });
        if protection.targets.len() != before {
            self.persist_registry(room, protection)?;
        }
        Ok(())
    }

    pub(super) fn controller_values(
        &self,
        room: &str,
    ) -> Result<Vec<Zeroizing<String>>, DaemonError> {
        let room = self.room_key(room);
        self.require(room, false)?;
        let rooms = self.rooms.lock().map_err(|_| protection_error())?;
        let protection = &rooms[room];
        Ok(protection
            .values
            .iter()
            .chain(&protection.retired_values)
            .cloned()
            .collect())
    }

    pub(super) fn revision(&self, room: &str) -> Result<u64, DaemonError> {
        let room = self.room_key(room);
        self.blocked(room, false)?;
        Ok(self.rooms.lock().map_err(|_| protection_error())?[room].revision)
    }

    pub(super) fn scrub_error(&self, room: &str, error: DaemonError) -> DaemonError {
        self.scrub_error_inner(room, error, true)
    }

    pub(super) fn scrub_runtime_error(&self, room: &str, error: DaemonError) -> DaemonError {
        self.scrub_error_inner(room, error, false)
    }

    fn scrub_error_inner(&self, room: &str, error: DaemonError, observation: bool) -> DaemonError {
        if matches!(error, DaemonError::UserDomainRefused { .. }) {
            return error;
        }
        let original = error.to_string();
        match if observation {
            self.scrub(room, original.clone())
        } else {
            self.scrub_runtime_result(room, original.clone())
        } {
            Ok(message) if message == original => error,
            Ok(message) => DaemonError::LocalTransport {
                operation: "room.observation",
                message,
            },
            Err(error) => error,
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
                protection.unknown
                    || !protection.values.is_empty()
                    || !protection.retired_values.is_empty()
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
        self.scrub_values(room, input, true)
    }

    // A missing-value fence restricts observations, not workflow, messaging or
    // credential mutations. Their results still scrub every available known value.
    pub(super) fn scrub_runtime_result<T: Serialize + DeserializeOwned>(
        &self,
        room: &str,
        input: T,
    ) -> Result<T, DaemonError> {
        self.scrub_values(room, input, false)
    }

    fn scrub_values<T: Serialize + DeserializeOwned>(
        &self,
        room: &str,
        input: T,
        observation: bool,
    ) -> Result<T, DaemonError> {
        let room = self.room_key(room);
        self.blocked(room, false)?;
        let rooms = self.rooms.lock().map_err(|_| protection_error())?;
        let protection = &rooms[room];
        // Recheck under the value lock: another mutation can follow admission.
        if observation && protection.unknown {
            return Err(fenced_observation_error());
        }
        if protection.values.is_empty() && protection.retired_values.is_empty() {
            return Ok(input);
        }
        let mut value = serde_json::to_value(input).map_err(|_| protection_error())?;
        scrub_value(&mut value, &protection.values);
        scrub_value(&mut value, &protection.retired_values);
        serde_json::from_value(value).map_err(|_| protection_error())
    }

    pub(super) fn protect_transcript_entry(
        &self,
        mut entry: crate::history::SessionHistoryEntry,
    ) -> crate::history::SessionHistoryEntry {
        use crate::history::SessionHistoryEntryKind;
        if self.withholds_history(&entry.session_id, entry.timestamp_ms) {
            entry.text = "[recovered sensitive Room history withheld]".into();
            entry.attachments.clear();
            return entry;
        }
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
            if self.withholds_history(room, event.timestamp_ms) {
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

    fn withholds_history(&self, room: &str, timestamp_ms: u64) -> bool {
        let room = self.room_key(room);
        if self.blocked(room, false).is_err() {
            return true;
        }
        self.rooms
            .lock()
            .map(|rooms| {
                let protection = &rooms[room];
                protection.recovered_artifacts && timestamp_ms <= protection.history_before_ms
            })
            .unwrap_or(true)
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

fn fenced_observation_error() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "room.secret_observation.fenced",
        message: "Room observation state is fenced: the secret registry is unavailable. Prior echoes remain withheld until a clean environment is reprovisioned; capture retries cannot clear this state.".into(),
    }
}

pub(super) fn protection_error() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "room.secret_observation",
        message: "Room observation protection state is unavailable; observations remain withheld."
            .into(),
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
mod tests;
