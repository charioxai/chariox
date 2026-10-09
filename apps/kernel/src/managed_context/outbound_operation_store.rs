//! MP-08/MP-11: restart-safe status and owner/target operation bindings.
use super::*;

#[path = "outbound_operation_retention.rs"]
mod retention;

impl ManagedContextOutboundOperationStore {
    pub(crate) fn get(&self, context_id: &str) -> Option<ManagedContextOutboundOperationStatus> {
        let memory = self
            .state
            .lock()
            .expect("managed-context outbound operation lock")
            .get(context_id)
            .cloned();
        memory.or_else(|| {
            if !valid_artifact_name(context_id) {
                return None;
            }
            let path = self.status_parent()?.join(format!("{context_id}.json"));
            let bytes = read_bounded_regular_file(&path, 256 * 1024).ok()?;
            let status: ManagedContextOutboundOperationStatus =
                serde_json::from_slice(&bytes).ok()?;
            (status.context_id == context_id).then_some(status)
        })
    }

    fn status_parent(&self) -> Option<PathBuf> {
        self.artifact_parent
            .as_deref()
            .map(|root| root.join(".operations"))
    }

    pub(super) fn persist_status(
        &self,
        status: &ManagedContextOutboundOperationStatus,
    ) -> Result<(), DaemonError> {
        let Some(parent) = self.status_parent() else {
            return Ok(());
        };
        create_private_directory(&parent)?;
        check_operation_capacity(&parent, &format!("{}.json", status.context_id))?;
        let bytes = serde_json::to_vec(status)
            .map_err(|_| outbound_service_error("serialize transfer status", true))?;
        crate::config::write_private_file(
            &parent.join(format!("{}.json", status.context_id)),
            &bytes,
        )
        .map_err(|error| outbound_service_io_error("persist transfer status", error))
    }

    pub(crate) fn prepare_owner_ticket(
        &self,
        config: &DaemonConfig,
        runtime: &crate::runtime::state::KernelRuntimeState,
        selection: crate::managed_context::owner_managed::OwnerManagedTransfer,
    ) -> Result<ManagedContextTransferTicket, DaemonError> {
        let candidate =
            crate::managed_context::owner_managed::prepare_ticket(config, runtime, selection)?;
        self.reclaim_operation_metadata(Some(candidate.context_plan.context_id()))?;
        let Some(parent) = self.status_parent() else {
            return Ok(candidate);
        };
        let _guard = self
            .artifact_lock
            .lock()
            .expect("owner operation binding lock");
        create_private_directory(&parent)?;
        let profile = config
            .cloud_relay
            .as_ref()
            .expect("validated source profile");
        let plan = candidate.context_plan.package_binding();
        let index = parent.join(format!(
            "owner-{}.json",
            plan.plan_digest.trim_start_matches("sha256:")
        ));
        if path_entry_exists(&index)? {
            let bytes = read_bounded_regular_file(&index, MAX_OUTBOUND_ARTIFACT_STATE_BYTES)?;
            let saved: PersistedOwnerTicket = serde_json::from_slice(&bytes)
                .map_err(|_| outbound_service_error("invalid owner operation binding", false))?;
            if saved.account_id != profile.account_id || saved.user_id != profile.user_id {
                return Err(crate::managed_context::owner_managed::admission_error(
                    "source context belongs to another account or user",
                ));
            }
            validate_ticket(config, &saved.ticket)?;
            let saved_plan = saved.ticket.context_plan.package_binding();
            if saved_plan.plan_digest != plan.plan_digest || saved.ticket.target != candidate.target
            {
                return Err(outbound_service_error(
                    "owner operation destination changed",
                    false,
                ));
            }
            // Reuse a recoverable operation, or an unconsumed binding whose
            // start was refused before status admission (e.g. busy slots).
            if self.get(&saved_plan.context_id).map_or(
                saved.consumption_attempted == Some(false),
                |status| {
                    status.phase != ManagedContextOutboundOperationPhase::Completed
                        && (status.phase != ManagedContextOutboundOperationPhase::Failed
                            || status.retryable)
                },
            ) {
                return Ok(saved.ticket);
            }
        }
        check_operation_capacity(&parent, &format!("{}-owner.json", plan.context_id))?;
        let saved = PersistedOwnerTicket {
            account_id: profile.account_id.clone(),
            user_id: profile.user_id.clone(),
            ticket: candidate.clone(),
            consumption_attempted: Some(false),
        };
        let bytes = serde_json::to_vec(&saved)
            .map_err(|_| outbound_service_error("serialize owner ticket binding", false))?;
        crate::config::write_private_file(&index, &bytes)
            .map_err(|error| outbound_service_io_error("persist owner ticket binding", error))?;
        crate::config::write_private_file(
            &parent.join(format!("{}-owner.json", plan.context_id)),
            &bytes,
        )
        .map_err(|error| outbound_service_io_error("persist owner context binding", error))?;
        Ok(candidate)
    }

