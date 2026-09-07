mod display;
mod local_docker;
mod model;
mod ports;
mod store;

pub(crate) use local_docker::managed_docker_broker_configured;
pub(crate) use local_docker::{
    cleanup_replaced_saved_state_generation, recover_pending_local_docker_slice_backup_restore,
    remove_local_docker_slice_backup_best_effort, restore_local_docker_slice_backup,
    SliceBackupRestoreResolution,
};
pub use local_docker::{
    collect_local_docker_slice_logs, create_local_docker_slice_backup,
    create_local_docker_slice_backup_live, default_local_docker_saved_state,
    initialize_managed_docker_broker, inspect_local_docker_slice_host_runtime,
    inspect_local_docker_slice_provider_auth, local_docker_private_relay,
    local_docker_private_relay_endpoint, local_docker_private_relay_token,
    remove_local_docker_saved_state, run_local_docker_slice_action, save_local_docker_slice_state,
    save_local_docker_slice_state_live, set_local_docker_default_saved_state,
    start_local_docker_slice_provider_login, validate_local_docker_slice_backup,
    LocalDockerProviderAccount, LocalDockerSliceOptions, LocalDockerSliceRelay,
};
#[cfg(test)]
use local_docker::{
    ensure_local_docker_slice_ports_available, local_docker_slice_action_log_path,
    relay_url_for_container,
};
pub use model::{
    CreateSliceInput, LocalDockerSliceAction, SliceBackendKind, SliceBackupRecord,
    SliceBackupRestoreTransactionRecord, SliceDevelopmentPublication, SliceDisplayBackend,
    SliceDisplayEndpoint, SliceDisplayEndpointAccess, SliceDisplayEndpointKind, SliceDisplayMode,
    SliceLocalDockerPorts, SliceLogEntry, SliceOperationStatus, SliceProviderLoginStart,
    SliceRecord, SliceRelayEndpoint, SliceSavedStateRecord, SliceSavedStateStatus, SliceStatus,
};
#[cfg(test)]
use ports::LocalDockerSlicePorts;
pub use store::{SliceAgentAttachment, SliceHostRuntimeState, SliceOperationGuard, SliceStore};

#[cfg(test)]
mod tests {
    use std::fs;
    use std::net::TcpListener;

    use super::*;
    use crate::config::SliceImageBuildPolicy;
    use crate::slice_provider_auth::SliceProviderAuthSummary;

    fn create_input(name: &str) -> CreateSliceInput {
        CreateSliceInput {
            name: name.to_string(),
            backend: SliceBackendKind::LocalDocker,
            os: "linux".to_string(),
            display_mode: SliceDisplayMode::Headed,
            display_backend: Default::default(),
            workspace_id: None,
            worktree_id: None,
            workspace_mount: Some("/repo".to_string()),
            development: None,
            worker_kernel_ref: None,
            display_url: Some("http://127.0.0.1:6080".to_string()),
            provider_auth: Vec::new(),
            from_saved_state: None,
            now_ms: 42,
        }
    }

    fn saved_state(id: &str) -> SliceSavedStateRecord {
        SliceSavedStateRecord {
            id: id.to_string(),
            slice_name: "gmail-ready".to_string(),
            source_slice_id: "slice-source".to_string(),
            backend: SliceBackendKind::LocalDocker,
            os: "linux".to_string(),
            image_ref: "chariox-slice-state:gmail-ready".to_string(),
            home_archive_path: "/tmp/gmail-ready-home.tar.zst".to_string(),
            manifest_path: "/tmp/gmail-ready-manifest.json".to_string(),
            created_at_ms: 1,
            updated_at_ms: 2,
            size_bytes: Some(1024),
            last_operation: Some("state.save".to_string()),
            last_operation_status: Some(SliceOperationStatus::Completed),
            last_error: None,
        }
    }

