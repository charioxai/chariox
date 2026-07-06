use crate::app::{DaemonApp, PromptActivityStore};
use crate::error::DaemonError;
use crate::provider::{
    classify_provider_terminal_failure_text, ProviderPromptSignalBatch, RuntimeProviderRun,
};
use crate::provider::{AgentEndpointMode, ProviderProcessServiceStore, ProviderRunState};
use crate::pty::PtyOutputChunk;
use crate::runtime::projection::AgentRuntimeProjectionStore;
use crate::terminal::{TerminalOutputKind, TerminalOutputRecord};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use super::provider_output_claude_native::ProviderOutputClaudeNativeBridge;
use super::provider_output_fanout::ProviderOutputFanout;
use super::provider_output_prompt_settlement::ProviderOutputPromptSettlement;
use super::provider_output_trace::ProviderOutputTrace;

pub(crate) const STRUCTURED_OUTPUT_EMPTY_POLL_BACKOFF_MS: u64 = 500;

#[derive(Clone, Default)]
pub(crate) struct StructuredOutputRecordStore {
    records: Arc<Mutex<BTreeMap<String, Vec<TerminalOutputRecord>>>>,
    next_poll_due_at_ms: Arc<Mutex<BTreeMap<String, u64>>>,
}

impl StructuredOutputRecordStore {
    pub(crate) fn take(&self, provider_run_id: &str) -> Vec<TerminalOutputRecord> {
        self.records
            .lock()
            .expect("structured output record store poisoned")
            .remove(provider_run_id)
            .unwrap_or_default()
    }

    pub(crate) fn append(&self, provider_run_id: String, records: Vec<TerminalOutputRecord>) {
        if records.is_empty() {
            return;
        }
        self.records
            .lock()
            .expect("structured output record store poisoned")
            .entry(provider_run_id)
            .or_default()
            .extend(records);
    }

    pub(crate) fn poll_due(&self, provider_run_id: &str, now_ms: u64) -> bool {
        self.next_poll_due_at_ms
            .lock()
            .expect("structured output poll schedule poisoned")
            .get(provider_run_id)
            .is_none_or(|due_at_ms| *due_at_ms <= now_ms)
    }

    pub(crate) fn mark_poll_enqueued(&self, provider_run_id: &str) {
        self.next_poll_due_at_ms
            .lock()
            .expect("structured output poll schedule poisoned")
            .remove(provider_run_id);
    }

    pub(crate) fn schedule_next_poll(&self, provider_run_id: String, due_at_ms: u64) {
        self.next_poll_due_at_ms
            .lock()
            .expect("structured output poll schedule poisoned")
            .insert(provider_run_id, due_at_ms);
    }

    pub(crate) fn poll_due_at_ms(&self, provider_run_id: &str) -> Option<u64> {
        self.next_poll_due_at_ms
            .lock()
            .expect("structured output poll schedule poisoned")
            .get(provider_run_id)
            .copied()
    }

    pub(crate) fn clear(&self, provider_run_id: &str) {
        self.records
            .lock()
            .expect("structured output record store poisoned")
            .remove(provider_run_id);
        self.next_poll_due_at_ms
            .lock()
            .expect("structured output poll schedule poisoned")
            .remove(provider_run_id);
    }

    pub(crate) fn schedule_after_empty_poll(
        &self,
        provider_run_id: impl Into<String>,
        now_ms: u64,
    ) {
        self.schedule_next_poll(
            provider_run_id.into(),
            now_ms.saturating_add(STRUCTURED_OUTPUT_EMPTY_POLL_BACKOFF_MS),
        );
    }
}

pub(crate) fn structured_output_batch_should_poll_immediately(
    batch: &ProviderPromptSignalBatch,
) -> bool {
    !batch.chunks.is_empty()
        || !batch.completions.is_empty()
        || batch.prompt_completed
        || batch.terminal_failure.is_some()
        || !batch.notices.is_empty()
}

pub(crate) struct ProviderOutputPumpRequest<'a> {
    pub(crate) session_id: &'a str,
    pub(crate) provider_run_id: &'a str,
    pub(crate) recipient_attachment_ids: Vec<String>,
    pub(crate) initial_liveness_already_checked: bool,
}

pub(crate) fn pump_terminal_output_for_attachment(
    app: &mut DaemonApp,
    session_id: &str,
    attachment_id: &str,
) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
    reap_structured_prompt_jobs(app);
    reap_provider_first_output_timeouts(app, session_id)?;
    reap_provider_inactivity_timeouts(app, session_id)?;
    crate::app::KernelSessionReadService::new(app)
        .ensure_attachment_in_session(session_id, attachment_id)?;
    pump_session_active_prompt_outputs(app, session_id);
    Ok(app.terminal.drain_output_records(session_id, attachment_id))
}

pub(crate) fn reap_structured_prompt_jobs(app: &mut DaemonApp) {
    ProviderOutputStructuredPromptReaper::new(app).reap();
}

pub(crate) fn pump_active_prompt_outputs(app: &mut DaemonApp) -> Vec<String> {
    reap_structured_prompt_jobs(app);
    let sessions = app.sessions.list_sessions();
    let mut pumped_provider_run_ids = Vec::new();
    for session in sessions {
        if let Err(error) = reap_provider_first_output_timeouts(app, session.id()) {
            crate::logging::warn_with_fields(
                "daemon.provider_output",
                "provider first-output timeout reap failed",
                serde_json::json!({
                    "session_id": session.id(),
                    "error": error.to_string(),
                }),
            );
        }
        if let Err(error) = reap_provider_inactivity_timeouts(app, session.id()) {
            crate::logging::warn_with_fields(
                "daemon.provider_output",
                "provider inactivity timeout reap failed",
                serde_json::json!({
                    "session_id": session.id(),
                    "error": error.to_string(),
                }),
            );
        }
        pumped_provider_run_ids.extend(pump_session_active_prompt_outputs(app, session.id()));
    }
    pumped_provider_run_ids
}

#[cfg(test)]
mod structured_output_record_store_tests {
    use super::StructuredOutputRecordStore;

