use super::*;
use crate::local::AgentSubstituteAction;
use crate::provider::{LaunchProviderRequest, ProviderRunState};

async fn configured_runtime() -> (
    Arc<Mutex<DaemonApp>>,
    KernelRuntimeState,
    String,
    String,
    crate::test_support::TestWorktree,
) {
    let worktree = crate::test_support::TestWorktree::new("workspace-substitute-launch");
    let (app, runtime, session, agent) = agent_config_runtime_in_worktree(
        crate::config::DaemonConfig::for_tests(),
        crate::session::DEFAULT_LOCAL_USER_ID,
        worktree.session_request(),
    )
    .await;
    runtime
        .update_agent_profile(
            &session,
            &agent,
            crate::session::DEFAULT_LOCAL_USER_ID,
            Some("dev-stub".into()),
            Some("starter-account".into()),
            Some("starter-model".into()),
            Some(Some("high".into())),
        )
        .await
        .unwrap();
    runtime
        .update_agent_substitutes(
            &session,
            &agent,
            crate::session::DEFAULT_LOCAL_USER_ID,
            AgentSubstituteAction::Add {
                provider: "dev-stub".into(),
                model: "substitute-model".into(),
                variant: Some("low".into()),
                account_profile: Some("substitute-account".into()),
                kernel_id: None,
                worktree_id: None,
            },
        )
        .await
        .unwrap();
    (app, runtime, session, agent, worktree)
}

fn start_stub(runtime: &KernelRuntimeState, session: &str, agent: &str) -> String {
    let config = runtime.owned.agent_store.get_agent(agent).unwrap();
    let run = runtime
        .owned
        .provider_store
        .launch_run_detached(
            LaunchProviderRequest::new(
                session,
                "dev-stub",
                config.provider(),
                config.provider_account_profile(),
                config.model().unwrap(),
            )
            .with_agent_id(agent)
            .with_variant(config.effort().map(str::to_string)),
        )
        .unwrap();
    runtime
        .owned
        .session_store
        .set_active_provider_run(session, Some(run.id().into()))
        .unwrap();
    run.id().into()
}

#[tokio::test]
async fn substitute_list_edits_do_not_interrupt_the_running_profile() {
    let (app, runtime, session, agent, _worktree) = configured_runtime().await;
    let old = start_stub(&runtime, &session, &agent);
    sync_active_prompt(&app, &session, &agent).await;
    for action in [
        AgentSubstituteAction::Move {
            from_index: 0,
            to_index: 0,
        },
        AgentSubstituteAction::SetTimeout {
            timeout_ms: Some(30_000),
        },
        AgentSubstituteAction::Remove { index: 0 },
        AgentSubstituteAction::Clear {},
    ] {
        runtime
            .update_agent_substitutes(
                &session,
                &agent,
                crate::session::DEFAULT_LOCAL_USER_ID,
                action,
            )
            .await
            .unwrap();
        assert_eq!(
            runtime.owned.provider_store.get_run(&old).unwrap().state(),
            ProviderRunState::Running
        );
    }
}

#[tokio::test]
async fn workflow_rotation_does_not_copy_another_providers_adapter() {
    for fresh in [false, true] {
        let (_app, runtime, session, agent, _worktree) = configured_runtime().await;
        let old = start_stub(&runtime, &session, &agent);
        runtime
            .owned
            .provider_store
            .enable_workflow_tools(&old)
            .unwrap();
        // Reproduce persisted selection changing while an earlier provider run is still present.
        // Both adapters are deterministic test adapters; no provider process or credentials are used.
        runtime
            .owned
            .agent_store
            .set_agent_runtime_profile_with_account_profile(
                &agent,
                "managed-dev-stub",
                Some("next-model".into()),
                Some("low".into()),
                Some("next-account".into()),
                crate::provider::ProviderResumeState::default(),
            )
            .unwrap();
        let (next, _) = runtime
            .owned
            .workflow_ensure_provider_run(&session, &agent, fresh, None)
            .unwrap();
        let run = runtime.owned.provider_store.get_run(&next).unwrap();
        assert_eq!(
            run.adapter_key(),
            "managed-dev-stub",
            "adapter must follow the current provider, fresh={fresh}"
        );
        assert_eq!(run.provider(), "managed-dev-stub");
        assert_eq!(run.model(), "next-model");
        assert_eq!(run.account_profile(), "next-account");
        assert_eq!(
            runtime.owned.provider_store.get_run(&old).unwrap().state(),
            ProviderRunState::Ended
        );
    }
}