    #[test]
    fn saved_state_record_is_persisted_before_the_in_memory_pointer_changes() {
        let store = SliceStore::default();
        let slice = store
            .create("kernel-1", "machine-1", create_input("transactional-state"))
            .expect("slice should create");
        let state = saved_state("transactional-state");
        let error = store
            .upsert_saved_state_transactionally(&slice.id, state.clone(), 43, |_, _| {
                Err(crate::error::DaemonError::LocalTransport {
                    operation: "slice.state.save",
                    message: "injected durable-state failure".to_string(),
                })
            })
            .expect_err("failed persistence must reject the state update");

        assert!(error.to_string().contains("injected durable-state failure"));
        assert_eq!(
            store
                .resolve(&slice.id)
                .expect("slice should remain available")
                .saved_state_ref,
            None
        );
        assert!(store.list_saved_states().is_empty());

        store
            .upsert_saved_state_transactionally(&slice.id, state.clone(), 44, |record, saved| {
                assert_eq!(record.saved_state_ref.as_deref(), Some(saved.id.as_str()));
                assert_eq!(
                    store
                        .resolve(&slice.id)
                        .expect("uncommitted slice should remain readable")
                        .saved_state_ref,
                    None
                );
                Ok(())
            })
            .expect("persisted state should commit in memory");
        assert_eq!(
            store
                .resolve(&slice.id)
                .expect("slice should remain available")
                .saved_state_ref
                .as_deref(),
            Some(state.id.as_str())
        );
        assert_eq!(store.list_saved_states(), vec![state]);
    }

    fn backup(id: &str, name: &str, source_slice_id: &str) -> SliceBackupRecord {
        SliceBackupRecord {
            id: id.to_string(),
            name: name.to_string(),
            source_slice_id: source_slice_id.to_string(),
            source_state_id: "dev".to_string(),
            image_ref: format!("chariox-slice-backup:{id}"),
            home_archive_path: format!("/tmp/{id}-home.tar.zst"),
            manifest_path: format!("/tmp/{id}-manifest.json"),
            created_at_ms: 1,
            size_bytes: Some(1024),
            home_archive_sha256: Some("a".repeat(64)),
            image_id: Some("sha256:fixture".to_string()),
        }
    }

    fn restore_transaction(id: &str, source_slice_id: &str) -> SliceBackupRestoreTransactionRecord {
        SliceBackupRestoreTransactionRecord {
            id: id.to_string(),
            source_slice_id: source_slice_id.to_string(),
            target_backup: backup("target", "target", source_slice_id),
            rollback_backup: backup("rollback", "rollback", source_slice_id),
            previous_saved_state: None,
            started_at_ms: 45,
        }
    }

    #[test]
    fn backup_restore_transaction_is_durable_before_mutation_and_resolves_atomically() {
        let store = SliceStore::default();
        let slice = store
            .create("kernel-1", "machine-1", create_input("restore-transaction"))
            .expect("slice should create");
        let transaction = restore_transaction("restore-1", &slice.id);
        let error = store
            .begin_backup_restore_transactionally(transaction.clone(), |_| {
                Err(crate::error::DaemonError::LocalTransport {
                    operation: "slice.backup.restore",
                    message: "injected journal failure".to_string(),
                })
            })
            .expect_err("restore must not begin without a durable journal record");
        assert!(error.to_string().contains("injected journal failure"));
        assert!(store.list_pending_backup_restores().is_empty());

        store
            .begin_backup_restore_transactionally(transaction.clone(), |_| Ok(()))
            .expect("restore journal should persist");
        assert_eq!(
            store.list_pending_backup_restores(),
            vec![transaction.clone()]
        );

        let restored_state = saved_state("restore-transaction");
        let error = store
            .resolve_backup_restore_transactionally(
                &transaction.id,
                &slice.id,
                restored_state.clone(),
                46,
                SliceOperationStatus::Completed,
                None,
                |_, _| {
                    Err(crate::error::DaemonError::LocalTransport {
                        operation: "slice.backup.restore",
                        message: "injected resolution failure".to_string(),
                    })
                },
            )
            .expect_err("failed resolution persistence must retain recovery intent");
        assert!(error.to_string().contains("injected resolution failure"));
        assert_eq!(
            store.list_pending_backup_restores(),
            vec![transaction.clone()]
        );
        assert_eq!(
            store
                .resolve(&slice.id)
                .expect("slice should remain available")
                .saved_state_ref,
            None
        );

        store
            .resolve_backup_restore_transactionally(
                &transaction.id,
                &slice.id,
                restored_state.clone(),
                47,
                SliceOperationStatus::Completed,
                None,
                |record, state| {
                    assert_eq!(record.saved_state_ref.as_deref(), Some(state.id.as_str()));
                    assert_eq!(
                        store.list_pending_backup_restores(),
                        vec![transaction.clone()]
                    );
                    Ok(())
                },
            )
            .expect("durable resolution should commit state and clear recovery intent");
        assert!(store.list_pending_backup_restores().is_empty());
        assert_eq!(
            store
                .active_saved_state_for_slice(&slice.id)
                .expect("state lookup should succeed"),
            Some(restored_state)
        );
    }