    #[test]
    fn structured_output_poll_schedule_defers_empty_poll_reenqueue() {
        let store = StructuredOutputRecordStore::default();

        assert!(store.poll_due("provider-run-1", 1_000));

        store.schedule_next_poll("provider-run-1".to_string(), 1_500);

        assert!(!store.poll_due("provider-run-1", 1_499));
        assert!(store.poll_due("provider-run-1", 1_500));
        assert_eq!(store.poll_due_at_ms("provider-run-1"), Some(1_500));

        store.mark_poll_enqueued("provider-run-1");

        assert!(store.poll_due("provider-run-1", 1_501));
        assert_eq!(store.poll_due_at_ms("provider-run-1"), None);
    }
}

fn reap_provider_first_output_timeouts(
    app: &mut DaemonApp,
    session_id: &str,
) -> Result<(), DaemonError> {
    let timed_out = app_first_output_timeout_candidates(app, session_id);
    for timeout in timed_out {
        let diagnostic = crate::app::provider_first_output_timeout_diagnostic(timeout.elapsed_ms);
        let run = app
            .providers
            .record_terminal_diagnostic(&timeout.provider_run_id, diagnostic.clone())?;
        app.update_provider_run_projection(run);
        app.record_notice(
            session_id,
            Some(&timeout.provider_run_id),
            app.attachments.list_session_attachment_ids(session_id),
            diagnostic.clone(),
        );
        crate::logging::warn_with_fields(
            "daemon.provider",
            "provider prompt produced no first output before timeout",
            serde_json::json!({
                "session_id": session_id,
                "agent_id": timeout.agent_id,
                "provider_run_id": timeout.provider_run_id,
                "elapsed_ms": timeout.elapsed_ms,
            }),
        );
        let provider_store = app.providers.clone();
        let active_turns = app.active_turns.clone();
        let prompt_activity = app.prompt_activity.clone();
        let agent_runtime_projection = app.agent_runtime_projection_store();
        ProviderOutputPromptSettlement::new(
            app,
            provider_store,
            active_turns,
            prompt_activity,
            agent_runtime_projection,
        )
        .fail_for_terminal_failure(session_id, &timeout.provider_run_id, &diagnostic)?;
    }
    Ok(())
}

fn reap_provider_inactivity_timeouts(
    app: &mut DaemonApp,
    session_id: &str,
) -> Result<(), DaemonError> {
    let timed_out = app_inactivity_timeout_candidates(app, session_id);
    for timeout in timed_out {
        let diagnostic = crate::app::provider_inactivity_timeout_diagnostic(timeout.elapsed_ms);
        let run = app
            .providers
            .record_terminal_diagnostic(&timeout.provider_run_id, diagnostic.clone())?;
        app.update_provider_run_projection(run);
        app.record_notice(
            session_id,
            Some(&timeout.provider_run_id),
            app.attachments.list_session_attachment_ids(session_id),
            diagnostic.clone(),
        );
        crate::logging::warn_with_fields(
            "daemon.provider",
            "provider prompt produced no output after prior activity before timeout",
            serde_json::json!({
                "session_id": session_id,
                "agent_id": timeout.agent_id,
                "provider_run_id": timeout.provider_run_id,
                "elapsed_ms": timeout.elapsed_ms,
            }),
        );
        let provider_store = app.providers.clone();
        let active_turns = app.active_turns.clone();
        let prompt_activity = app.prompt_activity.clone();
        let agent_runtime_projection = app.agent_runtime_projection_store();
        ProviderOutputPromptSettlement::new(
            app,
            provider_store,
            active_turns,
            prompt_activity,
            agent_runtime_projection,
        )
        .fail_for_terminal_failure(session_id, &timeout.provider_run_id, &diagnostic)?;
    }
    Ok(())
}

fn app_first_output_timeout_candidates(
    app: &DaemonApp,
    session_id: &str,
) -> Vec<crate::app::ProviderFirstOutputTimeoutCandidate> {
    let prompt_activity = app.prompt_activity.read().clone();
    let active_turns = app.active_turns.snapshot();
    let Ok(session) = app.sessions.get_session(session_id) else {
        return Vec::new();
    };
    crate::app::provider_first_output_timeout_candidates(
        session_id,
        active_turns.into_values(),
        &prompt_activity,
        |turn| {
            app.providers
                .get_run(&turn.provider_run_id)
                .is_ok_and(|run| {
                    run.session_id() == session_id
                        && run.agent_instance_id() == Some(turn.agent_id.as_str())
                        && run.terminal_diagnostic().is_none()
                        && matches!(
                            run.state(),
                            ProviderRunState::Starting
                                | ProviderRunState::Running
                                | ProviderRunState::Parked
                        )
                })
        },
        |turn| {
            app.prompt_state_owner
                .active_prompt_for_agent_snapshot(&session, &turn.agent_id)
                .is_some_and(|prompt| prompt.id() == turn.prompt_id)
        },
    )
}

fn app_inactivity_timeout_candidates(
    app: &DaemonApp,
    session_id: &str,
) -> Vec<crate::app::ProviderInactivityTimeoutCandidate> {
    let prompt_activity = app.prompt_activity.read().clone();
    let active_turns = app.active_turns.snapshot();
    let Ok(session) = app.sessions.get_session(session_id) else {
        return Vec::new();
    };
    crate::app::provider_inactivity_timeout_candidates(
        session_id,
        active_turns.into_values(),
        &prompt_activity,
        |turn| {
            app.providers
                .get_run(&turn.provider_run_id)
                .is_ok_and(|run| {
                    run.session_id() == session_id
                        && run.agent_instance_id() == Some(turn.agent_id.as_str())
                        && run.terminal_diagnostic().is_none()
                        && matches!(
                            run.state(),
                            ProviderRunState::Starting
                                | ProviderRunState::Running
                                | ProviderRunState::Parked
                        )
                })
        },
        |turn| {
            app.prompt_state_owner
                .active_prompt_for_agent_snapshot(&session, &turn.agent_id)
                .is_some_and(|prompt| prompt.id() == turn.prompt_id)
        },
    )
}

