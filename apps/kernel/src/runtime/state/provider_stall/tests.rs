//! A stalled workflow step is recovered inside its run.
use super::actions::RecoveryAction;
use super::*;
use crate::runtime::state::provider_launch_owned_state::owned_runtime_state;
use std::sync::Arc;
use tokio::sync::Mutex;

#[test]
fn a_workflow_step_is_not_offered_a_fresh_agent() {
    let ordinary = RecoveryAction::choices(false)
        .iter()
        .map(|choice| choice.id().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(ordinary, ["resume", "cancel", "start_fresh"]);
    let step = RecoveryAction::choices(true)
        .iter()
        .map(|choice| choice.id().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(step, ["resume", "cancel"]);
}

struct Step {
    runtime: KernelRuntimeState,
    room: String,
    agent: String,
    run: String,
    node: String,
    _worktree: crate::test_support::TestWorktree,
}

/// A workflow run whose only step is running as the agent's active turn.
async fn running_step(label: &str) -> Step {
    let worktree = crate::test_support::TestWorktree::new(label);
    let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap");
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session");
    let agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub").with_alias("stalled"),
        )
        .expect("agent");
    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), Some("stall-recovery".to_string()))
        .unwrap();
    let node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), agent.id())
        .unwrap();
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .unwrap();
    let queued = app
        .sessions_mut()
        .enqueue_workflow_prompt_by_agent(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("do the step".into()),
            None,
            crate::session::WorkflowQueuedPromptSource::Manual,
            None,
            None,
            Some(agent.id()),
        )
        .unwrap();
    app.sessions_mut()
        .remove_queued_workflow_prompt(session.id(), queued.id())
        .unwrap();
    let run = app
        .sessions_mut()
        .invoke_queued_workflow_endpoint(session.id(), &queued)
        .unwrap();
    let node_run = run.node_runs()[0].id().to_string();
    app.sessions_mut()
        .prepare_workflow_turn(
            session.id(),
            run.id(),
            &node_run,
            format!("workflow-ack:{node_run}"),
            "do the step".to_string(),
            None,
            None,
        )
        .unwrap();
    let (room, agent_id, run_id) = (
        session.id().to_string(),
        agent.id().to_string(),
        run.id().to_string(),
    );
    // The step runs as the agent's active turn on a real stub run, as an admitted turn does.
    let provider_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                &room,
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(&agent_id),
        )
        .expect("the stub run launches");
    app.update_provider_run_projection(provider_run);
    let prompt = crate::session::PromptQueueItem::new(
        format!("workflow-step:{run_id}:{node_run}"),
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(&run_id),
        &agent_id,
        "do the step",
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(&run_id, &node_run);
    let crate::session::PromptSubmissionOutcome::Started { .. } = app
        .prompt_owner_submit_prepared_prompt(&room, prompt, false)
        .expect("the step starts")
    else {
        panic!("the step starts immediately");
    };
    let runtime = owned_runtime_state(&Arc::new(Mutex::new(app))).await;
    Step {
        runtime,
        room,
        agent: agent_id,
        run: run_id,
        node: node_run,
        _worktree: worktree,
    }
}

impl Step {
    /// The agent's prompts: the running one first, then the queued ones.
    fn prompts(&self) -> Vec<crate::session::PromptQueueItem> {
        let session = self
            .runtime
            .owned
            .session_store
            .get_session(&self.room)
            .unwrap();
        let (active, queued) = self
            .runtime
            .owned
            .prompt_state_owner
            .state_parts(&session, &self.agent);
        active.into_iter().chain(queued).collect()
    }

    fn notice(&self, prompt: &crate::session::PromptQueueItem) -> StallNotice {
        StallNotice {
            room: self.room.clone(),
            agent: self.agent.clone(),
            prompt: prompt.id().to_owned(),
            run: None,
            quiet_since_ms: 1,
        }
    }
}

#[tokio::test]
async fn resume_sends_a_stalled_workflow_step_again_inside_its_run() {
    let step = running_step("stall-workflow-resume").await;
    let original = step.prompts().remove(0);
    assert_eq!(original.workflow_run_id(), Some(step.run.as_str()));
    let agent = step
        .runtime
        .owned
        .agent_store
        .get_agent(&step.agent)
        .unwrap();

    step.runtime
        .recover_stalled_turn(
            &step.notice(&original),
            &agent,
            &original,
            RecoveryAction::Resume,
        )
        .await
        .expect("a stalled workflow step resumes");

    let after = step.prompts();
    assert!(!after.is_empty(), "the step runs again");
    assert!(
        after.iter().all(|prompt| prompt.id() != original.id()
            && prompt.workflow_run_id() == Some(step.run.as_str())
            && prompt.workflow_node_run_id() == Some(step.node.as_str())),
        "the retry is the same step of the same run, not an unbound prompt: {after:?}"
    );
}

#[tokio::test]
async fn start_fresh_is_refused_for_a_workflow_step_and_stops_nothing() {
    let step = running_step("stall-workflow-fresh").await;
    let original = step.prompts().remove(0);
    let agent = step
        .runtime
        .owned
        .agent_store
        .get_agent(&step.agent)
        .unwrap();
    let agents = step
        .runtime
        .owned
        .agent_store
        .get_session_agents(&step.room)
        .len();

    let error = step
        .runtime
        .recover_stalled_turn(
            &step.notice(&original),
            &agent,
            &original,
            RecoveryAction::StartFresh,
        )
        .await
        .expect_err("a workflow step has no fresh-agent recovery");

    assert!(error.to_string().contains("Resume or Cancel"), "{error}");
    assert_eq!(
        step.prompts().first().map(|p| p.id().to_owned()),
        Some(original.id().to_owned())
    );
    assert_eq!(
        step.runtime
            .owned
            .agent_store
            .get_session_agents(&step.room)
            .len(),
        agents
    );
}