    #[test]
    fn unresolved_backup_restore_quarantines_slice_until_durable_resolution() {
        let store = SliceStore::default();
        let slice = store
            .create("kernel-1", "machine-1", create_input("restore-quarantine"))
            .expect("slice should create");
        let guard = store
            .try_begin_operation(&slice.id, "slice.backup.restore")
            .expect("restore should acquire the slice operation guard");
        let transaction = restore_transaction("restore-quarantine-1", &slice.id);
        store
            .begin_backup_restore_transactionally(transaction.clone(), |_| Ok(()))
            .expect("restore journal should persist");
        store
            .resolve_backup_restore_transactionally(
                &transaction.id,
                &slice.id,
                saved_state("failed-restore-state"),
                46,
                SliceOperationStatus::Failed,
                Some("automatic rollback failed".to_string()),
                |_, _| {
                    Err(crate::error::DaemonError::LocalTransport {
                        operation: "slice.backup.restore",
                        message: "injected rollback publication failure".to_string(),
                    })
                },
            )
            .expect_err("failed rollback publication must retain recovery intent");

        drop(guard);
        for operation in ["slice.start", "slice.delete", "slice.backup.restore"] {
            let error = store
                .try_begin_operation(&slice.id, operation)
                .expect_err("unresolved restore must quarantine every later operation");
            assert!(error.to_string().contains(&transaction.id));
        }

        let mut duplicate_persisted = false;
        let error = store
            .begin_backup_restore_transactionally(
                restore_transaction("restore-quarantine-2", &slice.id),
                |_| {
                    duplicate_persisted = true;
                    Ok(())
                },
            )
            .expect_err("a slice must not acquire a second pending restore");
        assert!(error.to_string().contains(&transaction.id));
        assert!(!duplicate_persisted);

        store
            .resolve_backup_restore_transactionally(
                &transaction.id,
                &slice.id,
                saved_state("rolled-back-state"),
                47,
                SliceOperationStatus::Failed,
                Some("backup restore failed; automatic rollback completed".to_string()),
                |_, _| Ok(()),
            )
            .expect("durable rollback resolution should clear quarantine");
        store
            .try_begin_operation(&slice.id, "slice.start")
            .expect("slice operation should resume after durable resolution");
    }

    #[test]
    fn slice_store_creates_resolves_and_exposes_display_endpoint() {
        let store = SliceStore::default();
        let slice = store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("slice should create");

        assert_eq!(slice.id, "slice-1");
        assert_eq!(slice.worker_kernel_ref, "slice:dev");
        assert_eq!(slice.last_operation.as_deref(), Some("create"));
        assert_eq!(
            slice.last_operation_status,
            Some(SliceOperationStatus::Completed)
        );
        assert_eq!(slice.last_error, None);
        assert_eq!(slice.last_operation_at_ms, Some(42));
        assert_eq!(
            store.resolve("dev").expect("slice should resolve").id,
            slice.id
        );
        assert_eq!(
            store
                .display_endpoint("dev")
                .expect("display endpoint should resolve")
                .capabilities,
            vec!["view", "keyboard", "mouse"]
        );
    }

