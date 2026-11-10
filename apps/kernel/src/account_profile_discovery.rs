//! Registry signals for machine-level account discovery.
//!
//! The registry is the one place every authentication change passes through, so
//! it also tells the kernel when the account part of a client catalog changed.
//! Locations the kernel already considered are remembered here so that removing
//! a discovered profile is never undone by the next discovery pass.

use super::*;
use std::hash::{Hash as _, Hasher as _};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct SeenMachineLocation {
    provider: String,
    path: PathBuf,
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
        let seen = SeenMachineLocation {
            provider: provider.to_string(),
            path: path.to_path_buf(),
        };
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

    /// Remember a machine location and report whether discovery should consider it.
    pub(crate) fn claim_machine_location(
        &self,
        provider: &str,
        path: &Path,
    ) -> Result<bool, DaemonError> {
        let provider = normalize_provider(provider)?;
        let mut document = self.write_document()?;
        if !document.remember_machine_location(provider, path) {
            return Ok(false);
        }
        self.persist_locked(&document)?;
        Ok(true)
    }
}