fn pump_session_active_prompt_outputs(app: &mut DaemonApp, session_id: &str) -> Vec<String> {
    let Ok(session) = app.sessions.get_session(session_id) else {
        return Vec::new();
    };
    let recipient_attachment_ids = app.attachments.list_session_attachment_ids(session.id());
    let mut provider_run_ids = BTreeSet::new();
    if let Some(provider_run_id) = session.active_provider_run_id().filter(|run_id| {
        app.providers
            .get_run(run_id)
            .is_ok_and(|run| provider_run_requires_background_pump(app, &session, &run))
    }) {
        provider_run_ids.insert(provider_run_id.to_string());
    }
    let mut agent_ids = session
        .agents()
        .iter()
        .map(|agent| agent.id().to_string())
        .collect::<Vec<_>>();
    agent_ids.extend(session.prompt_states().keys().cloned());
    agent_ids.sort();
    agent_ids.dedup();
    for agent_id in agent_ids {
        if app
            .prompt_state_owner
            .active_prompt_for_agent_snapshot(&session, &agent_id)
            .is_none()
        {
            continue;
        }
        if let Some(provider_run_id) = app
            .providers
            .get_run_for_agent(session.id(), &agent_id)
            .map(|run| run.id().to_string())
        {
            provider_run_ids.insert(provider_run_id);
        }
    }
    provider_run_ids.extend(
        app.providers
            .list_runs()
            .into_iter()
            .filter(|run| run.session_id() == session.id())
            .filter(should_pump_background_provider_run)
            .map(|run| run.id().to_string()),
    );
    let mut pumped_provider_run_ids = Vec::new();
    for provider_run_id in provider_run_ids {
        let agent_id = app
            .providers
            .get_run(&provider_run_id)
            .ok()
            .and_then(|run| run.agent_instance_id().map(str::to_string));
        pumped_provider_run_ids.push(provider_run_id.clone());
        if let Err(error) =
            ProviderOutputPump::new(app).pump_provider_output(ProviderOutputPumpRequest {
                session_id: session.id(),
                provider_run_id: &provider_run_id,
                recipient_attachment_ids: recipient_attachment_ids.clone(),
                initial_liveness_already_checked: false,
            })
        {
            crate::logging::warn_with_fields(
                "daemon.provider_output",
                "background prompt pump failed",
                serde_json::json!({
                    "session_id": session.id(),
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "error": error.to_string(),
                }),
            );
        }
    }
    pumped_provider_run_ids
}

#[cfg(test)]
mod tests {
    use super::*;