    fn read_owner_authorization_binding(
        &self,
        config: &DaemonConfig,
        ticket: &ManagedContextTransferTicket,
    ) -> Result<Option<PersistedOwnerTicket>, DaemonError> {
        let Some(parent) = self.status_parent() else {
            return Ok(None);
        };
        let context_id = ticket.context_plan.context_id();
        if !valid_artifact_name(context_id) {
            return Err(outbound_service_error("invalid context ID", false));
        }
        let path = parent.join(format!("{context_id}-owner.json"));
        if !path_entry_exists(&path)? {
            return Ok(None);
        }
        let bytes = read_bounded_regular_file(&path, MAX_OUTBOUND_ARTIFACT_STATE_BYTES)?;
        let saved: PersistedOwnerTicket = serde_json::from_slice(&bytes)
            .map_err(|_| outbound_service_error("invalid owner authorization binding", false))?;
        let profile = config
            .cloud_relay
            .as_ref()
            .ok_or_else(|| outbound_service_error("source Cloud owner is unavailable", false))?;
        if saved.account_id != profile.account_id || saved.user_id != profile.user_id {
            return Err(crate::managed_context::owner_managed::admission_error(
                "source context belongs to another account or user",
            ));
        }
        if saved.ticket != *ticket {
            return Err(outbound_service_error(
                "owner authorization binding changed",
                false,
            ));
        }
        validate_ticket(config, &saved.ticket)?;
        Ok(Some(saved))
    }

    pub(crate) fn owner_ticket_may_issue(
        &self,
        config: &DaemonConfig,
        ticket: &ManagedContextTransferTicket,
    ) -> Result<bool, DaemonError> {
        let attempted = self
            .read_owner_authorization_binding(config, ticket)?
            .and_then(|saved| saved.consumption_attempted);
        // A Preparing/Failed status alone says nothing about consumption. New
        // bindings explicitly record false; resumed legacy bindings stay conservative.
        Ok(match attempted {
            Some(attempted) => !attempted,
            None => self.get(ticket.context_plan.context_id()).is_none(),
        })
    }

    pub(crate) fn record_owner_ticket_consumption_attempt(
        &self,
        config: &DaemonConfig,
        ticket: &ManagedContextTransferTicket,
    ) -> Result<(), DaemonError> {
        self.reclaim_operation_metadata(Some(ticket.context_plan.context_id()))?;
        let Some(parent) = self.status_parent() else {
            return Ok(());
        };
        let _guard = self
            .artifact_lock
            .lock()
            .expect("owner authorization binding lock");
        let mut saved = match self.read_owner_authorization_binding(config, ticket)? {
            Some(saved) => saved,
            None => {
                let profile = config.cloud_relay.as_ref().ok_or_else(|| {
                    outbound_service_error("source Cloud owner is unavailable", false)
                })?;
                PersistedOwnerTicket {
                    account_id: profile.account_id.clone(),
                    user_id: profile.user_id.clone(),
                    ticket: ticket.clone(),
                    consumption_attempted: Some(false),
                }
            }
        };
        if saved.consumption_attempted == Some(true) {
            return Ok(());
        }
        saved.consumption_attempted = Some(true);
        create_private_directory(&parent)?;
        let name = format!("{}-owner.json", ticket.context_plan.context_id());
        check_operation_capacity(&parent, &name)?;
        let bytes = serde_json::to_vec(&saved)
            .map_err(|_| outbound_service_error("serialize owner authorization binding", false))?;
        // Persist before sending consume, including cases where its reply is lost.
        crate::config::write_private_file(&parent.join(name), &bytes)
            .map_err(|error| outbound_service_io_error("persist owner consumption attempt", error))
    }