    #[test]
    fn slice_store_keeps_headless_display_endpoint_hidden() {
        let store = SliceStore::default();
        let mut input = create_input("dev");
        input.display_mode = SliceDisplayMode::Headless;
        input.display_url = None;

        let slice = store
            .create("kernel-1", "machine-1", input)
            .expect("headless slice should create");

        assert_eq!(slice.display_mode, SliceDisplayMode::Headless);
        assert_eq!(slice.display_endpoint, None);
        assert!(store.display_endpoint("dev").is_err());
    }

    #[test]
    fn slice_store_rejects_names_that_collide_with_existing_ids() {
        let store = SliceStore::default();
        store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("first slice should create");

        assert!(store
            .create("kernel-1", "machine-1", create_input("slice-1"))
            .is_err());
    }

    #[test]
    fn slice_store_registers_inherited_saved_state_on_create() {
        let store = SliceStore::default();
        let mut input = create_input("dev");
        input.from_saved_state = Some(saved_state("state-default"));

        let slice = store
            .create("kernel-1", "machine-1", input)
            .expect("slice should create from saved state");

        assert_eq!(slice.saved_state_ref.as_deref(), Some("state-default"));
        assert_eq!(slice.saved_state_status, Some(SliceSavedStateStatus::Saved));
        assert_eq!(slice.saved_state_updated_at_ms, Some(2));
        assert_eq!(
            store
                .active_saved_state_for_slice("dev")
                .expect("saved state lookup should work")
                .expect("saved state should be registered")
                .id,
            "state-default"
        );
    }

    #[test]
    fn failed_resave_keeps_the_prior_saved_generation_valid() {
        let store = SliceStore::default();
        let mut input = create_input("dev");
        input.from_saved_state = Some(saved_state("state-known-good"));
        let slice = store
            .create("kernel-1", "machine-1", input)
            .expect("slice should create from saved state");
        let error = crate::error::DaemonError::LocalTransport {
            operation: "slice.state.save",
            message: "injected capture failure".to_string(),
        };

        let failed = store
            .mark_saved_state_failed(&slice.id, &error, 3)
            .expect("failed save should update diagnostics");

        assert_eq!(failed.saved_state_ref.as_deref(), Some("state-known-good"));
        assert_eq!(
            failed.saved_state_status,
            Some(SliceSavedStateStatus::Saved)
        );
        assert_eq!(failed.saved_state_updated_at_ms, Some(2));
        assert_eq!(
            failed.last_operation_status,
            Some(SliceOperationStatus::Failed)
        );
        assert_eq!(
            store
                .active_saved_state_for_slice(&slice.id)
                .expect("saved state lookup should work")
                .expect("prior state should remain active")
                .id,
            "state-known-good"
        );
    }

