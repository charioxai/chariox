use std::fs;
use std::path::PathBuf;

use crate::agent::CreateAgentRequest;
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::provider::LaunchProviderRequest;
use crate::session::{
    CreateSessionRequest, RuntimeSession, WorkflowMessage, WorkflowRun, WorkflowRunStatus,
};
use crate::{DaemonApp, DaemonConfig};

use super::prepare_workflow_turn_prompt;

fn create_scheduler_session_and_agent(
    app: &mut DaemonApp,
    client_id: &str,
) -> (RuntimeSession, String) {
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut *app)
        .create_session(CreateSessionRequest::new(
            "workspace-scheduler",
            "worktree-scheduler",
        ))
        .expect("session should exist");
    crate::app::KernelSessionService::new(app)
        .attach(AttachRequest::new(
            session.id(),
            client_id,
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("attachment should attach");
    let agent_id = crate::app::KernelSessionService::new(&mut *app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("agent-scheduler")
                .with_model("test-model")
                .with_worktree("worktree-scheduler"),
        )
        .expect("agent should spawn")
        .id()
        .to_string();
    (session, agent_id)
}

fn create_workflow_node(
    app: &mut DaemonApp,
    session_id: &str,
    workflow_alias: &str,
    agent_id: &str,
) -> (String, String) {
    let workflow_id = app
        .sessions_mut()
        .create_workflow(session_id, Some(workflow_alias.to_string()))
        .expect("workflow should exist")
        .id()
        .to_string();
    let node_id = app
        .sessions_mut()
        .add_workflow_node(session_id, &workflow_id, agent_id)
        .expect("node should be added")
        .id()
        .to_string();
    (workflow_id, node_id)
}

fn invoke_workflow_node(
    app: &mut DaemonApp,
    session_id: &str,
    workflow_id: &str,
    node_id: &str,
) -> WorkflowRun {
    app.sessions_mut()
        .set_workflow_flush_agent_context_before_run(session_id, &workflow_id, false)
        .expect("workflow flush context should update");
    app.sessions_mut()
        .create_workflow_endpoint(
            session_id,
            &workflow_id,
            &node_id,
            Some("entry".to_string()),
        )
        .expect("endpoint should exist");
    let (workflow_run, _, _) = app
        .invoke_workflow_endpoint_and_schedule(
            session_id,
            &workflow_id,
            "entry",
            Some("start".to_string()),
        )
        .expect("workflow should invoke");
    workflow_run
}

#[test]
fn workflow_start_preflights_local_provider_runs_for_all_nodes() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, first_agent_id) =
        create_scheduler_session_and_agent(&mut app, "client-scheduler-preflight");
    let second_agent_id = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("second-scheduler-agent")
                .with_model("test-model")
                .with_worktree("worktree-scheduler"),
        )
        .expect("second agent should spawn")
        .id()
        .to_string();
    let workflow_id = app
        .sessions_mut()
        .create_workflow(session.id(), Some("wf-scheduler-preflight".to_string()))
        .expect("workflow should exist")
        .id()
        .to_string();
    let first_node_id = app
        .sessions_mut()
        .add_workflow_node(session.id(), &workflow_id, &first_agent_id)
        .expect("first node should be added")
        .id()
        .to_string();
    let second_node_id = app
        .sessions_mut()
        .add_workflow_node(session.id(), &workflow_id, &second_agent_id)
        .expect("second node should be added")
        .id()
        .to_string();
    app.sessions_mut()
        .add_workflow_edge(
            session.id(),
            &workflow_id,
            &first_node_id,
            &second_node_id,
            None,
            None,
        )
        .expect("workflow edge should be added");
    app.sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            &workflow_id,
            &first_node_id,
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should exist");

    let workflow_run = app
        .invoke_workflow_endpoint_and_schedule(
            session.id(),
            &workflow_id,
            "entry",
            Some("start".to_string()),
        )
        .expect("workflow should invoke")
        .0;

    let first_provider_run = app
        .providers()
        .get_run_for_agent(session.id(), &first_agent_id)
        .expect("entry agent provider should be preflighted");
    let second_provider_run = app
        .providers()
        .get_run_for_agent(session.id(), &second_agent_id)
        .expect("downstream agent provider should be preflighted");
    assert_ne!(first_provider_run.id(), second_provider_run.id());
    assert_eq!(
        app.sessions()
            .get_session(session.id())
            .expect("session should resolve")
            .active_provider_run_id(),
        Some(first_provider_run.id()),
        "entry node should remain the active workflow provider after preflight"
    );
    assert_eq!(workflow_run.node_runs().len(), 1);
    assert_eq!(workflow_run.node_runs()[0].node_id(), first_node_id);
}

