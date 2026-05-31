mod local_docker;
mod model;
mod ports;
mod store;

pub use local_docker::{
    collect_local_docker_slice_logs, inspect_local_docker_slice_host_runtime,
    local_docker_private_relay, local_docker_private_relay_endpoint,
    local_docker_private_relay_token, run_local_docker_slice_action,
    start_local_docker_slice_provider_login, LocalDockerSliceOptions, LocalDockerSliceRelay,
};
#[cfg(test)]
use local_docker::{
    ensure_local_docker_slice_ports_available, local_docker_slice_action_log_path,
    relay_url_for_container,
};
pub use model::{
    CreateSliceInput, LocalDockerSliceAction, SliceBackendKind, SliceDisplayEndpoint,
    SliceDisplayEndpointAccess, SliceDisplayEndpointKind, SliceDisplayMode, SliceLocalDockerPorts,
    SliceLogEntry, SliceOperationStatus, SliceProviderLoginStart, SliceRecord, SliceRelayEndpoint,
    SliceStatus,
};
#[cfg(test)]
use ports::LocalDockerSlicePorts;
pub use store::{SliceHostRuntimeState, SliceOperationGuard, SliceStore};

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
            workspace_id: None,
            worktree_id: None,
            workspace_mount: Some("/repo".to_string()),
            worker_kernel_ref: None,
            display_url: Some("http://127.0.0.1:6080".to_string()),
            provider_auth: Vec::new(),
            now_ms: 42,
        }
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
    fn slice_store_reconciles_stopped_record_with_running_host_to_unhealthy() {
        let store = SliceStore::default();
        let slice = store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("slice should create");

        let reconciled = store.reconcile_after_kernel_restart_with_host_state(46, |record| {
            assert_eq!(record.id, slice.id);
            SliceHostRuntimeState::Running
        });

        assert_eq!(reconciled.len(), 1);
        assert_eq!(reconciled[0].status, SliceStatus::Unhealthy);
        assert_eq!(reconciled[0].updated_at_ms, 46);
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
            std::env::temp_dir().join(format!("arroba-slice-logs-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let options = LocalDockerSliceOptions {
            root: root.clone(),
            docker_image: "arroba-slice-linux:test".to_string(),
            build_image: SliceImageBuildPolicy::Never,
            extension_dockerfile: None,
            allow_unconfined_seccomp: false,
            memory_mb: None,
            cpus: None,
            screen_width: 1280,
            screen_height: 800,
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
    fn slice_store_sets_and_clears_provider_auth_aliases() {
        let store = SliceStore::default();
        let mut input = create_input("dev");
        input.provider_auth = vec![SliceProviderAuthSummary {
            provider: "codex".to_string(),
            state: crate::slice_provider_auth::SliceProviderAuthState::Configured,
            auth_type: Some("chatgpt".to_string()),
            account_id: Some("acct-1".to_string()),
            email: None,
            organization_id: None,
            organization_name: None,
            subscription_type: None,
            alias: None,
            source: "test".to_string(),
        }];
        let slice = store
            .create("kernel-1", "machine-1", input)
            .expect("slice should create");

        let aliased = store
            .set_provider_auth_alias(&slice.id, "codex", Some("Work"), 44)
            .expect("alias should update");
        assert_eq!(aliased.provider_auth[0].alias.as_deref(), Some("Work"));
        assert_eq!(aliased.updated_at_ms, 44);

        let cleared = store
            .set_provider_auth_alias(&slice.id, "codex", Some("  "), 45)
            .expect("empty alias should clear");
        assert_eq!(cleared.provider_auth[0].alias, None);
        assert!(store
            .set_provider_auth_alias(&slice.id, "claude", Some("Personal"), 46)
            .is_err());
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
                    state: crate::slice_provider_auth::SliceProviderAuthState::Configured,
                    auth_type: Some("api-key".to_string()),
                    account_id: Some("acct-1".to_string()),
                    email: None,
                    organization_id: None,
                    organization_name: None,
                    subscription_type: None,
                    alias: Some("work".to_string()),
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
                    state: crate::slice_provider_auth::SliceProviderAuthState::Configured,
                    auth_type: Some("api-key".to_string()),
                    account_id: Some("acct-2".to_string()),
                    email: None,
                    organization_id: None,
                    organization_name: None,
                    subscription_type: None,
                    alias: Some("personal".to_string()),
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
        assert_eq!(
            store
                .resolve(&first.id)
                .expect("first should resolve")
                .provider_auth[0]
                .alias
                .as_deref(),
            Some("work")
        );
        assert_eq!(
            store
                .resolve(&second.id)
                .expect("second should resolve")
                .provider_auth[0]
                .alias
                .as_deref(),
            Some("personal")
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
