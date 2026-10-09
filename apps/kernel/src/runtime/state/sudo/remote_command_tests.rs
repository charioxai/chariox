//! MP-08/MP-10/MP-11: post-admission waits cannot borrow a later sudo turn.
use super::queued_agent_command_tests::replace_turn;
use super::tests::{fixture_with_options, popup};
use super::window_tests::{approve, open};
use super::*;
use crate::runtime::agent_actor::AgentRuntime;
use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};
use crate::runtime::session_actor::FocusedAgentProjection;
use crate::runtime::state::kernel_access::test_support::worker_spy::WorkerSpy;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn remote_sudo_completion_rejects_replaced_turn_after_executor_admission() {
    assert_remote_wait(true).await;
}

#[tokio::test]
async fn remote_sudo_completion_accepts_its_original_turn_after_executor_admission() {
    assert_remote_wait(false).await;
}

async fn assert_remote_wait(replace: bool) {
    let mut f = fixture_with_options(None, true);
    let worker = WorkerSpy::new(false);
    let worktree = crate::test_support::TestWorktree::new("sudo-remote-wait");
    let window = open(&f, None).await;
    let (session, target) = {
        let mut app = f.app.lock().await;
        let (session, target) = crate::app::KernelSessionService::new(&mut app)
            .create_session(worktree.session_request())
            .unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "remote-sudo-holder",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        app.agents_mut()
            .bind_remote_execution(
                target.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: worker.id.clone(),
                    worker_machine_id: "fixture-machine".into(),
                    execution_lease_id: "lease".into(),
                    leased_agent_id: "leased-agent".into(),
                    active_worker_provider_run_id: Some("worker-run".into()),
                    relay_url: Some(worker.url.clone()),
                    relay_token: Some("sudo-remote-fixture".into()),
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .unwrap();
        let prompt = PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            target.id(),
            "active",
            PromptStatus::Queued,
        );
        app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
            .unwrap();
        (session, target)
    };
    let request = LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
        session_id: session.id().into(),
    });
    let confirmation = tokio::spawn({
        let state = f.state.clone();
        let window = window.clone();
        let request = request.clone();
        async move { state.confirm_sudo_scope(&window, &request).await }
    });
    approve(&f, &popup(&f.state).await, None).await.unwrap();
    confirmation.await.unwrap().unwrap();
    let probe = Arc::new(tokio::sync::Notify::new());
    f.state.observe_app_lock_wait_for_test(probe.clone());
    let runtime = AgentRuntime::new(
        f.state.clone(),
        Default::default(),
        FocusedAgentProjection::default(),
        f.state.owned.session_projection.clone(),
        f.state.owned.agent_runtime_projection.clone(),
        f.state.owned.prompt_state_owner.clone(),
        Default::default(),
    );
    let mut command = KernelCommand::from_local_request(
        "sudo-remote-wait",
        window.prompt_id.clone(),
        Some(window.entry_id.clone()),
        &request,
    );
    command.caller = KernelCaller::for_source(&KernelCommandSource::LocalIpc)
        .with_connection_class(KernelConnectionClass::KernelAgent);
    command.caller.caller_id = window.entry_id.clone();
    command.caller.user_id = Some(window.owner_user_id.clone());
    command.caller.metaagent_id = Some(window.agent_id.clone());
    command.provider_run_id = window.provider_run_id.clone();
    let lock = f.app.lock().await;
    let pending = tokio::spawn({
        let request = request.clone();
        async move {
            let LocalDaemonRequest::CompletePrompt(r) = request else {
                unreachable!()
            };
            runtime.dispatch_prompt_complete(&command, r).await
        }
    });
    tokio::time::timeout(Duration::from_secs(3), probe.notified())
        .await
        .unwrap();
    let before = f.state.owned.session_snapshot(session.id()).unwrap();
    if replace {
        replace_turn(&f, &window);
    }
    drop(lock);
    let result = tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        worker.requests.load(Ordering::SeqCst),
        usize::from(!replace),
        "an ended submitting turn must send no worker request: {result:?}"
    );
    if replace {
        assert!(
            result
                .as_ref()
                .is_err_and(|e| e.to_string().contains("original provider turn ended")),
            "{result:?}"
        );
        let after = f.state.owned.session_snapshot(session.id()).unwrap();
        assert_eq!(
            before.active_prompt_for_agent(target.id()),
            after.active_prompt_for_agent(target.id())
        );
        assert_eq!(
            before.queued_prompts_for_agent(target.id()),
            after.queued_prompts_for_agent(target.id())
        );
    } else {
        result.unwrap();
    }
    f.state
        .revoke_sudo(Some("local"), Some(&window.entry_id), "fixture_cleanup")
        .unwrap();
}
