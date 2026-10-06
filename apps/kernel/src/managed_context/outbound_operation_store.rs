//! MP-08/MP-11: restart-safe status and owner/target operation bindings.
use super::*;

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
            if self.get(&saved_plan.context_id).is_some_and(|status| {
                status.phase != ManagedContextOutboundOperationPhase::Completed
                    && (status.phase != ManagedContextOutboundOperationPhase::Failed
                        || status.retryable)
            }) {
                return Ok(saved.ticket);
            }
        }
        check_operation_capacity(&parent, &format!("{}-owner.json", plan.context_id))?;
        let saved = PersistedOwnerTicket {
            account_id: profile.account_id.clone(),
            user_id: profile.user_id.clone(),
            ticket: candidate.clone(),
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
        let mut ticket = store
            .prepare_owner_ticket(&config, &runtime, selection.clone())
            .unwrap();
        // MP-08/MP-11: a failed first status write cannot pin owner preparation
        // to an operation that has no active execution.
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
        assert_ne!(ticket.context_plan.context_id(), failed_context);
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
        let mut unavailable = selection;
        unavailable.context_selection.development_setup =
            OwnerManagedDevelopmentSelection::SourceProject {
                project_id: "missing-project".into(),
                repositories: vec![],
            };
        assert!(reopened
            .prepare_owner_ticket(&config, &runtime, unavailable)
            .is_err());
        drop(router);
        fs::remove_dir_all(root).unwrap();
    }
}
