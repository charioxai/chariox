use super::*;

const AGENT_TERMINAL_CONTRACTS: &str =
    include_str!("../../../../runtime/terminal_operation_registry/contracts.json");

#[test]
fn generated_agent_terminal_contract_samples_match_serde_wire_shapes() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 260);

    let samples = [
        serde_json::json!({
            "UpdateAgentSubstitutes": {
                "session_id": "session-1",
                "agent_id": "agent-1",
                "action": { "Add": { "provider": "codex", "model": "gpt-5" } }
            }
        }),
        serde_json::json!({
            "RunAgentUtility": {
                "session_id": "session-1",
                "agent_id": "agent-1",
                "kind": "WorkspaceCommitMessage",
                "input": { "WorkspaceCommitMessage": {
                    "workspace_id": "/repo",
                    "worktree_id": "/repo"
                }}
            }
        }),
        serde_json::json!({
            "CreateWorkflowWatchdog": {
                "session_id": "session-1",
                "workflow_ref": "workflow-1",
                "endpoint_ref": "endpoint-1",
                "interval_seconds": 60,
                "invocation_prompt": "run",
                "policy": "skip",
                "max_wakeups_configured": false,
                "max_wakeups": null
            }
        }),
    ];

    for sample in samples {
        let request: LocalDaemonRequest =
            serde_json::from_value(sample.clone()).expect("generated sample should decode");
        assert_eq!(
            serde_json::to_value(request).expect("decoded sample should re-encode"),
            sample
        );
    }

    let contracts: serde_json::Value =
        serde_json::from_str(AGENT_TERMINAL_CONTRACTS).expect("generated contracts should parse");
    let action =
        &contracts["contracts"]["UpdateAgentSubstitutes"]["input_schema"]["properties"]["action"];
    assert!(action["oneOf"].as_array().is_some_and(|branches| {
        branches.iter().any(|branch| {
            branch["properties"]["Add"]["properties"]["provider"]
                == serde_json::json!({ "type": "string" })
        })
    }));
    let utility_input =
        &contracts["contracts"]["RunAgentUtility"]["input_schema"]["properties"]["input"];
    assert!(utility_input["oneOf"].as_array().is_some_and(|branches| {
        branches.iter().any(|branch| {
            branch["properties"]["WorkspaceCommitMessage"]["properties"]["workspace_id"]
                == serde_json::json!({ "type": "string" })
        })
    }));
    assert_eq!(
        contracts["contracts"]["CreateWorkflowWatchdog"]["input_schema"]["properties"]["policy"]
            ["enum"],
        serde_json::json!(["skip", "queue"])
    );
}
