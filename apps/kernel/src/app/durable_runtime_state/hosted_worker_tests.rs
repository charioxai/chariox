use super::*;
use crate::slice::hosted_worker_test_support::*;

fn add_slice(app: &DaemonApp, name: &str, worker: &str, ssh: bool) -> crate::slice::SliceRecord {
    app.slices()
        .create(
            &app.config().daemon_id,
            &app.config().host_machine_id,
            crate::slice::CreateSliceInput {
                name: name.into(),
                backend: if ssh {
                    crate::slice::SliceBackendKind::SshDocker
                } else {
                    crate::slice::SliceBackendKind::LocalDocker
                },
                os: "linux".into(),
                display_mode: crate::slice::SliceDisplayMode::Headless,
                display_backend: Default::default(),
                workspace_id: None,
                worktree_id: None,
                workspace_mount: None,
                development: None,
                worker_kernel_ref: Some(worker.into()),
                display_url: None,
                provider_auth: Vec::new(),
                from_saved_state: None,
                now_ms: 1,
            },
        )
        .unwrap()
}

fn bind(app: &mut DaemonApp, kernel: &str, machine: &str) -> (String, String) {
    let (session, agent) = crate::app::KernelSessionService::new(app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace",
            "worktree",
        ))
        .unwrap();
    app.agents()
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: kernel.into(),
                worker_machine_id: machine.into(),
                execution_lease_id: "lease".into(),
                leased_agent_id: "leased".into(),
                active_worker_provider_run_id: None,
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .unwrap();
    (session.id().into(), agent.id().into())
}

fn app() -> DaemonApp {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.daemon_id = "kernel-a".into();
    config.host_machine_id = MACHINE.into();
    DaemonApp::bootstrap(config).unwrap()
}

#[test]
fn hosted_slice_context_restore_repairs_exact_worker_under_shared_parent_machine() {
    let _guard = crate::env_lock::lock();
    let mut app = app();
    let a = add_slice(&app, "a", &canonical_worker(0), false);
    let b = add_slice(&app, "b", &canonical_worker(3), false);
    let (session, agent) = bind(&mut app, &a.worker_kernel_ref, MACHINE);
    app.reconcile_restored_slice_agent_attachments().unwrap();
    let attached = app.slices().resolve(&a.id).unwrap();
    assert_eq!(attached.session_ids, vec![session]);
    assert_eq!(attached.agent_ids, vec![agent]);
    assert!(app.slices().resolve(&b.id).unwrap().agent_ids.is_empty());
}

#[test]
fn hosted_slice_context_restore_preserves_exact_custom_ssh_worker_identity() {
    let _guard = crate::env_lock::lock();
    let mut app = app();
    let mut slice = add_slice(&app, "ssh", "custom-worker", true);
    slice.worker_kernel_id = Some("observed-ssh-worker".into());
    slice.worker_machine_id = Some("ssh-machine".into());
    app.slices().restore_records(vec![slice.clone()]);
    let (_, agent) = bind(&mut app, "observed-ssh-worker", "ssh-machine");
    app.reconcile_restored_slice_agent_attachments().unwrap();
    assert_eq!(
        app.slices().resolve(&slice.id).unwrap().agent_ids,
        vec![agent]
    );
}

#[test]
fn hosted_slice_context_restore_keeps_legacy_synthetic_worker() {
    let _guard = crate::env_lock::lock();
    let mut app = app();
    let slice = add_slice(&app, "private", "private-worker", true);
    let (_, agent) = bind(&mut app, "private-worker", &format!("slice:{}", slice.id));
    app.reconcile_restored_slice_agent_attachments().unwrap();
    assert_eq!(
        app.slices().resolve(&slice.id).unwrap().agent_ids,
        vec![agent]
    );
}

#[test]
fn hosted_slice_context_restore_rejects_shared_worker_ambiguity() {
    let _guard = crate::env_lock::lock();
    let mut app = app();
    let a = add_slice(&app, "a", &canonical_worker(0), false);
    let b = add_slice(&app, "b", &canonical_worker(0), false);
    bind(&mut app, &a.worker_kernel_ref, MACHINE);
    app.reconcile_restored_slice_agent_attachments().unwrap();
    for slice in [a, b] {
        assert!(app
            .slices()
            .resolve(&slice.id)
            .unwrap()
            .agent_ids
            .is_empty());
    }
}

#[test]
fn hosted_slice_context_restore_rejects_canonical_observed_disagreement_and_wrong_parent() {
    let _guard = crate::env_lock::lock();
    for (observed, requested, machine) in [
        (Some(canonical_worker(3)), canonical_worker(3), MACHINE),
        (None, canonical_worker(0), "foreign-machine"),
    ] {
        let mut app = app();
        let mut slice = add_slice(&app, "a", &canonical_worker(0), false);
        slice.worker_kernel_id = observed;
        app.slices().restore_records(vec![slice.clone()]);
        bind(&mut app, &requested, machine);
        app.reconcile_restored_slice_agent_attachments().unwrap();
        assert!(app
            .slices()
            .resolve(&slice.id)
            .unwrap()
            .agent_ids
            .is_empty());
    }
}

#[test]
fn hosted_slice_context_restore_rejects_persisted_namespace_bound_to_foreign_parent() {
    let _guard = crate::env_lock::lock();
    let mut app = app();
    let slice = add_slice(&app, "foreign", &canonical_worker(1), false);
    bind(&mut app, &slice.worker_kernel_ref, MACHINE);
    app.reconcile_restored_slice_agent_attachments().unwrap();
    assert!(app
        .slices()
        .resolve(&slice.id)
        .unwrap()
        .agent_ids
        .is_empty());
}

#[test]
fn hosted_slice_context_restore_does_not_use_synthetic_machine_over_conflicting_kernel() {
    let _guard = crate::env_lock::lock();
    let mut app = app();
    let mut slice = add_slice(&app, "conflict", &canonical_worker(0), false);
    slice.worker_kernel_id = Some(canonical_worker(3));
    app.slices().restore_records(vec![slice.clone()]);
    bind(
        &mut app,
        &canonical_worker(3),
        &format!("slice:{}", slice.id),
    );
    app.reconcile_restored_slice_agent_attachments().unwrap();
    assert!(app
        .slices()
        .resolve(&slice.id)
        .unwrap()
        .agent_ids
        .is_empty());
}