    fn structured_provider_test_app() -> (DaemonApp, String, String, String) {
        let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-structured-poll",
                "worktree-structured-poll",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-structured-poll",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "opencode",
            "opencode",
            "default",
            "zen",
        )
        .with_agent_id(agent.id());
        let mut run = crate::provider::RuntimeProviderRun::new(
            "provider-run-structured-poll",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::External,
                process_label: "test-opencode-structured-poll".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: Some("test-opencode-runtime".to_string()),
            },
        );
        run.mark_running();
        app.providers_mut().insert_run_for_test(run.clone());
        app.sessions
            .set_active_provider_run(session.id(), Some(run.id().to_string()))
            .expect("active provider run should be set");
        app.update_provider_run_projection(run.clone());
        (
            app,
            session.id().to_string(),
            attachment.id().to_string(),
            run.id().to_string(),
        )
    }

    fn pump_structured_test_run(
        app: &mut DaemonApp,
        session_id: &str,
        attachment_id: &str,
        provider_run_id: &str,
    ) {
        let recipients = app.attachments.list_session_attachment_ids(session_id);
        ProviderOutputPump::new(app)
            .pump_provider_output(ProviderOutputPumpRequest {
                session_id,
                provider_run_id,
                recipient_attachment_ids: recipients,
                initial_liveness_already_checked: false,
            })
            .expect("structured provider output pump should succeed");
        let _ = attachment_id;
    }

    #[test]
    fn pump_active_prompt_outputs_ignores_projected_remote_active_run() {
        let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-1",
                "worktree-1",
            ))
            .expect("session should be created");
        app.sessions
            .set_active_provider_run(
                session.id(),
                Some("remote-projected-provider-run-1".to_string()),
            )
            .expect("active provider run should be recorded");

        let pumped = pump_active_prompt_outputs(&mut app);

        assert!(
            pumped.is_empty(),
            "projected remote provider runs are not local PTY pump targets"
        );
    }

    #[test]
    fn pump_active_prompt_outputs_skips_idle_running_arroba_provider_run() {
        let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-1",
                "worktree-1",
            ))
            .expect("session should be created");
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "opencode",
            "opencode",
            "default",
            "zen",
        )
        .with_agent_id(agent.id());
        let mut run = crate::provider::RuntimeProviderRun::new(
            "provider-run-idle",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::External,
                process_label: "test-opencode-idle".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: Some("test-opencode-runtime".to_string()),
            },
        );
        run.mark_running();
        app.providers_mut().insert_run_for_test(run.clone());
        app.sessions
            .set_active_provider_run(session.id(), Some(run.id().to_string()))
            .expect("active provider run should be set");

        let pumped = pump_active_prompt_outputs(&mut app);

        assert!(
            pumped.is_empty(),
            "idle running Arroba provider runs should not keep the background pump active"
        );
    }

    #[test]
    fn legacy_pump_reaps_inactive_provider_turn() {
        let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-1",
                "worktree-1",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-legacy-inactivity-timeout",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "opencode",
            "opencode",
            "default",
            "zen",
        )
        .with_agent_id(agent.id());
        let mut run = crate::provider::RuntimeProviderRun::new(
            "provider-run-legacy-inactivity-timeout",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::External,
                process_label: "test-opencode-timeout".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: Some("test-opencode-runtime".to_string()),
            },
        );
        run.mark_running();
        app.providers_mut().insert_run_for_test(run.clone());
        app.sessions
            .set_active_provider_run(session.id(), Some(run.id().to_string()))
            .expect("active provider run should be set");
        app.update_provider_run_projection(run.clone());
        let prompt = crate::session::PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            "start, emit a tool, then stall\n",
            crate::session::PromptStatus::Queued,
        );
        app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
            .expect("prompt should start");
        crate::transport::flow_control::note_prompt_started(&mut app, run.id());
        crate::transport::flow_control::note_prompt_response_content(&mut app, run.id());
        app.active_turns.mark_streaming(run.id());
        if let Some(state) = app.prompt_activity.write().get_mut(run.id()) {
            state.last_output_at =
                Some(std::time::Instant::now() - std::time::Duration::from_secs(11 * 60));
            state.saw_response_content = true;
        } else {
            panic!("prompt activity should exist for the active run");
        }

        let _ = pump_terminal_output_for_attachment(&mut app, session.id(), attachment.id())
            .expect("legacy provider output pump should reap inactive provider turn");

        let session = app
            .sessions
            .get_session(session.id())
            .expect("session should still exist");
        assert!(
            session.active_prompt_for_agent(agent.id()).is_none(),
            "legacy inactivity timeout must close the active prompt"
        );
        let run = app
            .providers
            .get_run(run.id())
            .expect("provider run should still exist");
        assert!(run
            .terminal_diagnostic()
            .expect("timeout diagnostic should be recorded")
            .contains("Provider prompt produced no output"));
    }

    #[test]
    fn app_side_structured_pump_defers_empty_poll_reenqueue() {
        let (mut app, session_id, attachment_id, provider_run_id) = structured_provider_test_app();
        app.providers_mut()
            .push_finished_structured_output_poll_for_test(provider_run_id.clone(), Ok(None));

        pump_structured_test_run(&mut app, &session_id, &attachment_id, &provider_run_id);

        let store = app.structured_output_record_store();
        let first_due_at = store
            .poll_due_at_ms(&provider_run_id)
            .expect("empty poll should schedule a next due time");
        assert!(
            !store.poll_due(&provider_run_id, crate::session::unix_epoch_ms()),
            "empty poll should back off instead of immediately re-enqueueing"
        );

        pump_structured_test_run(&mut app, &session_id, &attachment_id, &provider_run_id);

        assert_eq!(
            store.poll_due_at_ms(&provider_run_id),
            Some(first_due_at),
            "second app-side pump before due time must not alter the poll schedule"
        );
    }

    #[test]
    fn metadata_only_structured_batch_backs_off_polling() {
        let (mut app, session_id, attachment_id, provider_run_id) = structured_provider_test_app();
        app.providers_mut()
            .push_finished_structured_output_poll_for_test(
                provider_run_id.clone(),
                Ok(Some(crate::provider::ProviderPromptSignalBatch {
                    resolved_model: Some("resolved-zen".to_string()),
                    resolved_variant: Some("plan".to_string()),
                    resolved_usage_tokens_total: Some(42),
                    ..crate::provider::ProviderPromptSignalBatch::default()
                })),
            );

        pump_structured_test_run(&mut app, &session_id, &attachment_id, &provider_run_id);

        let store = app.structured_output_record_store();
        assert!(
            !store.poll_due(&provider_run_id, crate::session::unix_epoch_ms()),
            "metadata-only updates should not trigger immediate re-polling"
        );
        let run = app
            .providers
            .get_run(&provider_run_id)
            .expect("provider run should still exist");
        assert_eq!(run.model(), "resolved-zen");
        assert_eq!(run.variant(), Some("plan"));
        assert_eq!(run.usage_tokens_total(), Some(42));
    }

    #[test]
    fn structured_output_record_store_clear_removes_records_and_schedule() {
        let store = StructuredOutputRecordStore::default();
        store.schedule_next_poll("provider-run-1".to_string(), 1_500);
        store.append(
            "provider-run-1".to_string(),
            vec![TerminalOutputRecord {
                record_id: None,
                timestamp_ms: 1_000,
                session_id: "session-1".to_string(),
                provider_run_id: "provider-run-1".to_string(),
                agent_id: None,
                prompt_id: None,
                source_attachment_id: None,
                kind: TerminalOutputKind::ProviderOutput,
                merge_key: None,
                recipient_attachment_ids: Vec::new(),
                bytes: b"pending".to_vec(),
                pending_recipient_attachment_ids: Vec::new(),
                external_observation_metadata: None,
            }],
        );

        store.clear("provider-run-1");

        assert_eq!(store.poll_due_at_ms("provider-run-1"), None);
        assert!(store.take("provider-run-1").is_empty());
    }
}

fn should_pump_background_provider_run(run: &RuntimeProviderRun) -> bool {
    crate::provider::provider_run_uses_claude_native_bridge(run)
        && matches!(
            run.state(),
            ProviderRunState::Starting | ProviderRunState::Running
        )
}

fn provider_run_requires_background_pump(
    app: &DaemonApp,
    session: &crate::session::RuntimeSession,
    run: &RuntimeProviderRun,
) -> bool {
    if run.state() == ProviderRunState::Starting || should_pump_background_provider_run(run) {
        return true;
    }
    run.agent_instance_id().is_some_and(|agent_id| {
        app.prompt_state_owner
            .active_prompt_for_agent_snapshot(session, agent_id)
            .is_some()
    })
}

pub(crate) struct ProviderOutputPump<'a> {
    context: ProviderOutputPumpContext<'a>,
}

