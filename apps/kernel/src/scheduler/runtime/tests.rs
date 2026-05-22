use std::fs;
use std::path::PathBuf;

use crate::agent::CreateAgentRequest;
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::provider::LaunchProviderRequest;
use crate::session::{CreateSessionRequest, RuntimeSession, WorkflowMessage, WorkflowRun};
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
fn workflow_instruction_reference_is_written_under_agent_workdir() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent_id) = create_scheduler_session_and_agent(&mut app, "client-scheduler");

    let workdir = std::env::temp_dir().join(format!(
        "arroba-workflow-runtime-test-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&workdir);
    fs::create_dir_all(&workdir).expect("workdir should exist");
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
        .join("system-prompts")
        .join("workflow-turn.md");
    assert!(
        expected_prompt_template.exists(),
        "workflow system prompt template should be materialized"
    );
    let prompt_template_contents =
        fs::read_to_string(&expected_prompt_template).expect("template should read");
    assert!(prompt_template_contents.contains("ack_workflow_turn"));
    assert!(
        prompt.contains("If you do not remember them exactly, read that file before continuing.")
    );
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
