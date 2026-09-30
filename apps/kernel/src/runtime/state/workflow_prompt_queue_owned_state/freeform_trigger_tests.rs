//! A Freeform trigger is background work: its runs use a runtime copy of the
//! agent, never the owner's own agent and conversation, and never move focus.
use super::*;
use crate::local::CreateAgentWorkflowRequest;
use crate::session::{WorkflowOriginReason, WorkflowOriginSurface};

struct TriggerFixture {
    runtime: KernelRuntimeState,
    session_id: String,
    focus_agent_id: String,
    owner_user_id: String,
    workflow_id: String,
    endpoint_id: String,
    _test_root: TestRoot,
}

/// The owner is talking to their focus agent and gave it a trigger from Freeform.
fn trigger_on_the_focus_agent() -> TriggerFixture {
    let test_root = TestRoot(std::env::temp_dir().join(format!(
        "chariox-freeform-trigger-{}-{:032x}",
        std::process::id(),
        rand::random::<u128>()
    )));
    std::fs::create_dir_all(&test_root.0).expect("test root should be created");
    let root = test_root.0.to_string_lossy().to_string();
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(&root, &root))
        .expect("session should be created");
    let agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("todo-agent"),
        )
        .expect("agent should be created");
    crate::app::KernelSessionService::new(&mut app)
        .focus_agent(session.id(), agent.id())
        .expect("the owner focuses the agent");
    let runtime = runtime_state_from_app(app);
    let created = runtime
        .owned
        .workflow_create_from_agent(
            CreateAgentWorkflowRequest {
                session_id: session.id().to_string(),
                agent_id: agent.id().to_string(),
                reason: WorkflowOriginReason::Trigger,
                surface: WorkflowOriginSurface::Web,
                alias: None,
            },
            agent.owner_user_id(),
        )
        .expect("the trigger workflow should be created");
    let LocalDaemonResponse::AgentWorkflowCreated {
        workflow, endpoint, ..
    } = created
    else {
        panic!("unexpected response: {created:?}");
    };
    assert_eq!(workflow.nodes()[0].agent_id(), agent.id());
    TriggerFixture {
        runtime,
        session_id: session.id().to_string(),
        focus_agent_id: agent.id().to_string(),
        owner_user_id: agent.owner_user_id().to_string(),
        workflow_id: workflow.id().to_string(),
        endpoint_id: endpoint.id().to_string(),
        _test_root: test_root,
    }
}

fn fire(fixture: &TriggerFixture) -> crate::session::WorkflowRun {
    let (outcome, _) = fixture
        .runtime
        .owned
        .workflow_enqueue_prompt_and_maybe_start(
            &fixture.session_id,
            &fixture.workflow_id,
            &fixture.endpoint_id,
            Some("The Todo is due now.".to_string()),
            None,
            None,
        )
        .expect("the trigger should start a run");
    match outcome {
        crate::app::workflow_runtime::WorkflowLaunchOutcome::Started { workflow_run, .. } => {
            *workflow_run
        }
        crate::app::workflow_runtime::WorkflowLaunchOutcome::Enqueued { .. } => {
            panic!("an idle trigger should start its run")
        }
    }
}

/// The run went to a hidden runtime copy; the owner's focus agent has no
/// workflow turn and is still the session's focus.
fn assert_ran_beside_the_owner(
    fixture: &TriggerFixture,
    workflow_run: &crate::session::WorkflowRun,
) {
    let run_agent_id = workflow_run.node_runs()[0].agent_id();
    assert_ne!(
        run_agent_id, fixture.focus_agent_id,
        "a Freeform trigger must not run in the owner's own conversation"
    );
    let copy = fixture
        .runtime
        .owned
        .agent_store
        .get_agent(run_agent_id)
        .expect("the run's agent should be a runtime copy");
    assert!(!copy.visible_in_freeform());
    assert_eq!(copy.provider(), "dev-stub");

    let session = fixture
        .runtime
        .owned
        .session_snapshot(&fixture.session_id)
        .expect("session should resolve");
    assert_eq!(
        session.focused_agent_id(),
        Some(fixture.focus_agent_id.as_str()),
        "a workflow run must not move the session focus"
    );
    let (active, queued) = fixture
        .runtime
        .owned
        .prompt_state_owner
        .state_parts(&session, &fixture.focus_agent_id);
    assert!(active.is_none() && queued.is_empty());

    // The owner's own view of the session has no Freeform pane for the copy.
    let owner_view = session.redacted_for_user(&fixture.owner_user_id);
    assert!(owner_view
        .agents()
        .iter()
        .filter(|agent| agent.id() == run_agent_id)
        .all(|agent| !agent.visible_in_freeform()));
    assert!(owner_view
        .agents()
        .iter()
        .any(|agent| agent.id() == fixture.focus_agent_id && agent.visible_in_freeform()));
}

