//! Instruction files follow the lifecycle of runs invoked through the kernel.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;

#[path = "support/real_kernel.rs"]
mod real_kernel;
#[allow(dead_code)]
#[path = "../src/test_temp_dir.rs"]
mod test_temp_dir;
use real_kernel::{connect, create_session, request, Socket};

const DAEMON: &str = "instruction-lifecycle";

async fn invoke(
    socket: &mut Socket,
    scratch: &Path,
    model: &str,
    output_schema: Option<&Path>,
) -> (String, String, String) {
    let session = create_session(socket, scratch).await;
    let agent = request(
        socket,
        json!({"SpawnAgent": {"session_id": session, "alias": "instruction-agent",
            "provider": "dev-stub", "model": model, "no_subscribe": true}}),
    )
    .await
    .unwrap();
    let agent = agent["AgentSpawned"]["agent"]["id"].as_str().unwrap();
    let workflow = request(
        socket,
        json!({"CreateWorkflow": {"session_id": session, "alias": "instruction-flow"}}),
    )
    .await
    .unwrap();
    let workflow = workflow["WorkflowCreated"]["workflow"]["id"]
        .as_str()
        .unwrap();
    if let Some(schema) = output_schema {
        request(
            socket,
            json!({"SetWorkflowRunOutputSchema": {"session_id": session, "workflow_ref": workflow,
                "run_output_schema_ref": schema}}),
        )
        .await
        .unwrap();
    }
    let node = request(
        socket,
        json!({"AddWorkflowNode": {"session_id": session, "workflow_ref": workflow,
            "agent_id": agent}}),
    )
    .await
    .unwrap();
    let node = node["WorkflowNodeAdded"]["node"]["id"].as_str().unwrap();
    request(
        socket,
        json!({"UpdateWorkflowNodeInstructions": {"session_id": session, "workflow_ref": workflow,
            "node_id": node, "instructions": "workflow instruction lifecycle witness"}}),
    )
    .await
    .unwrap();
    request(
        socket,
        json!({"SetWorkflowNodeCanCompleteRun": {"session_id": session, "workflow_ref": workflow,
            "node_id": node, "can_complete_workflow_run": true}}),
    )
    .await
    .unwrap();
    request(
        socket,
        json!({"CreateWorkflowEndpoint": {"session_id": session, "workflow_ref": workflow,
            "entry_node_id": node, "alias": "entry"}}),
    )
    .await
    .unwrap();
    let run = request(
        socket,
        json!({"InvokeWorkflowEndpoint": {"session_id": session, "workflow_ref": workflow,
            "endpoint_ref": "entry", "prompt": "Start the workflow."}}),
    )
    .await
    .unwrap();
    let run = run["WorkflowRunInvoked"]["workflow_run"]["id"]
        .as_str()
        .unwrap();
    (session, run.into(), node.into())
}

fn instruction_file(home: &Path, session: &str, run: &str, node: &str) -> PathBuf {
    home.join("workflow-instructions")
        .join(DAEMON)
        .join(session)
        .join(run)
        .join(format!("node-{node}.md"))
}

async fn wait_for_file(file: &Path) {
    tokio::time::timeout(Duration::from_secs(15), async {
        while !file.is_file() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("the workflow must materialize its instructions");
}

async fn wait_for_status(socket: &mut Socket, session: &str, run: &str, expected: &str) {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let response = request(
                socket,
                json!({"GetWorkflowRun": {
                    "session_id": session, "workflow_run_ref": run,
                }}),
            )
            .await
            .unwrap();
            let status = response["WorkflowRun"]["workflow_run"]["status"]
                .as_str()
                .unwrap();
            if status == expected {
                break;
            }
            assert!(
                !matches!(status, "Completed" | "Failed" | "Stopped"),
                "{response}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the workflow must reach its expected status");
}

async fn cancel_run(socket: &mut Socket, session: &str, run: &str) {
    wait_for_status(socket, session, run, "Running").await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match request(
                socket,
                json!({"CancelWorkflowRun": {
                    "session_id": session, "workflow_run_ref": run,
                }}),
            )
            .await
            {
                Ok(_) => break,
                Err(error) if error["code"] == "no_active_prompt" => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(error) => panic!("workflow cancellation failed: {error}"),
            }
        }
    })
    .await
    .expect("the workflow must accept cancellation once its prompt is active");
}

#[tokio::test]
async fn terminal_workflow_runs_remove_their_instructions() {
    let scratch = test_temp_dir::TestTempDir::new("workflow-instruction-lifecycle");
    let kernel = real_kernel::start(
        &scratch,
        &[
            ("CHARIOX_DAEMON_ID", DAEMON.into()),
            ("CHARIOX_PROVIDER_DEV_STUB", "1".into()),
            ("CHARIOX_DEV_STUB_WORKFLOW_OUTPUT_DELAY_MS", "3000".into()),
        ],
        None,
    );
    let mut socket = connect(&kernel).await;
    let home = scratch.join("home/.chariox");
    let rejected_output = scratch.join("project/rejected-output.json");
    std::fs::write(
        &rejected_output,
        r#"{"type":"object","required":["different_field"]}"#,
    )
    .unwrap();
    for (model, expected, schema) in [
        ("workflow-test-idle", "Stopped", None),
        ("workflow-delayed-output", "Completed", None),
        (
            "workflow-delayed-output",
            "Failed",
            Some(rejected_output.as_path()),
        ),
    ] {
        let (session, run, node) = invoke(&mut socket, &scratch, model, schema).await;
        let file = instruction_file(&home, &session, &run, &node);
        wait_for_file(&file).await;
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "workflow instruction lifecycle witness"
        );
        if expected == "Stopped" {
            cancel_run(&mut socket, &session, &run).await;
        }
        wait_for_status(&mut socket, &session, &run, expected).await;
        let directory = file.parent().unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while directory.exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{expected} run left its instruction directory"));
    }
}

#[tokio::test]
async fn cold_start_removes_instructions_left_after_a_terminal_commit() {
    let scratch = test_temp_dir::TestTempDir::new("workflow-instruction-restart");
    let home = scratch.join(".chariox");
    let environment = [
        ("CHARIOX_DAEMON_ID", DAEMON.into()),
        ("CHARIOX_PROVIDER_DEV_STUB", "1".into()),
        ("CHARIOX_HOME", home.clone().into_os_string()),
    ];
    let first = scratch.join("first");
    let kernel = real_kernel::start(&first, &environment, None);
    let mut socket = connect(&kernel).await;
    let (session, run, node) = invoke(&mut socket, &first, "workflow-test-idle", None).await;
    let file = instruction_file(&home, &session, &run, &node);
    wait_for_file(&file).await;
    cancel_run(&mut socket, &session, &run).await;
    wait_for_status(&mut socket, &session, &run, "Stopped").await;
    drop(socket);
    drop(kernel);

    // Recreate the filesystem state of a crash between the terminal commit and deletion.
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "interrupted cleanup").unwrap();
    let restarted = real_kernel::start(&scratch.join("second"), &environment, None);
    let mut socket = connect(&restarted).await;
    wait_for_status(&mut socket, &session, &run, "Stopped").await;
    assert!(
        !file.parent().unwrap().exists(),
        "startup left a terminal run's instructions"
    );
}