    #[test]
    fn slice_store_resolves_backup_by_id_or_unique_name_within_source_slice() {
        let store = SliceStore::default();
        let first = store
            .create("kernel-1", "machine-1", create_input("first"))
            .expect("first slice should create");
        let second = store
            .create("kernel-1", "machine-1", create_input("second"))
            .expect("second slice should create");
        let failed_backup = backup("failed-snapshot", "snapshot", &first.id);
        let error = store
            .upsert_backup_transactionally(failed_backup, |_| {
                Err(crate::error::DaemonError::LocalTransport {
                    operation: "slice.backup.create",
                    message: "injected durable-state failure".to_string(),
                })
            })
            .expect_err("failed persistence must not publish the backup record");
        assert!(error.to_string().contains("injected durable-state failure"));
        assert!(store.list_backups().is_empty());

        store
            .upsert_backup_transactionally(
                backup("first-snapshot-1", "snapshot", &first.id),
                |_| Ok(()),
            )
            .expect("first backup should persist");
        store
            .upsert_backup_transactionally(
                backup("second-snapshot-1", "snapshot", &second.id),
                |_| Ok(()),
            )
            .expect("second backup should persist");

        assert_eq!(
            store
                .resolve_backup_for_slice(&first.id, "first-snapshot-1")
                .expect("backup id should resolve")
                .id,
            "first-snapshot-1"
        );
        assert_eq!(
            store
                .resolve_backup_for_slice(&first.id, "snapshot")
                .expect("unique backup name should resolve")
                .id,
            "first-snapshot-1"
        );
        assert!(store
            .resolve_backup_for_slice(&first.id, "second-snapshot-1")
            .is_err());
        store
            .upsert_backup_transactionally(
                backup("first-snapshot-2", "snapshot", &first.id),
                |_| Ok(()),
            )
            .expect("duplicate-name backup should persist");
        let ambiguous = store
            .resolve_backup_for_slice(&first.id, "snapshot")
            .expect_err("duplicate names within one slice must require a backup id");
        assert!(ambiguous.to_string().contains("ambiguous"));
        assert_eq!(
            store
                .resolve_backup_for_slice(&first.id, "first-snapshot-2")
                .expect("an exact id must remain unambiguous")
                .id,
            "first-snapshot-2"
        );
    }

    #[test]
    fn slice_store_restores_records_and_continues_numbering() {
        let store = SliceStore::default();
        let slice = store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("slice should create");
        let slice = store
            .set_relay_endpoint(
                &slice.id,
                Some(local_docker_private_relay_endpoint(&slice)),
                43,
            )
            .expect("relay endpoint should update");

        let restored = SliceStore::default();
        restored.restore_records(vec![slice.clone()]);
        assert_eq!(
            restored
                .resolve_by_worker_kernel_ref("slice:dev")
                .expect("worker ref should resolve")
                .relay_endpoint,
            slice.relay_endpoint
        );

        let next = restored
            .create("kernel-1", "machine-1", create_input("next"))
            .expect("new slice should create after restore");
        assert_eq!(next.id, "slice-2");
    }

    #[test]
    fn slice_store_reconciles_runtime_state_after_kernel_restart() {
        let store = SliceStore::default();
        let slice = store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("slice should create");
        let slice = store
            .set_relay_endpoint(
                &slice.id,
                Some(local_docker_private_relay_endpoint(&slice)),
                43,
            )
            .expect("relay endpoint should update");
        store
            .set_worker_presence(
                &slice.id,
                Some("worker-1".to_string()),
                Some("machine-2".to_string()),
                vec!["codex".to_string()],
                44,
            )
            .expect("worker presence should update");
        store
            .set_status(&slice.id, SliceStatus::Running, 45)
            .expect("slice should be running");

        let reconciled = store.reconcile_after_kernel_restart(46);

        assert_eq!(reconciled.len(), 1);
        assert_eq!(reconciled[0].status, SliceStatus::Unhealthy);
        assert_eq!(reconciled[0].worker_kernel_id, None);
        assert_eq!(reconciled[0].worker_machine_id, None);
        assert_eq!(reconciled[0].relay_endpoint, None);
        assert!(reconciled[0].providers.is_empty());
        assert_eq!(reconciled[0].updated_at_ms, 46);
    }