impl<'a> ProviderOutputPump<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self {
            context: ProviderOutputPumpContext::new(app),
        }
    }

    pub(crate) fn pump_provider_output(
        &mut self,
        request: ProviderOutputPumpRequest<'_>,
    ) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
        self.context.reap_structured_prompt_jobs();
        self.context
            .reap_provider_first_output_timeouts(request.session_id)?;
        self.context
            .reap_provider_inactivity_timeouts(request.session_id)?;
        if !request.initial_liveness_already_checked
            && self
                .context
                .reconcile_provider_run_exit(request.session_id, request.provider_run_id)?
        {
            self.context
                .pending_structured_output_records
                .clear(request.provider_run_id);
            return Ok(Vec::new());
        }
        let mut provider_run = self
            .context
            .ensure_provider_run_in_session(request.session_id, request.provider_run_id)?;
        if provider_run.state() == ProviderRunState::Ended {
            self.context
                .pending_structured_output_records
                .clear(request.provider_run_id);
            return Ok(Vec::new());
        }
        if provider_run.state() == ProviderRunState::Parked {
            if !self
                .context
                .provider_run_has_active_prompt(request.session_id, &provider_run)?
            {
                return Ok(Vec::new());
            }
            provider_run = self
                .context
                .resume_detached_provider_run(request.provider_run_id)?;
            crate::logging::warn_with_fields(
                "daemon.provider_output",
                "resumed parked provider run that still had an active prompt",
                serde_json::json!({
                    "session_id": request.session_id,
                    "provider_run_id": request.provider_run_id,
                    "agent_id": provider_run.agent_instance_id(),
                }),
            );
        }

        if self.context.run_uses_structured_prompt_io(&provider_run) {
            return self.context.pump_structured_output(
                request.session_id,
                request.provider_run_id,
                request.recipient_attachment_ids,
            );
        }
        if crate::provider::provider_run_uses_claude_native_bridge(&provider_run) {
            self.context.process_claude_native_tui_bridge(
                request.session_id,
                request.provider_run_id,
                &provider_run,
            )?;
        }

        let chunks = match self.context.drain_pty_output(request.provider_run_id) {
            Ok(chunks) => chunks,
            Err(error) => {
                if self
                    .context
                    .reconcile_provider_run_exit(request.session_id, request.provider_run_id)?
                {
                    self.context
                        .pending_structured_output_records
                        .clear(request.provider_run_id);
                    return Ok(Vec::new());
                }
                return Err(error);
            }
        };
        if crate::provider::provider_run_uses_claude_native_bridge(&provider_run) {
            let rendered = chunks
                .iter()
                .map(|chunk| String::from_utf8_lossy(&chunk.bytes))
                .collect::<String>();
            self.context.process_claude_native_terminal_output_bridge(
                request.session_id,
                request.provider_run_id,
                &provider_run,
                &rendered,
            )?;
        }
        let terminal_failure = classify_provider_terminal_failure_text(
            provider_run.adapter_key(),
            &chunks
                .iter()
                .map(|chunk| String::from_utf8_lossy(&chunk.bytes))
                .collect::<String>(),
        );
        if !chunks.is_empty() {
            if crate::provider::provider_run_is_claude_headless(&provider_run) {
                self.context.note_prompt_output(request.provider_run_id);
            } else {
                self.context
                    .note_prompt_response_content(request.provider_run_id);
            }
        }

        let records = if crate::provider::provider_run_is_claude_headless(&provider_run) {
            Vec::new()
        } else {
            chunks
                .into_iter()
                .map(|chunk| {
                    self.context.fan_out_provider_output(
                        request.session_id,
                        request.provider_run_id,
                        request.recipient_attachment_ids.clone(),
                        &chunk.bytes,
                    )
                })
                .collect::<Vec<_>>()
        };
        if let Some(message) = terminal_failure {
            let run = self
                .context
                .provider_store
                .record_terminal_diagnostic(request.provider_run_id, message.clone())?;
            self.context.app.update_provider_run_projection(run);
            self.context.fail_prompt_for_terminal_failure(
                request.session_id,
                request.provider_run_id,
                &message,
            )?;
            return Ok(records);
        }
        self.context
            .reconcile_provider_run_exit(request.session_id, request.provider_run_id)?;
        if records.is_empty() {
            self.context
                .settle_pty_prompt_if_quiet(request.session_id, request.provider_run_id)?;
        }

        Ok(records)
    }
}

struct ProviderOutputPumpContext<'a> {
    app: &'a mut DaemonApp,
    provider_store: ProviderProcessServiceStore,
    pending_structured_output_records: StructuredOutputRecordStore,
    active_turns: crate::app::ActiveTurnStore,
    prompt_activity: PromptActivityStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
}

struct ProviderOutputRecipientResolver<'a> {
    app: &'a DaemonApp,
}

impl<'a> ProviderOutputRecipientResolver<'a> {
    fn new(app: &'a DaemonApp) -> Self {
        Self { app }
    }

    fn session_attachment_ids(&self, session_id: &str) -> Vec<String> {
        self.app.attachments.list_session_attachment_ids(session_id)
    }
}

struct ProviderOutputLiveness<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> ProviderOutputLiveness<'a> {
    fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    fn reconcile_exit(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        super::provider_runtime::ProviderRunLivenessRuntime::new(self.app)
            .reconcile_provider_run_exit(session_id, provider_run_id)
    }
}

struct ProviderOutputPtyDrain<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> ProviderOutputPtyDrain<'a> {
    fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    fn drain_output(&mut self, provider_run_id: &str) -> Result<Vec<PtyOutputChunk>, DaemonError> {
        self.app.pty.drain_output(provider_run_id)
    }
}

struct ProviderOutputStructuredPromptReaper<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> ProviderOutputStructuredPromptReaper<'a> {
    fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    fn reap(&mut self) {
        self.app.reap_structured_prompt_jobs();
    }
}

impl<'a> ProviderOutputPumpContext<'a> {
    fn new(app: &'a mut DaemonApp) -> Self {
        Self {
            provider_store: app.providers.clone(),
            pending_structured_output_records: app.pending_structured_output_records.clone(),
            active_turns: app.active_turns.clone(),
            prompt_activity: app.prompt_activity.clone(),
            agent_runtime_projection: app.agent_runtime_projection_store(),
            app,
        }
    }

    fn reap_structured_prompt_jobs(&mut self) {
        reap_structured_prompt_jobs(self.app);
    }

    fn reap_provider_first_output_timeouts(&mut self, session_id: &str) -> Result<(), DaemonError> {
        reap_provider_first_output_timeouts(self.app, session_id)
    }

    fn reap_provider_inactivity_timeouts(&mut self, session_id: &str) -> Result<(), DaemonError> {
        reap_provider_inactivity_timeouts(self.app, session_id)
    }

