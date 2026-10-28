//! An OpenCode error answer to a prompt submission reaches the dispatch result
//! as the provider's refusal; faults of the connection do not.

use super::command_execution::execute_submit_command;
use super::runtime_slots::ProviderRunRuntimeRegistry;
use crate::error::DaemonError;
use crate::prompt_assembly::{PromptEnvelope, PromptManifest};
use crate::provider::{
    AgentEndpointMode, FakeOpenCodeServer, LaunchProviderRequest, OpenCodeEventSubscription,
    PromptReply, ProviderLaunchResult, RuntimeProviderRun,
};
use std::collections::BTreeMap;

const REFUSAL: &str = "Selected model is at capacity. Please try a different model.";

fn run_for(adapter: &str, endpoint: &str) -> RuntimeProviderRun {
    let request = LaunchProviderRequest::new("session", adapter, adapter, "default", "default")
        .with_agent_id("agent");
    RuntimeProviderRun::new(
        "run",
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "fixture".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some(endpoint.to_string()),
        },
    )
}

fn envelope(steering: bool) -> PromptEnvelope {
    PromptEnvelope::new("prompt", "", Vec::new(), PromptManifest::current()).with_steering(steering)
}

fn submit_to_opencode(reply: PromptReply) -> Result<(), DaemonError> {
    let server = FakeOpenCodeServer::start(reply);
    let registry = ProviderRunRuntimeRegistry::default();
    registry.insert_opencode_runtime(
        "run".to_string(),
        crate::provider::opencode_runtime::OpenCodeRuntimeState::new(
            server.endpoint.clone(),
            "session".to_string(),
            OpenCodeEventSubscription::for_tests(std::sync::mpsc::channel().1),
        ),
    );
    execute_submit_command(
        &registry,
        run_for("opencode", &server.endpoint),
        envelope(false),
        "prompt-1",
    )
    .map(|_| ())
}

#[test]
fn an_opencode_error_message_is_the_providers_refusal() {
    let error = submit_to_opencode(PromptReply::Status {
        code: 400,
        body: r#"{"name":"UnknownError","data":{"message":"Selected model is at capacity. Please try a different model."}}"#,
    })
    .expect_err("the provider refused the prompt");
    assert!(
        matches!(&error, DaemonError::ProviderRefusal { message } if message == REFUSAL),
        "{error:?}"
    );
}

#[test]
fn an_opencode_fault_without_provider_text_stays_a_protocol_fault() {
    for reply in [
        PromptReply::Dropped,
        PromptReply::Status {
            code: 500,
            body: "",
        },
        PromptReply::Status {
            code: 502,
            body: "<html>Bad gateway</html>",
        },
    ] {
        let error = submit_to_opencode(reply).expect_err("the submission fails");
        assert!(
            matches!(
                error,
                DaemonError::ProviderProtocol {
                    operation: "session_prompt",
                    ..
                }
            ),
            "{error:?}"
        );
    }
}