#[test]
fn workflow_notice_uses_current_run_after_dispatch_failure() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent_id) =
        create_scheduler_session_and_agent(&mut app, "client-scheduler-failed-notice");
    let (workflow_id, node_id) =
        create_workflow_node(&mut app, session.id(), "wf-failed-notice", &agent_id);
    let stale_workflow_run = invoke_workflow_node(&mut app, session.id(), &workflow_id, &node_id);
    let node_run_id = stale_workflow_run.node_runs()[0].id().to_string();
    app.sessions_mut()
        .fail_workflow_node_run(session.id(), stale_workflow_run.id(), &node_run_id)
        .expect("workflow node should fail");

    let current = super::lifecycle::current_workflow_run_for_notice(
        &app,
        session.id(),
        stale_workflow_run.clone(),
    );

    assert_eq!(stale_workflow_run.status(), WorkflowRunStatus::Running);
    assert_eq!(current.status(), WorkflowRunStatus::Failed);
    assert_eq!(
        super::lifecycle::workflow_run_status_notice_suffix(current.status()),
        "failed"
    );
}

#[test]
fn workflow_instruction_reference_is_written_under_agent_workdir() {
    let _guard = crate::env_lock::lock();
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent_id) = create_scheduler_session_and_agent(&mut app, "client-scheduler");

    let workdir = std::env::temp_dir().join(format!(
        "arroba-workflow-runtime-test-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&workdir);
    fs::create_dir_all(&workdir).expect("workdir should exist");
    let previous_arroba_home = std::env::var_os("ARROBA_HOME");
    std::env::set_var("ARROBA_HOME", workdir.join(".arroba"));
    app.launch_provider(
        LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "dev-stub",
            "default",
            "test-model",
        )
        .with_agent_id(agent_id.clone())
        .with_working_directory(workdir.clone()),
    )
    .expect("provider run should launch");

    let workflow_id = app
        .sessions_mut()
        .create_workflow(session.id(), Some("wf-scheduler".to_string()))
        .expect("workflow should exist")
        .id()
        .to_string();
    let node_id = app
        .sessions_mut()
        .add_workflow_node(session.id(), &workflow_id, &agent_id)
        .expect("node should be added")
        .id()
        .to_string();
    app.sessions_mut()
        .update_workflow_node_instructions(
            session.id(),
            &workflow_id,
            &node_id,
            Some("Read me from a workspace-local hidden file.".to_string()),
        )
        .expect("instructions should update");
    app.sessions_mut()
        .set_workflow_flush_agent_context_before_run(session.id(), &workflow_id, false)
        .expect("workflow flush context should update");
    app.sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            &workflow_id,
            &node_id,
            Some("entry".to_string()),
        )
        .expect("endpoint should exist");
    let (workflow_run, _, _) = app
        .invoke_workflow_endpoint_and_schedule(
            session.id(),
            &workflow_id,
            "entry",
            Some("start".to_string()),
        )
        .expect("workflow should invoke");
    let node_run_id = workflow_run
        .node_runs()
        .first()
        .expect("node run should exist")
        .id()
        .to_string();

    let prompt = prepare_workflow_turn_prompt(
        &app,
        session.id(),
        workflow_run.id(),
        &node_run_id,
        &node_id,
        "start",
        Option::<&[WorkflowMessage]>::None,
    )
    .expect("prompt should build");

    let prefix = workdir
        .join(".arroba")
        .join("workflow-runtime")
        .join(session.id())
        .join(workflow_run.id())
        .join("workflow-instructions");
    let prefix_string = prefix.to_string_lossy().to_string();
    assert!(
        prompt.contains(&prefix_string),
        "prompt should reference a file under agent workdir: {prompt}"
    );
    let expected_file = prefix.join(format!("node-{node_id}.md"));
    assert!(expected_file.exists(), "instruction file should be written");
    let contents = fs::read_to_string(&expected_file).expect("instruction file should read");
    assert!(contents.contains("Read me from a workspace-local hidden file."));
    let expected_prompt_template = workdir
        .join(".arroba")
        .join("prompts")
        .join("workflow")
        .join("turn.md");
    assert!(
        expected_prompt_template.exists(),
        "workflow system prompt template should be materialized"
    );
    let prompt_template_contents =
        fs::read_to_string(&expected_prompt_template).expect("template should read");
    assert!(prompt_template_contents.contains("ack_workflow_turn"));
    assert!(prompt_template_contents.contains("Do not ask the user which workflow runtime tool"));
    assert!(
        prompt.contains("If you do not remember them exactly, read that file before continuing.")
    );
    assert!(prompt.contains("Do not ask the user which workflow runtime tool"));
    if let Some(previous_arroba_home) = previous_arroba_home {
        std::env::set_var("ARROBA_HOME", previous_arroba_home);
    } else {
        std::env::remove_var("ARROBA_HOME");
    }
    let _ = fs::remove_dir_all(PathBuf::from(workdir));
}

