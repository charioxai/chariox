use super::worker_spy::WorkerSpy;
use super::*;
use crate::runtime::command::KernelCommand;
use crate::runtime::session_actor::{FocusedAgentProjection, SessionRuntime};
use std::sync::{atomic::Ordering, Arc};
use tokio::sync::{Mutex, Notify};
use tokio::time::timeout;

pub(super) fn runtime(state: &KernelRuntimeState, app: &crate::DaemonApp) -> SessionRuntime {
    SessionRuntime::with_queue_limit_and_focus_projection(
        state.clone(),
        4,
        FocusedAgentProjection::default(),
        state.owned.session_projection.clone(),
        state.owned.agent_runtime_projection.clone(),
        app.terminal_stream_store(),
    )
}

pub(super) fn external_command(request: &LocalDaemonRequest, grant: &str) -> KernelCommand {
    let mut command =
        KernelCommand::from_local_request("external-session-operation", None, None, request);
    command.caller.connection_class = Some(KernelConnectionClass::ExternalAgent);
    command.caller.caller_id = grant.into();
    command
}

#[tokio::test]
async fn kernel_access_main_requests_authorize_all_local_sessions_and_ordinary_global_operations() {
    let worktree = crate::test_support::TestWorktree::new("access-main-requests");
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .unwrap();
    let (allowed, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let (other, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        Arc::new(Mutex::new(app)),
        32,
    );
    let state = router.runtime_state();
    let grant = state.insert_access_grant_for_test(allowed.id());
    let delete = |id: &str| {
        LocalDaemonRequest::DeleteSession(crate::local::DeleteSessionRequest {
            session_ref: id.into(),
            workspace_id: None,
        })
    };
    assert!(state
        .authorize_external_request(&grant, &delete(allowed.id()))
        .is_ok());
    assert!(state
        .authorize_external_request(&grant, &delete(other.id()))
        .is_ok());
    assert!(state
        .authorize_external_request(
            &grant,
            &LocalDaemonRequest::GetDaemonHealth(crate::local::GetDaemonHealthRequest)
        )
        .is_ok());
    assert!(state
        .authorize_external_request(
            &grant,
            &LocalDaemonRequest::RevokeAppFileGrants(crate::local::RevokeAppFileGrantsRequest {
                installation_id: "owner-installation".into(),
                operation_id: None,
            })
        )
        .is_ok());
}

#[tokio::test]
async fn kernel_access_destroy_refuses_another_same_owner_session_agent() {
    let worktree = crate::test_support::TestWorktree::new("access-session-target");
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .unwrap();
    let (allowed, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let (other, victim) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let router =
        crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(app.clone(), 32);
    let state = router.runtime_state();
    let runtime = runtime(&state, &*app.lock().await);
    let grant = state.insert_access_grant_for_test(allowed.id());
    let before = state.session_snapshot(other.id()).await.unwrap();
    let request = LocalDaemonRequest::DestroyAgent(crate::local::DestroyAgentRequest {
        session_id: allowed.id().into(),
        agent_id: victim.id().into(),
    });
    let result = runtime
        .dispatch_session_command(external_command(&request, &grant), request)
        .await;
    assert!(result.is_err(), "a grant for A destroyed B's agent");
    assert_eq!(state.session_snapshot(other.id()).await.unwrap(), before);
    assert_eq!(
        state.owned.agent_store.get_agent(victim.id()).unwrap(),
        victim
    );
}

#[tokio::test]
async fn kernel_access_remote_destroy_rechecks_after_app_lock_wait() {
    revoked_remote_operation(false, false).await;
}

#[tokio::test]
async fn kernel_access_remote_spawn_rechecks_after_app_lock_wait() {
    revoked_remote_operation(true, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kernel_access_remote_destroy_rechecks_after_discovery_wait() {
    revoked_remote_operation(false, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kernel_access_remote_spawn_rechecks_after_discovery_wait() {
    revoked_remote_operation(true, true).await;
}

async fn revoked_remote_operation(spawn: bool, discovery_wait: bool) {
    let worktree = crate::test_support::TestWorktree::new("access-session-operation");
    let worker = WorkerSpy::new(discovery_wait);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some(worker.url.clone());
    config.relay_token = Some("session-operation-fixture".into());
    let mut daemon = crate::test_support::bootstrap_authenticated_app(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut daemon)
        .create_session(worktree.session_request())
        .unwrap();
    if !spawn {
        daemon
            .agents_mut()
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: worker.id.clone(),
                    worker_machine_id: "fixture-machine".into(),
                    execution_lease_id: "lease".into(),
                    leased_agent_id: "leased-agent".into(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .unwrap();
    }
    let app = Arc::new(Mutex::new(daemon));
    let router =
        crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(app.clone(), 32);
    let mut state = router.runtime_state();
    let probe = Arc::new(Notify::new());
    state.observe_app_lock_wait_for_test(probe.clone());
    let runtime = runtime(&state, &*app.lock().await);
    let before = state.session_snapshot(session.id()).await.unwrap();
    let grant = state.insert_access_grant_for_test(session.id());
    let request = if spawn {
        LocalDaemonRequest::SpawnAgent(crate::local::SpawnAgentRequest {
            session_id: session.id().into(),
            alias: Some("remote-created".into()),
            provider: Some("dev-stub".into()),
            account_profile: None,
            model: None,
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: Some(worktree.path().display().to_string()),
            kernel_ref: Some(worker.id.clone()),
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        })
    } else {
        LocalDaemonRequest::DestroyAgent(crate::local::DestroyAgentRequest {
            session_id: session.id().into(),
            agent_id: agent.id().into(),
        })
    };
    let guard = if discovery_wait {
        None
    } else {
        Some(app.lock().await)
    };
    let pending = tokio::spawn({
        let runtime = runtime.clone();
        let request = request.clone();
        let command = external_command(&request, &grant);
        async move { runtime.dispatch_session_command(command, request).await }
    });
    let reached = if discovery_wait {
        &worker.discovery_started
    } else {
        &probe
    };
    timeout(Duration::from_secs(3), reached.notified())
        .await
        .unwrap();
    state
        .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
        .unwrap();
    drop(guard);
    worker.release_discovery.notify_one();
    let result = timeout(Duration::from_secs(5), pending)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        worker.requests.load(Ordering::SeqCst),
        0,
        "revoked session operation reached worker"
    );
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("grant revoked or expired"));
    assert_eq!(state.session_snapshot(session.id()).await.unwrap(), before);
    let terminal =
        KernelCommand::from_local_request("terminal-session-control", None, None, &request);
    let response = runtime
        .dispatch_session_command(terminal, request)
        .await
        .unwrap();
    assert!(if spawn {
        matches!(response, LocalDaemonResponse::AgentSpawned { .. })
    } else {
        matches!(response, LocalDaemonResponse::AgentDestroyed { .. })
    });
    assert_eq!(
        worker.requests.load(Ordering::SeqCst),
        2,
        "terminal operation should reach worker"
    );
}

#[tokio::test]
async fn kernel_access_grants_allow_local_invites_and_scheduled_prompts() {
    let worktree = crate::test_support::TestWorktree::new("access-no-delegation");
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let state = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        Arc::new(Mutex::new(app)),
        32,
    )
    .runtime_state();
    let grant = state.insert_access_grant_for_test(session.id());
    for request in [
        LocalDaemonRequest::CreateSessionInvite(crate::local::CreateSessionInviteRequest {
            session_id: session.id().into(),
            expires_in_ms: None,
            max_uses: None,
            collaboration_level: Default::default(),
        }),
        LocalDaemonRequest::CreateAgentPromptSchedule(
            crate::local::CreateAgentPromptScheduleRequest {
                session_id: session.id().into(),
                agent_id: agent.id().into(),
                kind: crate::session::AgentPromptScheduleKind::Recurring,
                interval_seconds: 60,
                prompt: Some("run after grant expires".into()),
            },
        ),
    ] {
        assert!(state.authorize_external_request(&grant, &request).is_ok());
    }
    assert!(state
        .authorize_external_request(
            &grant,
            &LocalDaemonRequest::ResolveSession(crate::local::ResolveSessionRequest {
                session_ref: session.id().into(),
                workspace_id: None,
            })
        )
        .is_ok());
}

#[tokio::test]
async fn kernel_access_targeted_revocation_preserves_other_owners_pending_requests() {
    let app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .unwrap();
    let state = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        Arc::new(Mutex::new(app)),
        32,
    )
    .runtime_state();
    let pending = |owner: &str, id: &str| KernelAccessGrant {
        grant_id: id.into(),
        owner_user_id: owner.into(),
        holder_pid: 1,
        holder_executable: "fixture".into(),
        lifetime_minutes: 30,
        expires_at_ms: 0,
    };
    {
        let mut access = state.owned.kernel_access.lock().unwrap();
        access
            .pending
            .insert((1, 1, "a".into()), pending("owner-a", "grant-a"));
        access
            .pending
            .insert((2, 2, "b".into()), pending("owner-b", "grant-b"));
    }
    let generation = state.owned.kernel_access.lock().unwrap().generation;
    state
        .revoke_kernel_access(Some("owner-a"), None, "test")
        .unwrap();
    let access = state.owned.kernel_access.lock().unwrap();
    assert_eq!(access.generation, generation);
    assert_eq!(access.pending.len(), 1);
    assert_eq!(
        access.pending.values().next().unwrap().owner_user_id,
        "owner-b"
    );
    drop(access);
    state.revoke_kernel_access(None, None, "shutdown").unwrap();
    let access = state.owned.kernel_access.lock().unwrap();
    assert!(access.pending.is_empty());
    assert_ne!(access.generation, generation);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kernel_access_revocation_after_remote_destroy_still_settles_lease_and_home_state() {
    let worktree = crate::test_support::TestWorktree::new("access-destroy-settlement");
    let worker = WorkerSpy::new(false);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some(worker.url.clone());
    config.relay_token = Some("session-operation-fixture".into());
    let mut daemon = crate::test_support::bootstrap_authenticated_app(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut daemon)
        .create_session(worktree.session_request())
        .unwrap();
    daemon
        .agents_mut()
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: worker.id.clone(),
                worker_machine_id: "fixture-machine".into(),
                execution_lease_id: "lease".into(),
                leased_agent_id: "leased-agent".into(),
                active_worker_provider_run_id: None,
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .unwrap();
    let app = Arc::new(Mutex::new(daemon));
    let state =
        crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(app.clone(), 32)
            .runtime_state();
    let runtime = runtime(&state, &*app.lock().await);
    let grant = state.insert_access_grant_for_test(session.id());
    let request = LocalDaemonRequest::DestroyAgent(crate::local::DestroyAgentRequest {
        session_id: session.id().into(),
        agent_id: agent.id().into(),
    });
    worker.pause_destroy.store(true, Ordering::SeqCst);
    let pending = tokio::spawn({
        let command = external_command(&request, &grant);
        async move { runtime.dispatch_session_command(command, request).await }
    });
    timeout(Duration::from_secs(5), worker.destroy_committed.notified())
        .await
        .unwrap();
    state
        .revoke_kernel_access(Some(session.owner_user_id()), Some(&grant), "after-commit")
        .unwrap();
    worker.release_destroy.notify_one();
    assert!(pending.await.unwrap().is_ok());
    assert_eq!(
        worker.requests.load(Ordering::SeqCst),
        2,
        "lease cleanup must still run"
    );
    assert!(
        state.owned.agent_store.get_agent(agent.id()).is_err(),
        "home must settle the committed destruction"
    );
}
