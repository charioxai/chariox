//! Registry signals for machine-level account discovery.
//!
//! The registry is the one place every authentication change passes through, so
//! it also tells the kernel when the account part of a client catalog changed.
//! A location a profile was created from is remembered here so that removing a
//! discovered profile is never undone by the next discovery pass. A location
//! without a usable login is not remembered: it is probed again, with backoff.

use super::*;
use std::collections::HashMap;
use std::hash::{Hash as _, Hasher as _};

/// The first retry of a location without a usable login; each failure doubles it.
const PROBE_BACKOFF_BASE_MS: u64 = 60 * 1000;
const PROBE_BACKOFF_MAX_MS: u64 = 30 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) struct SeenMachineLocation {
    provider: String,
    path: PathBuf,
}

impl SeenMachineLocation {
    /// Links remember the canonical path, so a symlinked candidate must match it.
    fn new(provider: &str, path: &Path) -> Self {
        Self {
            provider: provider.to_string(),
            path: std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()),
        }
    }
}

struct FailedProbes {
    count: u32,
    retry_at_ms: u64,
}

/// Restarting the kernel probes every location once more.
#[derive(Default)]
pub(super) struct ProbeBackoff {
    failed: Mutex<HashMap<SeenMachineLocation, FailedProbes>>,
}

impl ProbeBackoff {
    fn failed(&self) -> std::sync::MutexGuard<'_, HashMap<SeenMachineLocation, FailedProbes>> {
        self.failed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn due(&self, location: &SeenMachineLocation, now_ms: u64) -> bool {
        self.failed()
            .get(location)
            .is_none_or(|failed| now_ms >= failed.retry_at_ms)
    }

    fn record_failure(&self, location: SeenMachineLocation, now_ms: u64) {
        let mut failed = self.failed();
        let entry = failed.entry(location).or_insert(FailedProbes {
            count: 0,
            retry_at_ms: 0,
        });
        entry.count = entry.count.saturating_add(1);
        let delay = PROBE_BACKOFF_BASE_MS
            .saturating_mul(1 << (entry.count - 1).min(16))
            .min(PROBE_BACKOFF_MAX_MS);
        entry.retry_at_ms = now_ms.saturating_add(delay);
    }
}

/// Counts changes to what clients show about who is signed in, not usage ticks.
pub(super) struct AuthChanges {
    revision: AtomicU64,
    digest: Mutex<u64>,
    changed: tokio::sync::Notify,
    discovery_requested: tokio::sync::Notify,
}

impl AuthChanges {
    pub(super) fn new(document: &RegistryDocument) -> Self {
        Self {
            revision: AtomicU64::new(0),
            digest: Mutex::new(auth_digest(document)),
            changed: tokio::sync::Notify::new(),
            discovery_requested: tokio::sync::Notify::new(),
        }
    }

    pub(super) fn observe(&self, document: &RegistryDocument) {
        let digest = auth_digest(document);
        let mut previous = self
            .digest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *previous == digest {
            return;
        }
        *previous = digest;
        self.revision.fetch_add(1, Ordering::AcqRel);
        self.changed.notify_waiters();
    }
}

fn auth_digest(document: &RegistryDocument) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for stored in &document.profiles {
        let profile = &stored.public;
        profile.owner_user_id.hash(&mut hasher);
        profile.provider.hash(&mut hasher);
        profile.profile_id.hash(&mut hasher);
        profile.is_default.hash(&mut hasher);
        format!("{:?}", profile.auth_state).hash(&mut hasher);
        profile.identity_summary.hash(&mut hasher);
    }
    hasher.finish()
}

impl RegistryDocument {
    /// True when the location was not remembered before.
    pub(super) fn remember_machine_location(&mut self, provider: &str, path: &Path) -> bool {
        let seen = SeenMachineLocation::new(provider, path);
        if self.seen_machine_locations.contains(&seen) {
            return false;
        }
        self.seen_machine_locations.push(seen);
        true
    }
}

impl ProviderAccountProfileRegistry {
    pub(crate) fn auth_revision(&self) -> u64 {
        self.auth_changes.revision.load(Ordering::Acquire)
    }

    pub(crate) async fn wait_for_auth_change_after(&self, revision: u64) {
        loop {
            let notified = self.auth_changes.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.auth_revision() != revision {
                return;
            }
            notified.await;
        }
    }

    /// Ask the kernel to look for provider logins again; the kernel decides when.
    pub(crate) fn request_discovery(&self) {
        self.auth_changes.discovery_requested.notify_one();
    }

    pub(crate) async fn wait_for_discovery_request(&self) {
        self.auth_changes.discovery_requested.notified().await;
    }

    /// Whether discovery should probe a machine location now. A location a profile
    /// was created from is never probed again, so removing that profile stands; one
    /// whose last probe found no usable login waits for its backoff.
    pub(crate) fn machine_location_due(
        &self,
        provider: &str,
        path: &Path,
        now_ms: u64,
    ) -> Result<bool, DaemonError> {
        let location = SeenMachineLocation::new(normalize_provider(provider)?, path);
        if self
            .read_document()?
            .seen_machine_locations
            .contains(&location)
        {
            return Ok(false);
        }
        Ok(self.probe_backoff.due(&location, now_ms))
    }

    /// Undo a link discovery made from `root` whose login turned out unusable.
    /// Unlike `remove_registration`, which is the user's decision, this forgets the
    /// location, so a login that appears there later is found.
    pub(crate) fn withdraw_discovered_link(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
        root: &Path,
        now_ms: u64,
    ) -> Result<(), DaemonError> {
        let provider = normalize_provider(provider)?;
        let mut document = self.write_document()?;
        let index = resolved_profile_index(&document, owner_user_id, provider, profile_id)?;
        take_registration(&mut document, owner_user_id, provider, index);
        let location = SeenMachineLocation::new(provider, root);
        document
            .seen_machine_locations
            .retain(|seen| *seen != location);
        self.probe_backoff.record_failure(location, now_ms);
        self.persist_locked(&document)
    }
}
