use super::super::*;
use crate::agent::{AgentInstance, GridPosition};
use crate::app::sanitize_resume_state_for_launch;
use crate::error::DaemonError;
use crate::provider::{LaunchProviderRequest, ProviderResumeState};

#[test]
fn sanitize_resume_state_keeps_adapter_resume_when_model_and_variant_match() {
    let mut agent = AgentInstance::new(
        "agent-1",
        "agent-1",
        "session-1",
        None,
        "opencode",
        Some("openai/gpt-5.4".to_string()),
        Some("high".to_string()),
        None,
        GridPosition::new(0, 0, 1, 1),
    );
    let mut resume_state = ProviderResumeState::from_opencode_session_id("open-session-1");
    resume_state.set_codex_thread_id("thread-1");
    agent.set_provider_resume_state(resume_state.clone());
    let request = LaunchProviderRequest::new(
        "session-1",
        "opencode",
        "opencode",
        "default",
        "openai/gpt-5.4",
    )
    .with_variant(Some("high".to_string()));

    assert_eq!(
        sanitize_resume_state_for_launch(&request, &agent),
        resume_state
    );
}

#[test]
fn sanitize_resume_state_keeps_codex_resume_when_request_model_is_unprefixed() {
    let mut agent = AgentInstance::new(
        "agent-1",
        "agent-1",
        "session-1",
        None,
        "codex",
        Some("codex/gpt-5.5".to_string()),
        Some("high".to_string()),
        None,
        GridPosition::new(0, 0, 1, 1),
    );
    let resume_state = ProviderResumeState::from_codex_thread_id("thread-1");
    agent.set_provider_resume_state(resume_state.clone());
    let request = LaunchProviderRequest::new("session-1", "codex", "codex", "default", "gpt-5.5")
        .with_variant(Some("high".to_string()));

    assert_eq!(
        sanitize_resume_state_for_launch(&request, &agent),
        resume_state
    );
}

#[test]
fn sanitize_resume_state_keeps_resume_when_agent_model_is_unknown() {
    let mut agent = AgentInstance::new(
        "agent-1",
        "agent-1",
        "session-1",
        None,
        "codex",
        None,
        None,
        None,
        GridPosition::new(0, 0, 1, 1),
    );
    let resume_state = ProviderResumeState::from_codex_thread_id("thread-1");
    agent.set_provider_resume_state(resume_state.clone());
    let request = LaunchProviderRequest::new("session-1", "codex", "codex", "default", "default");

    assert_eq!(
        sanitize_resume_state_for_launch(&request, &agent),
        resume_state
    );
}

#[test]
fn sanitize_resume_state_clears_opencode_resume_when_model_changes() {
    let mut agent = AgentInstance::new(
        "agent-1",
        "agent-1",
        "session-1",
        None,
        "opencode",
        Some("openai/gpt-5.4".to_string()),
        Some("high".to_string()),
        None,
        GridPosition::new(0, 0, 1, 1),
    );
    let mut resume_state = ProviderResumeState::from_opencode_session_id("open-session-1");
    resume_state.set_codex_thread_id("thread-1");
    agent.set_provider_resume_state(resume_state);
    let request = LaunchProviderRequest::new(
        "session-1",
        "opencode",
        "opencode",
        "default",
        "anthropic/claude-sonnet-4",
    )
    .with_variant(Some("high".to_string()));

    let sanitized = sanitize_resume_state_for_launch(&request, &agent);
    assert_eq!(sanitized.opencode_session_id(), None);
    assert_eq!(sanitized.codex_thread_id(), Some("thread-1"));
}

#[test]
fn sanitize_resume_state_clears_codex_resume_when_variant_changes() {
    let mut agent = AgentInstance::new(
        "agent-1",
        "agent-1",
        "session-1",
        None,
        "codex",
        Some("gpt-5.4".to_string()),
        Some("medium".to_string()),
        None,
        GridPosition::new(0, 0, 1, 1),
    );
    let mut resume_state = ProviderResumeState::from_codex_thread_id("thread-1");
    resume_state.set_opencode_session_id("open-session-1");
    agent.set_provider_resume_state(resume_state);
    let request = LaunchProviderRequest::new("session-1", "codex", "codex", "default", "gpt-5.4")
        .with_variant(Some("high".to_string()));

    let sanitized = sanitize_resume_state_for_launch(&request, &agent);
    assert_eq!(sanitized.opencode_session_id(), Some("open-session-1"));
    assert_eq!(sanitized.codex_thread_id(), None);
}

#[test]
fn codex_resume_failure_replacement_clears_only_codex_thread() {
    let mut resume_state = ProviderResumeState::from_codex_thread_id("thread-1");
    resume_state.set_opencode_session_id("open-session-1");
    let request = LaunchProviderRequest::new("session-1", "codex", "codex", "default", "gpt-5.5")
        .with_agent_id("agent-1")
        .with_resume_state(resume_state);
    let run = RuntimeProviderRun::new(
        "provider-run-1",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "codex".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: Default::default(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("ws://127.0.0.1:43123".to_string()),
        },
    );
    let error = DaemonError::ProviderProtocol {
        provider_run_id: run.id().to_string(),
        operation: "codex_thread_resume",
        message: "Codex could not resume thread `thread-1`: no rollout found".to_string(),
    };

    let replacement = failed_provider_resume_state_replacement(&run, &error)
        .expect("failed Codex resume should clear the stale thread id");

    assert_eq!(replacement.codex_thread_id(), None);
    assert_eq!(replacement.opencode_session_id(), Some("open-session-1"));
}