    #[test]
    fn slice_store_reconciles_interrupted_stop_after_kernel_restart() {
        let store = SliceStore::default();
        let slice = store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("slice should create");
        store
            .set_status(&slice.id, SliceStatus::Stopping, 44)
            .expect("slice should be stopping");

        let reconciled = store.reconcile_after_kernel_restart(45);

        assert_eq!(reconciled.len(), 1);
        assert_eq!(reconciled[0].status, SliceStatus::Unhealthy);
        assert_eq!(
            reconciled[0].last_operation.as_deref(),
            Some("restart_reconcile")
        );
        assert_eq!(
            reconciled[0].last_operation_status,
            Some(SliceOperationStatus::Reconciled)
        );
        assert!(reconciled[0]
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("needs restart")));
        assert_eq!(reconciled[0].updated_at_ms, 45);
    }

    #[test]
    fn slice_store_reconciles_running_slice_missing_on_host_to_stopped() {
        let store = SliceStore::default();
        let slice = store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("slice should create");
        store
            .set_relay_endpoint(
                &slice.id,
                Some(local_docker_private_relay_endpoint(&slice)),
                43,
            )
            .expect("relay endpoint should update");
        store
            .set_worker_presence(
                &slice.id,
                Some("worker-1".to_string()),
                Some("machine-2".to_string()),
                vec!["codex".to_string()],
                44,
            )
            .expect("worker presence should update");
        store
            .set_status(&slice.id, SliceStatus::Running, 45)
            .expect("slice should be running");

        let reconciled = store
            .reconcile_after_kernel_restart_with_host_state(46, |_| SliceHostRuntimeState::Missing);

        assert_eq!(reconciled.len(), 1);
        assert_eq!(reconciled[0].status, SliceStatus::Stopped);
        assert_eq!(reconciled[0].worker_kernel_id, None);
        assert_eq!(reconciled[0].worker_machine_id, None);
        assert_eq!(reconciled[0].relay_endpoint, None);
        assert!(reconciled[0].providers.is_empty());
        assert_eq!(reconciled[0].updated_at_ms, 46);
    }

    #[test]
    fn slice_store_reconciles_stopped_record_with_running_host_to_running() {
        let store = SliceStore::default();
        let slice = store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("slice should create");

        let reconciled = store.reconcile_after_kernel_restart_with_host_state(46, |record| {
            assert_eq!(record.id, slice.id);
            SliceHostRuntimeState::Running
        });

        assert_eq!(reconciled.len(), 1);
        assert_eq!(reconciled[0].status, SliceStatus::Running);
        assert_eq!(reconciled[0].last_error, None);
        assert_eq!(reconciled[0].updated_at_ms, 46);
    }

    #[test]
    fn slice_store_recovers_unhealthy_record_when_host_is_running() {
        let store = SliceStore::default();
        let slice = store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("slice should create");
        let relay_endpoint = local_docker_private_relay_endpoint(&slice);
        store
            .set_worker_presence(
                &slice.id,
                Some("worker-1".to_string()),
                Some("machine-2".to_string()),
                vec!["codex".to_string()],
                44,
            )
            .expect("slice worker presence should update");
        store
            .set_relay_endpoint(&slice.id, Some(relay_endpoint.clone()), 44)
            .expect("slice relay endpoint should update");
        store
            .set_status(&slice.id, SliceStatus::Unhealthy, 45)
            .expect("slice should be unhealthy");

        let reconciled = store
            .reconcile_after_kernel_restart_with_host_state(46, |_| SliceHostRuntimeState::Running);

        assert_eq!(reconciled.len(), 1);
        assert_eq!(reconciled[0].status, SliceStatus::Running);
        assert_eq!(
            reconciled[0].last_operation.as_deref(),
            Some("restart_reconcile")
        );
        assert_eq!(
            reconciled[0].last_operation_status,
            Some(SliceOperationStatus::Reconciled)
        );
        assert_eq!(reconciled[0].last_error, None);
        assert_eq!(reconciled[0].worker_kernel_id.as_deref(), Some("worker-1"));
        assert_eq!(
            reconciled[0].worker_machine_id.as_deref(),
            Some("machine-2")
        );
        assert_eq!(reconciled[0].providers, vec!["codex"]);
        assert_eq!(reconciled[0].relay_endpoint, Some(relay_endpoint));
    }

    #[test]
    fn slice_store_rejects_overlapping_operations_until_guard_drops() {
        let store = SliceStore::default();
        store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("slice should create");

        let guard = store
            .try_begin_operation("dev", "slice.start")
            .expect("first operation should start");
        let error = store
            .try_begin_operation("dev", "slice.stop")
            .expect_err("second operation should be rejected");
        assert!(error.to_string().contains("slice.start"));

        drop(guard);
        store
            .try_begin_operation("dev", "slice.stop")
            .expect("operation should start after first guard drops");
    }

    #[test]
    fn local_docker_slice_port_check_reports_busy_ports() {
        let store = SliceStore::default();
        let slice = store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("slice should create");
        let ports = LocalDockerSlicePorts::for_record(&slice);
        let _listener = TcpListener::bind(("127.0.0.1", ports.relay)).ok();

        let error = ensure_local_docker_slice_ports_available(&slice)
            .expect_err("busy port should be reported before provisioning");

        assert!(
            error.to_string().contains(&ports.relay.to_string()),
            "error should name the busy port: {error}"
        );
    }

    #[test]
    fn slice_store_assigns_distinct_local_docker_ports_per_slice() {
        let store = SliceStore::default();
        let mut first_input = create_input("one");
        first_input.display_url = None;
        let first = store
            .create("kernel-1", "machine-1", first_input)
            .expect("first slice should create");
        let second = store
            .create("kernel-1", "machine-1", create_input("two"))
            .expect("second slice should create");

        let first_ports = first
            .local_docker_ports
            .expect("local Docker slices should persist assigned ports");
        let second_ports = second
            .local_docker_ports
            .expect("local Docker slices should persist assigned ports");
        assert_ne!(first_ports, second_ports);
        assert_ne!(first_ports.relay, second_ports.relay);
        assert!(first
            .display_endpoint
            .as_ref()
            .expect("headed slice should expose display")
            .url
            .contains(&first_ports.novnc.to_string()));
    }

    #[test]
    fn local_docker_slice_logs_include_tailed_action_logs() {
        let store = SliceStore::default();
        let slice = store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("slice should create");
        let root =
            std::env::temp_dir().join(format!("chariox-slice-logs-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let options = LocalDockerSliceOptions {
            root: root.clone(),
            home_public_key: crate::config::DaemonConfig::for_tests().relay_public_key,
            docker_image: "chariox-slice-linux:test".to_string(),
            build_image: SliceImageBuildPolicy::Never,
            extension_dockerfile: None,
            allow_unconfined_seccomp: false,
            memory_mb: None,
            cpus: None,
            screen_width: 1280,
            screen_height: 800,
            saved_home_archive: None,
        };
        let log_path =
            local_docker_slice_action_log_path(&root, &slice, LocalDockerSliceAction::Provision);
        fs::create_dir_all(log_path.parent().expect("log should have parent"))
            .expect("log dir should create");
        fs::write(&log_path, "line-1\nline-2\nline-3\n").expect("log should write");

        let entries = collect_local_docker_slice_logs(&slice, &options, Some(2))
            .expect("logs should collect");

        let provision = entries
            .iter()
            .find(|entry| entry.source == "provision")
            .expect("provision log should be present");
        assert_eq!(provision.text, "line-2\nline-3");
        assert!(provision.truncated);
        assert!(entries.iter().any(|entry| entry.source == "runtime"));
        assert!(entries.iter().any(|entry| entry.source == "container"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn slice_store_attaches_records_to_multiple_sessions_and_agents() {
        let store = SliceStore::default();
        let slice = store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("slice should create");

        let attached = store
            .attach_session(&slice.id, "session-1", 44)
            .expect("slice should attach");

        assert_eq!(attached.session_id.as_deref(), Some("session-1"));
        assert_eq!(attached.session_ids, vec!["session-1"]);
        assert_eq!(attached.updated_at_ms, 44);
        assert_eq!(store.list_by_session("session-1").len(), 1);
        let attached = store
            .attach_agent(&slice.id, "session-2", "agent-2", 45)
            .expect("slice should support another session in same worktree");
        assert_eq!(attached.session_id.as_deref(), Some("session-2"));
        assert_eq!(attached.session_ids, vec!["session-1", "session-2"]);
        assert_eq!(attached.agent_ids, vec!["agent-2"]);
        assert_eq!(store.list_by_session("session-2").len(), 1);
    }

    #[test]
    fn slice_store_batch_attaches_agents_with_one_record_update_per_slice() {
        let store = SliceStore::default();
        let slice = store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("slice should create");

        let attached = store
            .attach_agents(
                vec![
                    SliceAgentAttachment {
                        slice_ref: slice.id.clone(),
                        session_id: "session-1".to_string(),
                        agent_id: "agent-1".to_string(),
                    },
                    SliceAgentAttachment {
                        slice_ref: slice.name.clone(),
                        session_id: "session-1".to_string(),
                        agent_id: "agent-2".to_string(),
                    },
                ],
                46,
            )
            .expect("slice should batch attach agents");

        assert_eq!(
            attached.len(),
            1,
            "batch attach should return one updated record per touched slice"
        );
        assert_eq!(attached[0].id, slice.id);
        assert_eq!(attached[0].session_id.as_deref(), Some("session-1"));
        assert_eq!(attached[0].session_ids, vec!["session-1"]);
        assert_eq!(attached[0].agent_ids, vec!["agent-1", "agent-2"]);
        assert_eq!(attached[0].updated_at_ms, 46);
    }

    #[test]
    fn slice_store_keeps_provider_auth_summaries_per_slice() {
        let store = SliceStore::default();
        let first = store
            .create("kernel-1", "machine-1", create_input("first"))
            .expect("first slice should create");
        let second = store
            .create("kernel-1", "machine-1", create_input("second"))
            .expect("second slice should create");

        let first = store
            .set_provider_auth(
                &first.id,
                vec![SliceProviderAuthSummary {
                    provider: "codex".to_string(),
                    account_profile: "default".to_string(),
                    state: crate::slice_provider_auth::SliceProviderAuthState::Configured,
                    auth_type: Some("api-key".to_string()),
                    account_id: Some("acct-1".to_string()),
                    email: None,
                    organization_id: None,
                    organization_name: None,
                    subscription_type: None,
                    source: "test".to_string(),
                }],
                44,
            )
            .expect("first auth should update");
        let second = store
            .set_provider_auth(
                &second.id,
                vec![SliceProviderAuthSummary {
                    provider: "codex".to_string(),
                    account_profile: "default".to_string(),
                    state: crate::slice_provider_auth::SliceProviderAuthState::Configured,
                    auth_type: Some("api-key".to_string()),
                    account_id: Some("acct-2".to_string()),
                    email: None,
                    organization_id: None,
                    organization_name: None,
                    subscription_type: None,
                    source: "test".to_string(),
                }],
                45,
            )
            .expect("second auth should update");

        assert_eq!(first.provider_auth[0].account_id.as_deref(), Some("acct-1"));
        assert_eq!(
            second.provider_auth[0].account_id.as_deref(),
            Some("acct-2")
        );
    }

    #[test]
    fn local_docker_private_relay_uses_host_endpoint_without_container_override() {
        let store = SliceStore::default();
        let slice = store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("slice should create");

        let relay = local_docker_private_relay(&slice);
        let ports = LocalDockerSlicePorts::for_record(&slice);

        assert_eq!(relay.relay_url, format!("ws://127.0.0.1:{}", ports.relay));
        assert_eq!(relay.container_relay_url, None);
        assert_eq!(relay.relay_token, "slice-local-kernel-1-slice-1");
    }

    #[test]
    fn local_docker_relay_url_rewrites_host_loopback_for_container() {
        assert_eq!(
            relay_url_for_container("ws://127.0.0.1:43130"),
            "ws://host.docker.internal:43130"
        );
        assert_eq!(
            relay_url_for_container("ws://localhost:43130"),
            "ws://host.docker.internal:43130"
        );
        assert_eq!(
            relay_url_for_container("wss://relay.example/ws"),
            "wss://relay.example/ws"
        );
    }
}