    pub(crate) fn authorize_status_owner(
        &self,
        config: &DaemonConfig,
        context_id: &str,
    ) -> Result<(), DaemonError> {
        if !valid_artifact_name(context_id) {
            return Err(outbound_service_error("invalid context ID", false));
        }
        let Some(parent) = self.status_parent() else {
            return Ok(());
        };
        let path = parent.join(format!("{context_id}-owner.json"));
        if !path_entry_exists(&path)? {
            return Ok(());
        }
        let bytes = read_bounded_regular_file(&path, MAX_OUTBOUND_ARTIFACT_STATE_BYTES)?;
        let saved: PersistedOwnerTicket = serde_json::from_slice(&bytes)
            .map_err(|_| outbound_service_error("invalid owner status binding", false))?;
        let profile = config
            .cloud_relay
            .as_ref()
            .ok_or_else(|| outbound_service_error("source Cloud owner is unavailable", false))?;
        if saved.account_id != profile.account_id || saved.user_id != profile.user_id {
            return Err(crate::managed_context::owner_managed::admission_error(
                "source context belongs to another account or user",
            ));
        }
        validate_ticket(config, &saved.ticket)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedOwnerTicket {
    account_id: String,
    user_id: String,
    ticket: ManagedContextTransferTicket,
    // Kernel-private checkpoint; absent in legacy records means unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    consumption_attempted: Option<bool>,
}

// MP-08/MP-11: durable metadata is bounded separately from archive retention.
fn check_operation_capacity(parent: &Path, name: &str) -> Result<(), DaemonError> {
    if path_entry_exists(&parent.join(name))? {
        return Ok(());
    }
    let count = fs::read_dir(parent)
        .map_err(|error| outbound_service_io_error("count transfer operations", error))?
        .take(MAX_OUTBOUND_OPERATIONS * 3)
        .count();
    if count >= MAX_OUTBOUND_OPERATIONS * 3 - 2 {
        return Err(outbound_service_error(
            "durable transfer operation capacity reached",
            false,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_context::owner_managed::*;

    fn write_owner_metadata(store: &ManagedContextOutboundOperationStore, context_id: &str) {
        write_owner_metadata_for_ticket(
            store,
            super::super::tests::persisted_test_ticket(context_id),
        );
    }

    fn write_owner_metadata_for_ticket(
        store: &ManagedContextOutboundOperationStore,
        ticket: ManagedContextTransferTicket,
    ) {
        let context_id = ticket.context_plan.context_id().to_owned();
        let plan = ticket.context_plan.package_binding();
        let saved = PersistedOwnerTicket {
            account_id: "account".into(),
            user_id: "owner".into(),
            ticket,
            consumption_attempted: Some(false),
        };
        let bytes = serde_json::to_vec(&saved).unwrap();
        let parent = store.status_parent().unwrap();
        for name in [
            format!("{context_id}-owner.json"),
            format!(
                "owner-{}.json",
                plan.plan_digest.trim_start_matches("sha256:")
            ),
        ] {
            crate::config::write_private_file(&parent.join(name), &bytes).unwrap();
        }
    }

    #[test]
    fn mp08_mp11_durable_terminal_metadata_reclaims_capacity_across_restarts() {
        let root = std::env::temp_dir().join(format!(
            "chariox-operation-retention-{:032x}",
            rand::random::<u128>()
        ));
        let _cleanup = ArtifactRootCleanup::new(root.clone());
        let mut store = ManagedContextOutboundOperationStore::open(root.clone()).unwrap();
        for index in 0..MAX_OUTBOUND_OPERATIONS * 4 {
            let context_id = format!("context-retention-{index:04}");
            let ticket = super::super::tests::persisted_test_ticket(&context_id);
            let (_, permit) = store
                .start(
                    &context_id,
                    &ticket.context_plan.package_binding().plan_digest,
                )
                .expect("ordinary completions and cancellations cannot consume lifetime capacity");
            write_owner_metadata(&store, &context_id);
            assert!(store.update(&context_id, |status| {
                status.phase = if index % 2 == 0 {
                    ManagedContextOutboundOperationPhase::Completed
                } else {
                    ManagedContextOutboundOperationPhase::Failed
                };
                status.retryable = false;
            }));
            drop(permit);
            store.finish(&context_id);
            if index % 32 == 31 {
                store = ManagedContextOutboundOperationStore::open(root.clone()).unwrap();
            }
        }
        let reopened = ManagedContextOutboundOperationStore::open(root.clone()).unwrap();
        assert!(reopened
            .start("context-after-reopen", "sha256:next")
            .is_ok());
        assert!(
            reopened.get("context-retention-0000").is_none(),
            "evicted durable status must disappear too"
        );
        assert!(!root
            .join(".operations/context-retention-0000-owner.json")
            .exists());
        assert!(
            fs::read_dir(root.join(".operations")).unwrap().count() < MAX_OUTBOUND_OPERATIONS * 3
        );
    }

    #[test]
    fn r4_abandoned_retryable_operations_retire_after_recovery_state_expires() {
        let root = std::env::temp_dir().join(format!(
            "chariox-operation-abandoned-{:032x}",
            rand::random::<u128>()
        ));
        let _cleanup = ArtifactRootCleanup::new(root.clone());
        let store = ManagedContextOutboundOperationStore::open(root.clone()).unwrap();
        // Retryable failures are never terminal receipts, so enough abandoned
        // copies fill the metadata quota while no package or upload remains.
        let ids = (0..MAX_OUTBOUND_OPERATIONS - 1)
            .map(|index| format!("context-abandoned-{index:04}"))
            .collect::<Vec<_>>();
        for id in &ids {
            let mut ticket = super::super::tests::persisted_test_ticket(id);
            ticket.context_plan = ManagedKernelContextPlan::source_project_for_tests(
                id,
                "realm-1",
                "source-kernel",
                &"a".repeat(64),
                &format!("project-{id}"),
            );
            let (_, permit) = store
                .start(id, &ticket.context_plan.package_binding().plan_digest)
                .unwrap();
            write_owner_metadata_for_ticket(&store, ticket);
            assert!(store.update(id, |status| {
                status.phase = ManagedContextOutboundOperationPhase::Failed;
                status.retryable = true;
            }));
            drop(permit);
            store.finish(id);
        }
        // Before expiration this owner's metadata quota is actually full.
        crate::config::write_private_file(&root.join(".operations/unknown.json"), b"{}").unwrap();
        assert!(store
            .start("context-before-expiration", "sha256:next")
            .is_err());
        // Only the oldest are past the recovery window; the last stays viable.
        let (viable, expired) = ids.split_last().unwrap();
        for id in expired {
            let mut status = store.get(id).unwrap();
            status.updated_at_ms = 1;
            store.persist_status(&status).unwrap();
        }
        drop(store);
        let reopened = ManagedContextOutboundOperationStore::open(root.clone()).unwrap();
        for id in expired {
            assert!(reopened.get(id).is_none(), "{id} is no longer recoverable");
            assert!(!root
                .join(".operations")
                .join(format!("{id}-owner.json"))
                .exists());
        }
        assert!(
            reopened.get(viable).is_some(),
            "a viable retry is preserved"
        );
        let next = super::super::tests::persisted_test_ticket("context-after-reopen");
        assert!(reopened
            .start(
                "context-after-reopen",
                &next.context_plan.package_binding().plan_digest
            )
            .unwrap()
            .1
            .is_some());
    }

    #[test]
    fn r4_retirement_preserves_active_operations_and_unexpired_recovery() {
        // MP-08 / MP-11: neither age nor pressure retires live authority.
        let root = std::env::temp_dir().join(format!(
            "chariox-operation-live-{:032x}",
            rand::random::<u128>()
        ));
        let _cleanup = ArtifactRootCleanup::new(root.clone());
        let store = ManagedContextOutboundOperationStore::open(root.clone()).unwrap();
        let active = "context-active-old";
        let viable = "context-viable-old";
        let mut active_permit = None;
        for id in [active, viable] {
            let ticket = super::super::tests::persisted_test_ticket(id);
            let (_, permit) = store
                .start(id, &ticket.context_plan.package_binding().plan_digest)
                .unwrap();
            write_owner_metadata(&store, id);
            let mut status = store.get(id).unwrap();
            status.updated_at_ms = 1;
            if id == viable {
                status.phase = ManagedContextOutboundOperationPhase::Failed;
                status.retryable = true;
                drop(permit);
                store.finish(id);
                let artifact_root = root.join(id);
                create_private_directory(&artifact_root).unwrap();
                let persisted = super::super::tests::persisted_test_artifact(
                    &ticket,
                    crate::session::unix_epoch_ms(),
                    1,
                );
                crate::config::write_private_file(
                    &artifact_root.join("state.json"),
                    &serde_json::to_vec(&persisted).unwrap(),
                )
                .unwrap();
                crate::config::write_private_file(&artifact_root.join("managed-context.pkg"), b"x")
                    .unwrap();
            } else {
                active_permit = permit;
            }
            store.persist_status(&status).unwrap();
            store.state.lock().unwrap().insert(id.into(), status);
        }
        store.reclaim_operation_metadata(None).unwrap();
        assert!(
            store.get(active).is_some(),
            "a live old operation retains authority"
        );
        assert!(
            store.get(viable).is_some(),
            "a matching unexpired package retains retry authority"
        );
        drop(active_permit);
        store.finish(active);
        drop(store);
        let reopened = ManagedContextOutboundOperationStore::open(root.clone()).unwrap();
        assert!(
            reopened.get(active).is_none(),
            "abandoned old operation has no recovery package"
        );
        assert!(
            reopened.get(viable).is_some(),
            "unexpired package still resumes after reopen"
        );
    }

    #[test]
    fn mp08_mp11_metadata_retirement_preserves_unfinished_and_retryable_bindings() {
        let root = std::env::temp_dir().join(format!(
            "chariox-operation-expiry-{:032x}",
            rand::random::<u128>()
        ));
        let _cleanup = ArtifactRootCleanup::new(root.clone());
        let store = ManagedContextOutboundOperationStore::open(root.clone()).unwrap();
        for (id, phase, retryable) in [
            (
                "context-active",
                ManagedContextOutboundOperationPhase::Preparing,
                false,
            ),
            (
                "context-retry",
                ManagedContextOutboundOperationPhase::Failed,
                true,
            ),
            (
                "context-completed",
                ManagedContextOutboundOperationPhase::Completed,
                false,
            ),
            (
                "context-cancelled",
                ManagedContextOutboundOperationPhase::Failed,
                false,
            ),
        ] {
            let ticket = super::super::tests::persisted_test_ticket(id);
            let (_, permit) = store
                .start(id, &ticket.context_plan.package_binding().plan_digest)
                .unwrap();
            write_owner_metadata(&store, id);
            let mut status = store.get(id).unwrap();
            status.phase = phase;
            status.retryable = retryable;
            status.updated_at_ms =
                if retryable || phase == ManagedContextOutboundOperationPhase::Preparing {
                    crate::session::unix_epoch_ms()
                } else {
                    1
                };
            store.persist_status(&status).unwrap();
            drop(permit);
            store.finish(id);
        }
        drop(store);
        let reopened = ManagedContextOutboundOperationStore::open(root.clone()).unwrap();
        for id in ["context-active", "context-retry"] {
            assert!(reopened.get(id).is_some());
            assert!(root
                .join(".operations")
                .join(format!("{id}-owner.json"))
                .is_file());
        }
        for id in ["context-completed", "context-cancelled"] {
            assert!(
                reopened.get(id).is_none(),
                "expired terminal status must retire"
            );
            assert!(!root
                .join(".operations")
                .join(format!("{id}-owner.json"))
                .exists());
        }
        assert!(reopened
            .start(
                "context-retry",
                &super::super::tests::persisted_test_ticket("context-retry")
                    .context_plan
                    .package_binding()
                    .plan_digest
            )
            .unwrap()
            .1
            .is_some());
    }

    #[test]
    fn mp08_mp11_retirement_tombstone_settles_after_crash() {
        let root = std::env::temp_dir().join(format!(
            "chariox-operation-tombstone-{:032x}",
            rand::random::<u128>()
        ));
        let _cleanup = ArtifactRootCleanup::new(root.clone());
        let store = ManagedContextOutboundOperationStore::open(root.clone()).unwrap();
        let id = "context-retiring";
        let ticket = super::super::tests::persisted_test_ticket(id);
        let digest = ticket.context_plan.package_binding().plan_digest;
        let (_, permit) = store.start(id, &digest).unwrap();
        write_owner_metadata(&store, id);
        assert!(store.update(id, |status| status.phase =
            ManagedContextOutboundOperationPhase::Completed));
        drop(permit);
        store.finish(id);
        let parent = store.status_parent().unwrap();
        fs::rename(
            parent.join(format!("{id}.json")),
            parent.join(format!(".retired-{id}.json")),
        )
        .unwrap();
        drop(store);
        let reopened = ManagedContextOutboundOperationStore::open(root).unwrap();
        assert!(reopened.get(id).is_none());
        assert_eq!(
            fs::read_dir(parent).unwrap().count(),
            0,
            "a young retirement tombstone must settle all matching owner files on reopen"
        );
    }

    #[test]
    fn mp08_mp11_retirement_keeps_rebound_plan_index_and_live_terminal_status() {
        let root = std::env::temp_dir().join(format!(
            "chariox-operation-rebound-{:032x}",
            rand::random::<u128>()
        ));
        let _cleanup = ArtifactRootCleanup::new(root.clone());
        let store = ManagedContextOutboundOperationStore::open(root.clone()).unwrap();
        let first = "context-old";
        let second = "context-new";
        let digest = super::super::tests::persisted_test_ticket(first)
            .context_plan
            .package_binding()
            .plan_digest;
        let (_, permit) = store.start(first, &digest).unwrap();
        write_owner_metadata(&store, first);
        assert!(store.update(first, |status| status.phase =
            ManagedContextOutboundOperationPhase::Completed));
        let mut status = store.get(first).unwrap();
        status.updated_at_ms = 1;
        store.persist_status(&status).unwrap();
        store.reclaim_operation_metadata(None).unwrap();
        assert!(
            store.get(first).is_some(),
            "live terminal update still owns its recovery bindings"
        );
        drop(permit);
        store.finish(first);
        let (_, second_permit) = store.start(second, &digest).unwrap();
        write_owner_metadata(&store, second);
        // Model a shared plan index already rebound while an old receipt remains.
        crate::config::write_private_file(
            &store.status_parent().unwrap().join(format!("{first}.json")),
            &serde_json::to_vec(&status).unwrap(),
        )
        .unwrap();
        write_owner_metadata(&store, first);
        write_owner_metadata(&store, second);
        drop(second_permit);
        store.finish(second);
        drop(store);
        let reopened = ManagedContextOutboundOperationStore::open(root).unwrap();
        assert!(reopened.get(first).is_none());
        assert!(reopened.get(second).is_some());
        let parent = reopened.status_parent().unwrap();
        assert!(!parent.join(format!("{first}-owner.json")).exists());
        let saved: PersistedOwnerTicket = serde_json::from_slice(
            &fs::read(parent.join(format!(
                "owner-{}.json",
                digest.trim_start_matches("sha256:")
            )))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(saved.ticket.context_plan.context_id(), second);
    }

    #[test]
    fn mp08_mp11_retirement_preserves_ambiguous_tombstone_with_unfinished_status() {
        let root = std::env::temp_dir().join(format!(
            "chariox-operation-ambiguous-{:032x}",
            rand::random::<u128>()
        ));
        let _cleanup = ArtifactRootCleanup::new(root.clone());
        let store = ManagedContextOutboundOperationStore::open(root.clone()).unwrap();
        let id = "context-ambiguous";
        let digest = super::super::tests::persisted_test_ticket(id)
            .context_plan
            .package_binding()
            .plan_digest;
        let (_, permit) = store.start(id, &digest).unwrap();
        write_owner_metadata(&store, id);
        let mut old = store.get(id).unwrap();
        old.phase = ManagedContextOutboundOperationPhase::Completed;
        old.updated_at_ms = 1;
        let parent = store.status_parent().unwrap();
        crate::config::write_private_file(
            &parent.join(format!(".retired-{id}.json")),
            &serde_json::to_vec(&old).unwrap(),
        )
        .unwrap();
        drop(permit);
        store.finish(id);
        drop(store);
        let reopened = ManagedContextOutboundOperationStore::open(root).unwrap();
        assert_eq!(
            reopened.get(id).unwrap().phase,
            ManagedContextOutboundOperationPhase::Preparing
        );
        assert!(parent.join(format!("{id}-owner.json")).exists());
        assert!(
            parent.join(format!(".retired-{id}.json")).exists(),
            "ambiguous retirement must retain unfinished authority"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn mp08_mp11_source_owner_operation_status_and_binding_survive_restart() {
        crate::test_support::isolated_env_test!();
        let _lock = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-owner-status-{:032x}",
            rand::random::<u128>()
        ));
        create_private_directory(&root).unwrap();
        let _cleanup = ArtifactRootCleanup::new(root.clone());
        let mut config = DaemonConfig::for_tests();
        config.user_config.state.path = Some(root.join("state.db").display().to_string());
        config.user_config_path = root.join("config.toml");
        config.user_config.history.operational.path =
            Some(root.join("history.db").display().to_string());
        config.user_config.artifacts.operational.root =
            Some(root.join("artifacts").display().to_string());
        config.user_config.artifacts.operational.index_path =
            Some(root.join("artifacts.db").display().to_string());
        config = config.with_session_history_root(root.join("sessions"));
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            account_id: "account".into(),
            user_id: "owner".into(),
            realm_id: "realm".into(),
            machine_id: Some(config.host_machine_id.clone()),
            kernel_id: Some(config.daemon_id.clone()),
            kernel_credential: Some("synthetic-test-enrollment".into()),
            kernel_public_key_thumbprint: Some(public_key_thumbprint(&config.relay_public_key)),
            ..Default::default()
        });
        let app = crate::app::DaemonApp::bootstrap(config.clone()).unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
            Arc::new(tokio::sync::Mutex::new(app)),
            1,
        );
        let runtime = router.runtime_state();
        let selection = OwnerManagedTransfer {
            target: ManagedContextTransferTarget {
                relay_realm_id: "realm".into(),
                machine_id: "target-machine".into(),
                kernel_id: "target-kernel".into(),
                relay_public_key: config.relay_public_key.clone(),
                key_thumbprint: public_key_thumbprint(&config.relay_public_key),
            },
            context_selection: OwnerManagedContextSelection {
                kernel_context: OwnerManagedKernelSelection::Empty,
                development_setup: OwnerManagedDevelopmentSelection::Empty,
            },
        };
        let store = ManagedContextOutboundOperationStore::open(root.join("outbound")).unwrap();
        // MP-08/MP-11: enrolled local clients create runtime Projects as the
        // authenticated owner, even though provider credentials use `local`.
        let worktree = crate::test_support::TestWorktree::new("owner-source-project");
        let caller = router
            .local_command_caller(
                crate::runtime::command::KernelCommandSource::LocalCli,
                crate::local::KernelConnectionClass::Terminal,
            )
            .await;
        let response = runtime
            .create_session_response(
                worktree
                    .session_request()
                    .with_owner_user_id(caller.user_id.unwrap()),
            )
            .await
            .unwrap();
        let crate::local::LocalDaemonResponse::SessionCreated { session, .. } = response else {
            panic!("session response")
        };
        assert_eq!(session.owner_user_id(), "owner");
        let mut project_selection = selection.clone();
        project_selection.context_selection.development_setup =
            OwnerManagedDevelopmentSelection::SourceProject {
                project_id: session.project_id().to_string(),
                repositories: vec![OwnerManagedRepositorySelection {
                    role: crate::managed_context::development::DevelopmentRepositoryRole::Primary,
                    workspace_id: session.workspace_id().to_string(),
                    worktree_id: None,
                }],
            };
        let project_ticket = store
            .prepare_owner_ticket(&config, &runtime, project_selection)
            .expect("MP-08 enrolled runtime owner Project must be admitted");
        let project_plan = project_ticket.context_plan.package_binding();
        let (_, project_permit) = store
            .start(&project_plan.context_id, &project_plan.plan_digest)
            .unwrap();
        assert!(project_permit.is_some());
        drop(project_permit);
        store.finish(&project_plan.context_id);
        for (runtime_owner, allowed) in [("local", true), ("foreign-owner", false)] {
            let workspace =
                crate::test_support::TestWorktree::new("owner-admission-legacy-foreign");
            let response = runtime
                .create_session_response(
                    workspace
                        .session_request()
                        .with_owner_user_id(runtime_owner),
                )
                .await
                .unwrap();
            let crate::local::LocalDaemonResponse::SessionCreated { session, .. } = response else {
                panic!("session response")
            };
            let mut selection = selection.clone();
            selection.context_selection.development_setup =
                OwnerManagedDevelopmentSelection::SourceProject {
                    project_id: session.project_id().to_string(),
                    repositories: vec![OwnerManagedRepositorySelection {
                        role:
                            crate::managed_context::development::DevelopmentRepositoryRole::Primary,
                        workspace_id: session.workspace_id().to_string(),
                        worktree_id: None,
                    }],
                };
            assert_eq!(
                store
                    .prepare_owner_ticket(&config, &runtime, selection)
                    .is_ok(),
                allowed
            );
        }
        let mut ticket = store
            .prepare_owner_ticket(&config, &runtime, selection.clone())
            .unwrap();
        // MP-08/MP-11: a failed first status write leaves no phantom operation;
        // owner preparation reuses the unconsumed binding and can start it.
        let failed_context = ticket.context_plan.context_id().to_string();
        let blocked_status = root
            .join("outbound/.operations")
            .join(format!("{failed_context}.json"));
        fs::create_dir(&blocked_status).unwrap();
        assert!(store
            .start(
                &failed_context,
                &ticket.context_plan.package_binding().plan_digest
            )
            .is_err());
        assert!(store.get(&failed_context).is_none());
        assert!(store.active_context_ids().is_empty());
        fs::remove_dir(&blocked_status).unwrap();
        ticket = store
            .prepare_owner_ticket(&config, &runtime, selection.clone())
            .unwrap();
        assert_eq!(ticket.context_plan.context_id(), failed_context);
        let plan = ticket.context_plan.package_binding();
        // MP-08/MP-11: ambiguous/noninteractive admission and a disconnected
        // source stop before creating an operation or reaching Cloud.
        for (interactive, include_ticket, include_owner) in [
            (false, false, true),
            (true, true, true),
            (true, false, false),
            (true, false, true),
        ] {
            let request = crate::local::LocalDaemonRequest::StartManagedContextTransfer(
                crate::local::StartManagedContextTransferRequest {
                    ticket: include_ticket.then(|| ticket.clone()),
                    owner_managed: include_owner.then(|| selection.clone()),
                    interactive,
                },
            );
            let error = crate::runtime::managed_context_outbound_control::execute_managed_context_outbound_request(
                config.clone(), Arc::new(RwLock::new(RelayClientState::default())), store.clone(),
                runtime.provider_account_profile_registry().clone(), runtime.clone(), "owner", request,
            ).unwrap_err();
            if interactive && include_owner && !include_ticket {
                assert!(error
                    .to_string()
                    .contains("Source kernel is offline or stale"));
            }
            assert!(store.get(&plan.context_id).is_none());
        }
        let (_, permit) = store.start(&plan.context_id, &plan.plan_digest).unwrap();
        drop(permit);
        let reopened = ManagedContextOutboundOperationStore::open(root.join("outbound")).unwrap();
        assert_eq!(
            reopened.get(&plan.context_id).unwrap().plan_digest,
            plan.plan_digest
        );
        assert_eq!(
            reopened
                .prepare_owner_ticket(&config, &runtime, selection.clone())
                .unwrap(),
            ticket
        );
        reopened
            .authorize_status_owner(&config, &plan.context_id)
            .unwrap();
        let mut foreign = config.clone();
        foreign.cloud_relay.as_mut().unwrap().user_id = "another-owner".into();
        assert!(reopened
            .authorize_status_owner(&foreign, &plan.context_id)
            .is_err());
        foreign.cloud_relay.as_mut().unwrap().user_id = "owner".into();
        foreign.cloud_relay.as_mut().unwrap().account_id = "another-account".into();
        assert!(reopened
            .authorize_status_owner(&foreign, &plan.context_id)
            .is_err());
        let mut unavailable = selection.clone();
        unavailable.context_selection.development_setup =
            OwnerManagedDevelopmentSelection::SourceProject {
                project_id: "missing-project".into(),
                repositories: vec![],
            };
        assert!(reopened
            .prepare_owner_ticket(&config, &runtime, unavailable)
            .is_err());
        // MP-08/MP-11: a start refused before status admission (both transfer
        // slots busy) reuses its prepared binding instead of orphaning one per
        // retry; reopening retires a legacy status-less binding no index names.
        let held = (0..2)
            .map(|index| {
                reopened
                    .start(&format!("context-slot-{index}"), "sha256:slot")
                    .unwrap()
                    .1
                    .expect("occupy a transfer slot")
            })
            .collect::<Vec<_>>();
        let mut busy = selection.clone();
        busy.target.machine_id = "busy-target-machine".into();
        let first = reopened
            .prepare_owner_ticket(&config, &runtime, busy.clone())
            .unwrap();
        for _ in 0..4 {
            let plan = first.context_plan.package_binding();
            assert!(reopened.start(&plan.context_id, &plan.plan_digest).is_err());
            assert_eq!(
                reopened
                    .prepare_owner_ticket(&config, &runtime, busy.clone())
                    .unwrap(),
                first
            );
        }
        drop(held);
        write_owner_metadata(&reopened, "context-orphan-a");
        write_owner_metadata(&reopened, "context-orphan-b");
        let operations = root.join("outbound/.operations");
        let reopened = ManagedContextOutboundOperationStore::open(root.join("outbound")).unwrap();
        reopened.reclaim_operation_metadata(None).unwrap();
        assert!(!operations.join("context-orphan-a-owner.json").exists());
        assert!(operations.join("context-orphan-b-owner.json").exists());
        let names = fs::read_dir(&operations)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let bindings = names.iter().filter(|name| name.ends_with("-owner.json"));
        let indexes = names.iter().filter(|name| {
            name.strip_prefix("owner-")
                .and_then(|name| name.strip_suffix(".json"))
                .is_some_and(|digest| {
                    digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
        });
        assert_eq!(
            bindings.count(),
            indexes.count(),
            "one context binding per prepared owner plan"
        );
        let plan = first.context_plan.package_binding();
        assert!(reopened.start(&plan.context_id, &plan.plan_digest).is_ok());
        drop(router);
        fs::remove_dir_all(root).unwrap();
    }
}