#[test]
fn freeform_trigger_runs_on_a_runtime_copy_beside_the_focus_agent() {
    let fixture = trigger_on_the_focus_agent();
    let workflow_run = fire(&fixture);
    assert_ran_beside_the_owner(&fixture, &workflow_run);
}

#[test]
fn a_primary_lane_left_on_a_freeform_trigger_retires_before_the_next_run() {
    let fixture = trigger_on_the_focus_agent();
    // A lane registered before triggers became copy-only: it would run on
    // the owner's own agent.
    let revision = fixture
        .runtime
        .owned
        .session_store
        .read()
        .resolve_workflow_ref(&fixture.session_id, &fixture.workflow_id)
        .expect("workflow should resolve")
        .revision();
    let node_id = fixture
        .runtime
        .owned
        .session_store
        .read()
        .resolve_workflow_ref(&fixture.session_id, &fixture.workflow_id)
        .expect("workflow should resolve")
        .nodes()[0]
        .id()
        .to_string();
    let primary = crate::session::WorkflowEndpointRuntimeInstance::new(
        "workflow-instance-legacy-primary",
        fixture.workflow_id.clone(),
        fixture.endpoint_id.clone(),
        revision,
        1,
        true,
        BTreeMap::from([(node_id, fixture.focus_agent_id.clone())]),
        fixture._test_root.0.to_string_lossy().to_string(),
    );
    fixture
        .runtime
        .owned
        .session_store
        .write()
        .register_workflow_runtime_instance(&fixture.session_id, primary)
        .expect("legacy primary lane should register");

    let workflow_run = fire(&fixture);

    assert_ran_beside_the_owner(&fixture, &workflow_run);
    assert!(fixture
        .runtime
        .owned
        .session_store
        .get_session(&fixture.session_id)
        .expect("session should resolve")
        .workflow_runtime_instances()
        .iter()
        .all(|instance| !instance.primary()));
}

#[test]
fn a_remote_backed_trigger_agent_keeps_running_on_its_lease() {
    let fixture = trigger_on_the_focus_agent();
    let agents = &fixture.runtime.owned.agent_store;
    let session = fixture
        .runtime
        .owned
        .session_store
        .get_session(&fixture.session_id)
        .expect("session should resolve");
    let workflow = session
        .workflow(&fixture.workflow_id)
        .expect("trigger workflow should exist")
        .clone();
    assert!(
        !crate::app::workflow_runtime::workflow_may_use_source_agents(agents, &session, &workflow),
        "a local agent's trigger runs on a runtime copy"
    );

    // A copy cannot keep a remote agent's placement: it has no lease of its
    // own, so the trigger keeps running on the agent's lease.
    let mut agent = agents
        .get_agent(&fixture.focus_agent_id)
        .expect("agent should exist");
    agent.set_remote_execution(Some(crate::agent::RemoteAgentBinding {
        worker_kernel_id: "worker-kernel".to_string(),
        worker_machine_id: "worker-machine".to_string(),
        execution_lease_id: "lease-1".to_string(),
        leased_agent_id: "leased-agent".to_string(),
        active_worker_provider_run_id: None,
        relay_url: None,
        relay_token: None,
        relay_peer_protocol_version: None,
    }));
    agents.restore_agent(agent);
    assert!(
        crate::app::workflow_runtime::workflow_may_use_source_agents(agents, &session, &workflow)
    );
}
