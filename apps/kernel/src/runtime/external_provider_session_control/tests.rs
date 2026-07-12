use super::*;
use crate::config::DaemonConfig;
use crate::history::SessionHistoryEntryKind;
use crate::local::{
    ExternalProviderSessionCapabilities, ImportExternalProviderAgentRequest,
    ImportExternalProviderSessionRequest,
};
use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

mod alias;
mod history_authority;
mod import;
mod poller;

pub(super) fn observer_target(agent_id: &str) -> AttachedExternalObserverTarget {
    attached_external_observer_target_from_import(
        format!("session-{agent_id}"),
        agent_id.to_string(),
        None,
        ExternalProviderImportMetadata::observed_history(
            format!("codex:thread-{agent_id}"),
            "codex".to_string(),
            format!("thread-{agent_id}"),
        ),
    )
}

#[test]
fn external_observed_history_entry_match_includes_visible_metadata() {
    let existing = ExternalImportHistoryEntry {
        kind: SessionHistoryEntryKind::ProviderOutput,
        text: "same output".to_string(),
        external_provider: Some("codex".to_string()),
        external_provider_session_id: Some("thread-1".to_string()),
        external_provider_turn_id: Some("assistant-1".to_string()),
        observed_at_ms: Some(2_000),
        external_observation: None,
    };
    let next = SessionHistoryEntry::external_provider_observed(
        "session-1",
        None,
        "agent-1",
        SessionHistoryEntryKind::ProviderOutput,
        "same output",
        "codex",
        "thread-1",
        Some("assistant-1".to_string()),
        Some(2_100),
    );

    assert!(
        !external_observed_history_entry_matches(&existing, &next),
        "metadata-only updates must be persisted and fanned out"
    );
}

pub(super) fn test_codex_run(
    session_id: &str,
    agent_id: &str,
    provider_run_id: &str,
    provider_session_id: &str,
) -> RuntimeProviderRun {
    let request = LaunchProviderRequest::new(session_id, "codex", "codex", "default", "gpt-test")
        .with_agent_id(agent_id);
    let launch = crate::provider::ProviderLaunchResult {
        process_label: "codex:test".to_string(),
        endpoint_mode: crate::provider::AgentEndpointMode::External,
        pty_target: None,
        pty_program: None,
        pty_args: Vec::new(),
        pty_env: BTreeMap::new(),
        pty_env_remove: Vec::new(),
        working_directory: None,
        structured_endpoint: Some("codex:test".to_string()),
    };
    let mut run = RuntimeProviderRun::new(provider_run_id, &request, launch);
    run.set_provider_session_id(Some(provider_session_id.to_string()));
    run.mark_running();
    run
}

pub(super) fn test_starting_codex_run(
    session_id: &str,
    agent_id: &str,
    provider_run_id: &str,
    provider_session_id: &str,
) -> RuntimeProviderRun {
    let request = LaunchProviderRequest::new(session_id, "codex", "codex", "default", "gpt-test")
        .with_agent_id(agent_id);
    let launch = crate::provider::ProviderLaunchResult {
        process_label: "codex:test".to_string(),
        endpoint_mode: crate::provider::AgentEndpointMode::External,
        pty_target: None,
        pty_program: None,
        pty_args: Vec::new(),
        pty_env: BTreeMap::new(),
        pty_env_remove: Vec::new(),
        working_directory: None,
        structured_endpoint: Some("codex:test".to_string()),
    };
    let mut run = RuntimeProviderRun::new(provider_run_id, &request, launch);
    run.set_provider_session_id(Some(provider_session_id.to_string()));
    run
}

pub(super) fn single_attached_target(app: &DaemonApp) -> AttachedExternalObserverTarget {
    let targets = attached_external_observer_targets(app);
    assert_eq!(
        targets.len(),
        1,
        "expected exactly one attached observer target"
    );
    targets.into_iter().next().expect("target should exist")
}

pub(super) fn attach_test_session(app: &DaemonApp, session_id: &str) {
    let session_store = app.session_state_store();
    let mut sessions = session_store.write();
    app.attachments()
        .attach(
            &mut sessions,
            crate::attachment::AttachRequest::new(
                session_id,
                format!("client-{session_id}"),
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ),
        )
        .expect("test attachment should be created");
}

pub(super) fn record(
    provider: &str,
    provider_session_id: &str,
    worktree_path: &str,
) -> ExternalProviderSessionRecord {
    ExternalProviderSessionRecord {
        external_session_id: format!("{provider}:{provider_session_id}"),
        provider: provider.to_string(),
        provider_session_id: provider_session_id.to_string(),
        title: Some(provider_session_id.to_string()),
        title_source: Some("test".to_string()),
        first_prompt_preview: Some("test prompt".to_string()),
        created_at_ms: None,
        last_modified_at_ms: 10,
        worktree_path: Some(worktree_path.to_string()),
        account_profile: None,
        capabilities: ExternalProviderSessionCapabilities {
            can_read_history: true,
            ..ExternalProviderSessionCapabilities::default()
        },
        attached_to_arroba: false,
        attached_session_ids: Vec::new(),
        attached_agent_ids: Vec::new(),
    }
}

pub(super) fn temp_root(name: &str) -> PathBuf {
    let path = env::temp_dir().join(format!("arroba-{name}-{}", crate::session::unix_epoch_ms()));
    fs::create_dir_all(&path).expect("temp root should create");
    path
}

pub(super) fn restore_env_var(key: &str, previous: Option<std::ffi::OsString>) {
    match previous {
        Some(value) => env::set_var(key, value),
        None => env::remove_var(key),
    }
}
