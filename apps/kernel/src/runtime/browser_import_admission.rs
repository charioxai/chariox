//! Kernel-local import consent. Callers must derive identities from authenticated
//! transports, never from an import payload. No cookie values belong in this store.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const LIFETIME: Duration = Duration::from_secs(120);
const CAPACITY: usize = 128;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ImportBinding {
    pub user_id: String,
    pub source_attachment_id: String,
    pub source_identity: String,
    pub room_id: String,
    pub environment_id: String,
    pub runtime_generation: u64,
    pub tab_id: String,
    pub document_revision: u64,
    pub source_store_id: String,
    pub domains: Vec<String>,
    pub partition_sites: Vec<String>,
    pub overwrite: bool,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ImportRequestId(String);

impl ImportRequestId {
    pub(crate) fn from_wire(value: &str) -> Result<Self, ImportAdmissionError> {
        if value.len() != 32 || !value.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(ImportAdmissionError::Denied);
        }
        Ok(Self(value.to_string()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ImportRequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[browser import request]")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImportAdmissionError {
    InvalidBinding,
    Denied,
    Busy,
    Full,
}

#[derive(PartialEq, Eq)]
enum Phase {
    Pending,
    Approved,
    Reading,
    Applying,
    Cancelled,
}

struct Entry {
    binding: ImportBinding,
    expires: Instant,
    phase: Phase,
}

#[derive(Clone, Default)]
pub(crate) struct BrowserImportAdmission {
    entries: Arc<Mutex<BTreeMap<ImportRequestId, Entry>>>,
}

impl BrowserImportAdmission {
    pub(crate) fn prepare(
        &self,
        binding: ImportBinding,
        now: Instant,
    ) -> Result<ImportRequestId, ImportAdmissionError> {
        validate_binding(&binding)?;
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| ImportAdmissionError::Denied)?;
        // An expired active operation still owns its slot until recovery finishes.
        entries.retain(|_, entry| {
            now < entry.expires || matches!(entry.phase, Phase::Applying | Phase::Cancelled)
        });
        if entries
            .values()
            .any(|entry| entry.binding.environment_id == binding.environment_id)
        {
            return Err(ImportAdmissionError::Busy);
        }
        if entries.len() >= CAPACITY {
            return Err(ImportAdmissionError::Full);
        }
        let id = loop {
            let candidate = ImportRequestId(format!("{:032x}", rand::random::<u128>()));
            if !entries.contains_key(&candidate) {
                break candidate;
            }
        };
        entries.insert(
            id.clone(),
            Entry {
                binding,
                expires: now + LIFETIME,
                phase: Phase::Pending,
            },
        );
        Ok(id)
    }

    /// Only the initiating authenticated human can approve the immutable binding.
    pub(crate) fn approve(
        &self,
        id: &ImportRequestId,
        current: &ImportBinding,
        now: Instant,
    ) -> Result<(), ImportAdmissionError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| ImportAdmissionError::Denied)?;
        let entry = entries.get_mut(id).ok_or(ImportAdmissionError::Denied)?;
        check(entry, current, now)?;
        if entry.phase != Phase::Pending {
            return Err(ImportAdmissionError::Denied);
        }
        entry.phase = Phase::Approved;
        Ok(())
    }

    /// Claim once before any source cookie read. This phase owns no destination writer.
    pub(crate) fn claim_source(
        &self,
        id: &ImportRequestId,
        current: &ImportBinding,
        now: Instant,
    ) -> Result<(), ImportAdmissionError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| ImportAdmissionError::Denied)?;
        let entry = entries.get_mut(id).ok_or(ImportAdmissionError::Denied)?;
        check(entry, current, now)?;
        if entry.phase != Phase::Approved {
            return Err(ImportAdmissionError::Denied);
        }
        entry.phase = Phase::Reading;
        Ok(())
    }

    pub(crate) fn authorize_source(
        &self,
        id: &ImportRequestId,
        current: &ImportBinding,
        now: Instant,
    ) -> Result<(), ImportAdmissionError> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| ImportAdmissionError::Denied)?;
        let entry = entries.get(id).ok_or(ImportAdmissionError::Denied)?;
        check(entry, current, now)?;
        if entry.phase != Phase::Reading {
            return Err(ImportAdmissionError::Denied);
        }
        Ok(())
    }

    /// Trusted destination execution only, after source claim and within the Environment operation.
    pub(crate) fn claim(
        &self,
        id: &ImportRequestId,
        current: &ImportBinding,
        now: Instant,
    ) -> Result<(), ImportAdmissionError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| ImportAdmissionError::Denied)?;
        let entry = entries.get_mut(id).ok_or(ImportAdmissionError::Denied)?;
        check(entry, current, now)?;
        if entry.phase != Phase::Reading {
            return Err(ImportAdmissionError::Denied);
        }
        entry.phase = Phase::Applying;
        Ok(())
    }

    pub(crate) fn authorize_active(
        &self,
        id: &ImportRequestId,
        current: &ImportBinding,
        now: Instant,
    ) -> Result<(), ImportAdmissionError> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| ImportAdmissionError::Denied)?;
        let entry = entries.get(id).ok_or(ImportAdmissionError::Denied)?;
        check(entry, current, now)?;
        if entry.phase != Phase::Applying {
            return Err(ImportAdmissionError::Denied);
        }
        Ok(())
    }

    pub(crate) fn cancel(
        &self,
        id: &ImportRequestId,
        user_id: &str,
        room_id: &str,
    ) -> Result<bool, ImportAdmissionError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| ImportAdmissionError::Denied)?;
        let entry = entries.get_mut(id).ok_or(ImportAdmissionError::Denied)?;
        if entry.binding.user_id != user_id || entry.binding.room_id != room_id {
            return Err(ImportAdmissionError::Denied);
        }
        let active = matches!(entry.phase, Phase::Applying | Phase::Cancelled);
        if active {
            entry.phase = Phase::Cancelled;
        } else {
            entries.remove(id);
        }
        Ok(active)
    }

    /// Normal trusted execution completion only, after successful verification.
    /// Cancellation and missing volatile state require the durable recovery path.
    pub(crate) fn finish(&self, id: &ImportRequestId) -> Result<(), ImportAdmissionError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| ImportAdmissionError::Denied)?;
        let entry = entries.get(id).ok_or(ImportAdmissionError::Denied)?;
        if entry.phase != Phase::Applying {
            return Err(ImportAdmissionError::Denied);
        }
        entries.remove(id);
        Ok(())
    }

    /// Durable recovery is the only authority allowed to retire a cancelled
    /// admission, and remains valid after volatile admission state is lost.
    pub(crate) fn finish_recovery(
        &self,
        id: &ImportRequestId,
    ) -> Result<(), ImportAdmissionError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| ImportAdmissionError::Denied)?;
        let Some(entry) = entries.get(id) else {
            return Ok(());
        };
        if !matches!(entry.phase, Phase::Applying | Phase::Cancelled) {
            return Err(ImportAdmissionError::Denied);
        }
        entries.remove(id);
        Ok(())
    }
}