    fn reconcile_provider_run_exit(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        ProviderOutputLiveness::new(self.app).reconcile_exit(session_id, provider_run_id)
    }

    fn ensure_provider_run_in_session(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let provider_run = self.provider_store.get_run(provider_run_id)?;
        if provider_run.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: provider_run_id.to_string(),
            });
        }
        Ok(provider_run)
    }

    fn run_uses_structured_prompt_io(&self, provider_run: &RuntimeProviderRun) -> bool {
        self.provider_store
            .run_uses_structured_prompt_io(provider_run)
    }

    fn provider_run_has_active_prompt(
        &self,
        session_id: &str,
        provider_run: &RuntimeProviderRun,
    ) -> Result<bool, DaemonError> {
        self.app
            .provider_run_has_active_prompt(session_id, provider_run)
    }

    fn resume_detached_provider_run(
        &mut self,
        provider_run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let run = self.provider_store.resume_run_detached(provider_run_id)?;
        self.app.update_provider_run_projection(run.clone());
        Ok(run)
    }

    fn pump_structured_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
    ) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
        let mut provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
        if provider_run.state() == ProviderRunState::Parked {
            if !self.provider_run_has_active_prompt(session_id, &provider_run)? {
                return Ok(Vec::new());
            }
            provider_run = self.resume_detached_provider_run(provider_run_id)?;
        }
        if provider_run.endpoint_mode() != AgentEndpointMode::External {
            if let Err(error) = self.drain_pty_output(provider_run_id) {
                if self.reconcile_provider_run_exit(session_id, provider_run_id)? {
                    self.pending_structured_output_records
                        .clear(provider_run_id);
                    return Ok(Vec::new());
                }
                if !matches!(error, DaemonError::PtyProcessNotFound { .. }) {
                    return Err(error);
                }
            }
        }
        let mut records = self.pending_structured_output_records.take(provider_run_id);
        records.extend(self.drain_finished_structured_output_jobs_for_run(
            session_id,
            provider_run_id,
            recipient_attachment_ids.clone(),
        )?);
        if self
            .pending_structured_output_records
            .poll_due(provider_run_id, crate::session::unix_epoch_ms())
        {
            match self
                .provider_store
                .enqueue_structured_output_poll(provider_run_id)?
            {
                true => self
                    .pending_structured_output_records
                    .mark_poll_enqueued(provider_run_id),
                false => self
                    .pending_structured_output_records
                    .schedule_after_empty_poll(
                        provider_run_id.to_string(),
                        crate::session::unix_epoch_ms(),
                    ),
            }
        }
        Ok(records)
    }

    fn drain_finished_structured_output_jobs_for_run(
        &mut self,
        requested_session_id: &str,
        requested_provider_run_id: &str,
        requested_recipient_attachment_ids: Vec<String>,
    ) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
        let mut requested_records = Vec::new();
        for finished in self
            .provider_store
            .drain_finished_structured_output_poll_jobs()
        {
            let provider_run_id = finished.provider_run_id.clone();
            let is_requested_run = provider_run_id == requested_provider_run_id;
            let now_ms = crate::session::unix_epoch_ms();
            let poll_result = match finished.result {
                Ok(Some(poll_result)) => poll_result,
                Ok(None) => {
                    self.pending_structured_output_records
                        .schedule_after_empty_poll(provider_run_id, now_ms);
                    continue;
                }
                Err(error) => {
                    let reconcile_result = if is_requested_run {
                        self.reconcile_provider_run_exit(
                            requested_session_id,
                            requested_provider_run_id,
                        )
                    } else {
                        self.provider_store
                            .get_run(&provider_run_id)
                            .and_then(|run| {
                                let session_id = run.session_id().to_string();
                                self.reconcile_provider_run_exit(&session_id, &provider_run_id)
                            })
                    };
                    match reconcile_result {
                        Ok(true) => {
                            self.pending_structured_output_records
                                .clear(&provider_run_id);
                            continue;
                        }
                        Ok(false) if is_requested_run => return Err(error),
                        Ok(false) => {
                            self.pending_structured_output_records
                                .schedule_after_empty_poll(provider_run_id.clone(), now_ms);
                            crate::logging::error_with_fields(
                                "daemon.app",
                                "background structured output poll failed",
                                serde_json::json!({
                                    "provider_run_id": provider_run_id,
                                    "error": error.to_string(),
                                }),
                            );
                            continue;
                        }
                        Err(reconcile_error) if is_requested_run => return Err(reconcile_error),
                        Err(reconcile_error) => {
                            self.pending_structured_output_records
                                .schedule_after_empty_poll(provider_run_id.clone(), now_ms);
                            crate::logging::error_with_fields(
                                "daemon.app",
                                "background structured output poll reconciliation failed",
                                serde_json::json!({
                                    "provider_run_id": provider_run_id,
                                    "error": reconcile_error.to_string(),
                                }),
                            );
                            continue;
                        }
                    }
                }
            };
            let provider_run = match self.provider_store.get_run(&provider_run_id) {
                Ok(run) => run,
                Err(_) => {
                    self.pending_structured_output_records
                        .clear(&provider_run_id);
                    continue;
                }
            };
            let session_id = provider_run.session_id().to_string();
            let recipient_attachment_ids = if is_requested_run {
                requested_recipient_attachment_ids.clone()
            } else {
                self.recipient_attachment_ids_for_session(&session_id)
            };
            let next_poll_due_at_ms =
                if structured_output_batch_should_poll_immediately(&poll_result) {
                    now_ms
                } else {
                    now_ms.saturating_add(STRUCTURED_OUTPUT_EMPTY_POLL_BACKOFF_MS)
                };
            let records = self.apply_structured_output_batch(
                &session_id,
                &provider_run_id,
                recipient_attachment_ids,
                poll_result,
            )?;
            if is_requested_run {
                requested_records.extend(records);
            } else {
                self.pending_structured_output_records
                    .append(provider_run_id.clone(), records);
            }
            self.pending_structured_output_records
                .schedule_next_poll(provider_run_id, next_poll_due_at_ms);
        }
        Ok(requested_records)
    }

    fn apply_structured_output_batch(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        poll_result: ProviderPromptSignalBatch,
    ) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
        self.trace_structured_poll_batch(
            session_id,
            provider_run_id,
            "structured_poll_batch_received",
            &poll_result,
        );
        self.provider_store
            .apply_structured_output_metadata(provider_run_id, &poll_result)?;
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
        self.persist_resolved_resume_state(&provider_run, &poll_result)?;
        self.mark_resolved_external_provider_session_attached(&provider_run);
        self.app
            .update_provider_run_projection(provider_run.clone());
        let terminal_sink = ProviderOutputFanout::new(self.app);
        for notice in &poll_result.notices {
            terminal_sink.record_notice(
                session_id,
                Some(provider_run_id),
                recipient_attachment_ids.clone(),
                notice.to_string(),
            );
        }
        let saw_response_content = poll_result.chunks.iter().any(|chunk| {
            matches!(
                chunk.kind,
                TerminalOutputKind::ProviderOutput | TerminalOutputKind::ProviderReasoning
            )
        });
        let saw_runtime_activity = poll_result.chunks.iter().any(|chunk| {
            matches!(
                chunk.kind,
                TerminalOutputKind::ProviderOutput
                    | TerminalOutputKind::ProviderReasoning
                    | TerminalOutputKind::ProviderTool
                    | TerminalOutputKind::ProviderStatus
            )
        });
        let saw_settlement_blocking_activity = poll_result.chunks.iter().any(|chunk| {
            matches!(
                chunk.kind,
                TerminalOutputKind::ProviderOutput
                    | TerminalOutputKind::ProviderReasoning
                    | TerminalOutputKind::ProviderTool
            )
        });
        if saw_response_content {
            self.note_prompt_response_content(provider_run_id);
        } else if saw_runtime_activity {
            self.note_prompt_output(provider_run_id);
        }
        for completion in &poll_result.completions {
            terminal_sink.record_assistant_message_completion(
                session_id,
                provider_run_id,
                recipient_attachment_ids.clone(),
                &completion.message_id,
                completion.completed_at_ms,
            );
            self.mark_prompt_completion_recorded(provider_run_id);
        }
        let prompt_completed = poll_result.prompt_completed;
        let terminal_failure = poll_result.terminal_failure.clone();
        if let Some(message) = terminal_failure.as_deref() {
            let run = self
                .provider_store
                .record_terminal_diagnostic(provider_run_id, message.to_string())?;
            self.app.update_provider_run_projection(run);
        }
        let records: Vec<TerminalOutputRecord> = poll_result
            .chunks
            .into_iter()
            .filter_map(|chunk| {
                let record = self.fan_out_terminal_output(
                    session_id,
                    provider_run_id,
                    chunk.kind,
                    chunk.merge_key,
                    recipient_attachment_ids.clone(),
                    &chunk.bytes,
                );
                if record.pending_recipient_attachment_ids.is_empty() && record.bytes.is_empty() {
                    None
                } else {
                    Some(record)
                }
            })
            .collect();
        self.trace_terminal_records(
            session_id,
            provider_run_id,
            "structured_poll_records_fanned_out",
            &records,
        );
        let exited = self.reconcile_provider_run_exit(session_id, provider_run_id)?;
        if exited {
            self.trace_prompt_state(
                session_id,
                provider_run_id,
                "structured_poll_provider_exited",
            );
            return Ok(records);
        }
        if let Some(message) = terminal_failure {
            self.fail_prompt_for_terminal_failure(session_id, provider_run_id, &message)?;
            self.trace_prompt_state(
                session_id,
                provider_run_id,
                "structured_poll_terminal_failure_settled",
            );
            return Ok(records);
        }
        let should_trace_settlement =
            prompt_completed || saw_settlement_blocking_activity || !records.is_empty();
        if should_trace_settlement {
            self.trace_prompt_state(
                session_id,
                provider_run_id,
                "structured_poll_before_settlement",
            );
        }
        let settlement = self.settle_structured_prompt_completion(
            session_id,
            provider_run_id,
            prompt_completed,
            saw_settlement_blocking_activity,
        );
        if should_trace_settlement || settlement.is_err() {
            self.trace_prompt_state(
                session_id,
                provider_run_id,
                if settlement.is_ok() {
                    "structured_poll_after_settlement"
                } else {
                    "structured_poll_settlement_error"
                },
            );
        }
        settlement?;
        Ok(records)
    }

    fn persist_resolved_resume_state(
        &mut self,
        provider_run: &RuntimeProviderRun,
        poll_result: &ProviderPromptSignalBatch,
    ) -> Result<(), DaemonError> {
        let Some(resume_state) = poll_result.resolved_resume_state.as_ref() else {
            return Ok(());
        };
        let Some(agent_id) = provider_run.agent_instance_id() else {
            return Ok(());
        };
        let agent = self.app.agents.set_agent_runtime_profile(
            agent_id,
            provider_run.provider(),
            Some(provider_run.model().to_string()),
            provider_run.variant().map(str::to_string),
            resume_state.clone(),
        )?;
        self.app.durable_state_store().append_event(
            "agent.runtime_profile_updated",
            Some(agent.id().to_string()),
            serde_json::json!({
                "agent": &agent,
                "provider_run_id": provider_run.id(),
            }),
        )?;
        let _ = crate::app::KernelSessionReadService::new(self.app)
            .session_snapshot(provider_run.session_id())?;
        Ok(())
    }

    fn mark_resolved_external_provider_session_attached(&self, provider_run: &RuntimeProviderRun) {
        let Some(agent_id) = provider_run.agent_instance_id() else {
            return;
        };
        self.app
            .external_provider_sessions
            .mark_provider_run_attached(
                provider_run.adapter_key(),
                provider_run.provider_session_id(),
                provider_run.resume_state(),
                provider_run.session_id(),
                agent_id,
            );
    }

    fn trace_structured_poll_batch(
        &self,
        session_id: &str,
        provider_run_id: &str,
        source: &str,
        poll_result: &ProviderPromptSignalBatch,
    ) {
        self.trace()
            .structured_poll_batch(session_id, provider_run_id, source, poll_result);
    }

    fn trace_terminal_records(
        &self,
        session_id: &str,
        provider_run_id: &str,
        source: &str,
        records: &[TerminalOutputRecord],
    ) {
        self.trace()
            .terminal_records(session_id, provider_run_id, source, records);
    }

    fn trace_prompt_state(&self, session_id: &str, provider_run_id: &str, source: &str) {
        self.trace()
            .prompt_state_turn(session_id, provider_run_id, source);
    }

    fn trace(&self) -> ProviderOutputTrace {
        ProviderOutputTrace::new(
            self.app,
            self.provider_store.clone(),
            self.active_turns.clone(),
            self.prompt_activity.clone(),
        )
    }

    fn drain_pty_output(
        &mut self,
        provider_run_id: &str,
    ) -> Result<Vec<PtyOutputChunk>, DaemonError> {
        ProviderOutputPtyDrain::new(self.app).drain_output(provider_run_id)
    }

    fn process_claude_native_tui_bridge(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        provider_run: &RuntimeProviderRun,
    ) -> Result<(), DaemonError> {
        // The interactive TUI pump revisits transcripts on its own cadence, so
        // the deferred-drain hint is not needed on this path.
        ProviderOutputClaudeNativeBridge::new(self.app)
            .process(
                session_id,
                provider_run_id,
                provider_run,
                self.provider_store.native_interaction_bridge(),
            )
            .map(|_| ())
    }

    fn process_claude_native_terminal_output_bridge(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        provider_run: &RuntimeProviderRun,
        rendered: &str,
    ) -> Result<(), DaemonError> {
        ProviderOutputClaudeNativeBridge::new(self.app).process_terminal_output(
            session_id,
            provider_run_id,
            provider_run,
            self.provider_store.native_interaction_bridge(),
            rendered,
        )
    }

    fn recipient_attachment_ids_for_session(&self, session_id: &str) -> Vec<String> {
        ProviderOutputRecipientResolver::new(self.app).session_attachment_ids(session_id)
    }

    fn note_prompt_output(&mut self, provider_run_id: &str) {
        crate::transport::flow_control::note_prompt_output(self.app, provider_run_id);
    }

    fn note_prompt_response_content(&mut self, provider_run_id: &str) {
        crate::transport::flow_control::note_prompt_response_content(self.app, provider_run_id);
    }

    fn mark_prompt_completion_recorded(&self, provider_run_id: &str) {
        if let Some(state) = self.prompt_activity.write().get_mut(provider_run_id) {
            state.completion_recorded = true;
        }
    }

    fn settle_structured_prompt_completion(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        prompt_completed: bool,
        saw_settlement_blocking_activity: bool,
    ) -> Result<(), DaemonError> {
        self.prompt_settlement().settle_structured_completion(
            session_id,
            provider_run_id,
            prompt_completed,
            saw_settlement_blocking_activity,
        )
    }

    fn settle_pty_prompt_if_quiet(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<(), DaemonError> {
        self.prompt_settlement()
            .settle_pty_if_quiet(session_id, provider_run_id)
    }

    fn fail_prompt_for_terminal_failure(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        message: &str,
    ) -> Result<(), DaemonError> {
        self.prompt_settlement()
            .fail_for_terminal_failure(session_id, provider_run_id, message)
    }

    fn prompt_settlement(&mut self) -> ProviderOutputPromptSettlement<'_> {
        ProviderOutputPromptSettlement::new(
            self.app,
            self.provider_store.clone(),
            self.active_turns.clone(),
            self.prompt_activity.clone(),
            self.agent_runtime_projection.clone(),
        )
    }

    fn fan_out_provider_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        self.fan_out_terminal_output(
            session_id,
            provider_run_id,
            TerminalOutputKind::ProviderOutput,
            None,
            recipient_attachment_ids,
            bytes,
        )
    }

    fn fan_out_terminal_output(
        &self,
        session_id: &str,
        provider_run_id: &str,
        kind: TerminalOutputKind,
        merge_key: Option<String>,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        ProviderOutputFanout::new(self.app).fan_out(
            session_id,
            provider_run_id,
            kind,
            merge_key,
            recipient_attachment_ids,
            bytes,
        )
    }
}