#[test]
fn terminating_nodes_receive_completion_and_last_turn_prompt_blocks() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent_id) =
        create_scheduler_session_and_agent(&mut app, "client-scheduler-terminating");

    let (workflow_id, node_id) = create_workflow_node(
        &mut app,
        session.id(),
        "wf-scheduler-terminating",
        &agent_id,
    );
    app.sessions_mut()
        .set_workflow_node_can_complete_run(session.id(), &workflow_id, &node_id, true)
        .expect("node completion setting should update");
    app.sessions_mut()
        .set_workflow_node_max_turns(session.id(), &workflow_id, &node_id, Some(1))
        .expect("node max turns should update");
    let workflow_run = invoke_workflow_node(&mut app, session.id(), &workflow_id, &node_id);
    let node_run_id = workflow_run
        .node_runs()
        .first()
        .expect("node run should exist")
        .id()
        .to_string();
    let prompt = prepare_workflow_turn_prompt(
        &app,
        session.id(),
        workflow_run.id(),
        &node_run_id,
        &node_id,
        "start",
        Option::<&[WorkflowMessage]>::None,
    )
    .expect("prompt should build");

    assert!(prompt.contains("This node is authorized to complete the workflow run."));
    assert!(prompt.contains("This is turn 1 for this node in the current workflow run."));
    assert!(
        prompt.contains("This is the last allowed turn for this node in the current workflow run.")
    );
    assert!(prompt.contains("validate_and_submit_workflow_run_output"));
}

#[test]
fn non_last_turn_nodes_still_receive_turn_index_prompt_block() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent_id) =
        create_scheduler_session_and_agent(&mut app, "client-scheduler-turn-index");

    let (workflow_id, node_id) =
        create_workflow_node(&mut app, session.id(), "wf-scheduler-turn-index", &agent_id);
    app.sessions_mut()
        .set_workflow_node_max_turns(session.id(), &workflow_id, &node_id, Some(3))
        .expect("node max turns should update");
    let workflow_run = invoke_workflow_node(&mut app, session.id(), &workflow_id, &node_id);
    let node_run_id = workflow_run
        .node_runs()
        .first()
        .expect("node run should exist")
        .id()
        .to_string();
    let prompt = prepare_workflow_turn_prompt(
        &app,
        session.id(),
        workflow_run.id(),
        &node_run_id,
        &node_id,
        "start",
        Option::<&[WorkflowMessage]>::None,
    )
    .expect("prompt should build");

    assert!(prompt.contains("This is turn 1 for this node in the current workflow run."));
    assert!(prompt.contains("- node max turns: 3"));
    assert!(!prompt
        .contains("This is the last allowed turn for this node in the current workflow run."));
}