fn check(entry: &Entry, current: &ImportBinding, now: Instant) -> Result<(), ImportAdmissionError> {
    if entry.binding != *current || now >= entry.expires {
        return Err(ImportAdmissionError::Denied);
    }
    Ok(())
}

fn validate_binding(binding: &ImportBinding) -> Result<(), ImportAdmissionError> {
    let valid_id = |value: &str| {
        !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
    };
    let bounded_selection = |values: &[String]| {
        values.len() <= 32
            && values.iter().all(|value| valid_id(value))
            && values
                .iter()
                .enumerate()
                .all(|(index, value)| !values[..index].contains(value))
    };
    if ![
        &binding.user_id,
        &binding.source_attachment_id,
        &binding.source_identity,
        &binding.room_id,
        &binding.environment_id,
        &binding.tab_id,
        &binding.source_store_id,
    ]
    .iter()
    .all(|value| valid_id(value))
        || binding.runtime_generation == 0
        || binding.domains.is_empty()
        || !bounded_selection(&binding.domains)
        || !bounded_selection(&binding.partition_sites)
    {
        return Err(ImportAdmissionError::InvalidBinding);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding() -> ImportBinding {
        ImportBinding {
            source_identity: "source-client-key-binding".into(),
            user_id: "user-1".into(),
            source_attachment_id: "connector-1".into(),
            room_id: "room-1".into(),
            environment_id: "environment-1".into(),
            runtime_generation: 1,
            tab_id: "tab-1".into(),
            document_revision: 1,
            source_store_id: "0".into(),
            domains: vec!["example.test".into()],
            partition_sites: vec![],
            overwrite: false,
        }
    }

    #[test]
    fn import_requires_approval_and_can_only_be_claimed_once() {
        let store = BrowserImportAdmission::default();
        let now = Instant::now();
        let scope = binding();
        let id = store.prepare(scope.clone(), now).unwrap();
        assert_eq!(
            store.claim(&id, &scope, now),
            Err(ImportAdmissionError::Denied)
        );
        store.approve(&id, &scope, now).unwrap();
        assert_eq!(
            store.claim(&id, &scope, now),
            Err(ImportAdmissionError::Denied)
        );
        store.claim_source(&id, &scope, now).unwrap();
        store.claim(&id, &scope, now).unwrap();
        assert_eq!(
            store.authorize_source(&id, &scope, now),
            Err(ImportAdmissionError::Denied)
        );
        assert_eq!(
            store.claim(&id, &scope, now),
            Err(ImportAdmissionError::Denied)
        );
        store.authorize_active(&id, &scope, now).unwrap();
        store.finish(&id).unwrap();
        assert_eq!(
            store.claim(&id, &scope, now),
            Err(ImportAdmissionError::Denied)
        );
    }

    #[test]
    fn approval_does_not_cover_changed_identity_scope_or_overwrite() {
        let now = Instant::now();
        let original = binding();
        let changes: Vec<Box<dyn Fn(&mut ImportBinding)>> = vec![
            Box::new(|b| b.user_id = "other-user".into()),
            Box::new(|b| b.source_attachment_id = "other-connector".into()),
            Box::new(|b| b.room_id = "other-room".into()),
            Box::new(|b| b.environment_id = "other-environment".into()),
            Box::new(|b| b.runtime_generation += 1),
            Box::new(|b| b.tab_id = "other-tab".into()),
            Box::new(|b| b.document_revision += 1),
            Box::new(|b| b.source_store_id = "other-store".into()),
            Box::new(|b| b.domains.push("other.test".into())),
            Box::new(|b| b.partition_sites.push("https://other.test".into())),
            Box::new(|b| b.overwrite = true),
        ];
        for change in changes {
            let store = BrowserImportAdmission::default();
            let id = store.prepare(original.clone(), now).unwrap();
            let mut altered = original.clone();
            change(&mut altered);
            assert_eq!(
                store.approve(&id, &altered, now),
                Err(ImportAdmissionError::Denied)
            );
            store.approve(&id, &original, now).unwrap();
            store.claim_source(&id, &original, now).unwrap();
            assert_eq!(
                store.claim(&id, &altered, now),
                Err(ImportAdmissionError::Denied)
            );
            store.claim(&id, &original, now).unwrap();
            assert_eq!(
                store.authorize_active(&id, &altered, now),
                Err(ImportAdmissionError::Denied)
            );
        }
    }

    #[test]
    fn expiry_revokes_authority_without_releasing_an_in_flight_writer() {
        let store = BrowserImportAdmission::default();
        let now = Instant::now();
        let scope = binding();
        let id = store.prepare(scope.clone(), now).unwrap();
        assert_eq!(
            store.approve(&id, &scope, now + LIFETIME),
            Err(ImportAdmissionError::Denied)
        );
        let id = store.prepare(scope.clone(), now + LIFETIME).unwrap();
        store.approve(&id, &scope, now + LIFETIME).unwrap();
        store.claim_source(&id, &scope, now + LIFETIME).unwrap();
        store.claim(&id, &scope, now + LIFETIME).unwrap();
        let expired = now + LIFETIME * 2;
        assert_eq!(
            store.authorize_active(&id, &scope, expired),
            Err(ImportAdmissionError::Denied)
        );
        assert_eq!(
            store.prepare(scope.clone(), expired),
            Err(ImportAdmissionError::Busy)
        );
        store.finish(&id).unwrap();
        store.prepare(scope, expired).unwrap();
    }

    #[test]
    fn only_owner_can_cancel_and_active_cancellation_waits_for_recovery() {
        let store = BrowserImportAdmission::default();
        let now = Instant::now();
        let scope = binding();
        let id = store.prepare(scope.clone(), now).unwrap();
        assert_eq!(
            store.cancel(&id, "other-user", &scope.room_id),
            Err(ImportAdmissionError::Denied)
        );
        store.approve(&id, &scope, now).unwrap();
        store.claim_source(&id, &scope, now).unwrap();
        store.claim(&id, &scope, now).unwrap();
        store.cancel(&id, &scope.user_id, &scope.room_id).unwrap();
        assert_eq!(
            store.authorize_active(&id, &scope, now),
            Err(ImportAdmissionError::Denied)
        );
        assert_eq!(
            store.prepare(scope.clone(), now),
            Err(ImportAdmissionError::Busy)
        );
        assert_eq!(store.finish(&id), Err(ImportAdmissionError::Denied));
        store.finish_recovery(&id).unwrap();
        store.prepare(scope, now).unwrap();
    }

    #[test]
    fn pending_cancellation_and_process_restart_reject_old_requests() {
        let store = BrowserImportAdmission::default();
        let now = Instant::now();
        let scope = binding();
        let id = store.prepare(scope.clone(), now).unwrap();
        store.cancel(&id, &scope.user_id, &scope.room_id).unwrap();
        assert_eq!(
            store.approve(&id, &scope, now),
            Err(ImportAdmissionError::Denied)
        );
        let next = store.prepare(scope.clone(), now).unwrap();
        assert_ne!(id, next);
        let restarted = BrowserImportAdmission::default();
        assert_eq!(
            restarted.approve(&next, &scope, now),
            Err(ImportAdmissionError::Denied)
        );
    }

    #[test]
    fn cloned_handles_share_consent_cancellation_and_environment_reservation() {
        let store = BrowserImportAdmission::default();
        let other = store.clone();
        let now = Instant::now();
        let scope = binding();
        let id = store.prepare(scope.clone(), now).unwrap();
        other.approve(&id, &scope, now).unwrap();
        store.claim_source(&id, &scope, now).unwrap();
        store.claim(&id, &scope, now).unwrap();
        assert_eq!(
            other.claim(&id, &scope, now),
            Err(ImportAdmissionError::Denied)
        );
        other.cancel(&id, &scope.user_id, &scope.room_id).unwrap();
        assert_eq!(
            store.authorize_active(&id, &scope, now),
            Err(ImportAdmissionError::Denied)
        );
        assert_eq!(
            other.prepare(scope.clone(), now),
            Err(ImportAdmissionError::Busy)
        );
        assert_eq!(store.finish(&id), Err(ImportAdmissionError::Denied));
        store.finish_recovery(&id).unwrap();
        other.prepare(scope, now).unwrap();
    }

    #[test]
    fn racing_claims_have_exactly_one_winner() {
        let store = BrowserImportAdmission::default();
        let now = Instant::now();
        let scope = binding();
        let id = store.prepare(scope.clone(), now).unwrap();
        store.approve(&id, &scope, now).unwrap();
        store.claim_source(&id, &scope, now).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (store, barrier, id, scope) =
                    (store.clone(), barrier.clone(), id.clone(), scope.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    store.claim(&id, &scope, now).is_ok()
                })
            })
            .collect();
        assert_eq!(
            threads
                .into_iter()
                .map(|thread| usize::from(thread.join().unwrap()))
                .sum::<usize>(),
            1
        );
    }

    #[test]
    fn capacity_is_bounded_and_expired_unclaimed_requests_are_reclaimed() {
        let store = BrowserImportAdmission::default();
        let now = Instant::now();
        for index in 0..CAPACITY {
            let mut scope = binding();
            scope.environment_id = format!("environment-{index}");
            let id = store.prepare(scope.clone(), now).unwrap();
            store.approve(&id, &scope, now).unwrap();
            store.claim_source(&id, &scope, now).unwrap();
            assert_eq!(
                store.authorize_source(&id, &scope, now + LIFETIME),
                Err(ImportAdmissionError::Denied)
            );
        }
        let mut scope = binding();
        scope.environment_id = "overflow".into();
        assert_eq!(
            store.prepare(scope.clone(), now),
            Err(ImportAdmissionError::Full)
        );
        store.prepare(scope, now + LIFETIME).unwrap();
    }

    #[test]
    fn invalid_or_unbounded_metadata_is_rejected_and_ids_are_redacted() {
        let store = BrowserImportAdmission::default();
        let now = Instant::now();
        let mut scope = binding();
        scope.domains = vec!["example.test".into(); 33];
        assert_eq!(
            store.prepare(scope, now),
            Err(ImportAdmissionError::InvalidBinding)
        );
        let mut scope = binding();
        scope.source_attachment_id = "x".repeat(257);
        assert_eq!(
            store.prepare(scope, now),
            Err(ImportAdmissionError::InvalidBinding)
        );
        let id = store.prepare(binding(), now).unwrap();
        assert_eq!(format!("{id:?}"), "[browser import request]");
    }
}