impl DaemonApp {
    pub(crate) fn process_claude_native_bridge_for_runtime(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        provider_run: &RuntimeProviderRun,
    ) -> Result<crate::app::ClaudeNativeProcessOutcome, DaemonError> {
        let native_interaction_bridge = self.providers.native_interaction_bridge();
        ProviderOutputClaudeNativeBridge::new(self).process(
            session_id,
            provider_run_id,
            provider_run,
            native_interaction_bridge,
        )
    }

    pub(crate) fn drain_claude_native_headless_transcripts_for_runtime(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        provider_run: &RuntimeProviderRun,
    ) -> Result<(), DaemonError> {
        ProviderOutputClaudeNativeBridge::new(self).drain_headless_transcripts_for_context(
            session_id,
            provider_run_id,
            provider_run,
        )
    }

    pub(crate) fn process_claude_native_prompt_dispatch_attempt_for_runtime(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        provider_run: &RuntimeProviderRun,
        dispatch: &crate::app::KernelPromptDispatch,
    ) -> Result<crate::app::ClaudeNativeDispatchAttempt, DaemonError> {
        ProviderOutputClaudeNativeBridge::new(self).process_prompt_dispatch_attempt(
            session_id,
            provider_run_id,
            provider_run,
            dispatch,
        )
    }

    pub(crate) fn process_claude_native_terminal_output_bridge_for_runtime(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        provider_run: &RuntimeProviderRun,
        rendered: &str,
    ) -> Result<(), DaemonError> {
        let native_interaction_bridge = self.providers.native_interaction_bridge();
        ProviderOutputClaudeNativeBridge::new(self).process_terminal_output(
            session_id,
            provider_run_id,
            provider_run,
            native_interaction_bridge,
            rendered,
        )
    }
}
