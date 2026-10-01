//! Per-turn substitutes: a turn whose provider fails reruns on the agent's next
//! substitute, and the next turn starts on the agent's configured profile.

use super::*;
use crate::provider::{ProviderRunState, ProviderRunTermination, TurnSubstitute};

// The deterministic dev-stub provider launches a local `cat` process, so the
// substitute's real launch path runs without provider credentials.
const PRIMARY_MODEL: &str = "primary-model";
const SUBSTITUTE_A: &str = "substitute-a";
const SUBSTITUTE_B: &str = "substitute-b";
const SERVER_OVERLOADED: &str =
    "Provider prompt dispatch failed: Codex error [server_overloaded]: \
     Selected model is at capacity. Please try a different model.";

struct FailingTurn {
    runtime: KernelRuntimeState,
    session_id: String,
    agent_id: String,
    failed_run_id: String,
    prompt_id: String,
    worktree: std::path::PathBuf,
}

impl Drop for FailingTurn {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.worktree);
    }
}

impl FailingTurn {
    fn active_prompt(&self) -> Option<crate::session::PromptQueueItem> {
        let session = self
            .runtime
            .owned
            .session_store
            .get_session(&self.session_id)
            .unwrap();
        self.runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &self.agent_id)
    }

    fn live_run(&self) -> Option<crate::provider::RuntimeProviderRun> {
        self.runtime
            .owned
            .provider_store
            .get_run_for_agent(&self.session_id, &self.agent_id)
    }

    fn notices(&self) -> Vec<String> {
        self.runtime
            .owned
            .terminal_stream
            .notice_records()
            .into_iter()
            .map(|notice| notice.message)
            .collect()
    }

    fn rerun_notices(&self) -> Vec<String> {
        self.notices()
            .into_iter()
            .filter(|notice| notice.starts_with("This turn runs on "))
            .collect()
    }

    async fn fail_run(&self, provider_run_id: &str, message: &str) {
        self.runtime
            .fail_owned_provider_prompt(&self.session_id, provider_run_id, message, true)
            .await
            .expect("provider failure should settle or rerun the turn");
    }

    /// The turn is still the same active prompt, now delivered to a live
    /// substitute run, on substitute `substitute_index` of the current list,
    /// that serves only it.
    fn assert_rerun_on(&self, substitute_index: usize, model: &str) -> String {
        let prompt = self
            .active_prompt()
            .expect("a rerun keeps the failed turn active");
        assert_eq!(prompt.id(), self.prompt_id, "the same turn is rerun");
        let run = self.live_run().expect("the substitute run is live");
        assert_eq!(run.model(), model);
        let agent = self
            .runtime
            .owned
            .agent_store
            .get_agent(&self.agent_id)
            .unwrap();
        let turn = run.turn_substitute().expect("the run is a substitute's");
        assert_eq!(turn.prompt_id, self.prompt_id);
        assert_eq!(
            turn.tried.last(),
            agent.substitutes().get(substitute_index),
            "the run is on substitute {substitute_index}"
        );
        assert_eq!(prompt.durable_delivery_provider_run_id(), Some(run.id()));
        assert_eq!(agent.provider(), "dev-stub");
        assert_eq!(
            agent.model(),
            Some(PRIMARY_MODEL),
            "a substitute never becomes the agent's profile"
        );
        run.id().to_string()
    }

    fn record_history(&self, provider_run_id: Option<&str>, text: &str) {
        let entry = match provider_run_id {
            Some(provider_run_id) => crate::history::SessionHistoryEntry::provider_output(
                &self.session_id,
                provider_run_id,
                Some(&self.agent_id),
                crate::terminal::TerminalOutputKind::ProviderOutput,
                None,
                text,
            ),
            None => crate::history::SessionHistoryEntry::user_prompt(
                &self.session_id,
                "substitute-test",
                &self.agent_id,
                text,
            ),
        };
        self.runtime
            .owned
            .append_operational_history_entry(&entry, None, None, None);
    }

    fn provider_inputs(&self, provider_run_id: &str) -> String {
        self.runtime
            .owned
            .terminal_stream
            .input_records()
            .into_iter()
            .filter(|record| record.provider_run_id == provider_run_id)
            .map(|record| String::from_utf8_lossy(&record.bytes).into_owned())
            .collect()
    }

    fn assert_turn_failed(&self) {
        assert!(self.active_prompt().is_none(), "the turn must fail");
        let settlement = self
            .runtime
            .owned
            .operational_history_store
            .load_prompt_settlement_event(&self.session_id, &self.agent_id, &self.prompt_id)
            .unwrap()
            .expect("the failed turn has durable settlement history");
        assert_eq!(
            settlement
                .metadata
                .get(crate::history::PROMPT_SETTLEMENT_STATUS_METADATA_KEY)
                .and_then(serde_json::Value::as_str),
            Some("failed")
        );
    }
}

async fn failing_turn(substitutes: &[(&str, &str, Option<&str>)]) -> FailingTurn {
    failing_turn_with(substitutes, false).await
}

/// A dev-stub agent on `PRIMARY_MODEL` with `substitutes` (provider, model,
/// account), whose active turn — a workflow node's when `workflow` — runs on a
/// provider run the test drives by hand.
async fn failing_turn_with(
    substitutes: &[(&str, &str, Option<&str>)],
    workflow: bool,
) -> FailingTurn {
    let worktree = std::env::temp_dir().join(format!(
        "chariox-turn-substitute-{:016x}",
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&worktree).unwrap();
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            worktree.to_string_lossy(),
            worktree.to_string_lossy(),
        ))
        .expect("session should be created");
    let (session_id, agent_id) = (session.id().to_string(), agent.id().to_string());
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    for (provider, model, account) in substitutes {
        runtime
            .owned
            .agent_store
            .add_agent_substitute(
                &agent_id,
                crate::agent::AgentSubstituteProfile::new(*provider, *model, None)
                    .with_account_profile(account.map(str::to_string)),
            )
            .expect("configured substitute");
    }
    runtime
        .owned
        .agent_store
        .set_agent_runtime_profile_with_account_profile(
            &agent_id,
            "dev-stub",
            Some(PRIMARY_MODEL.to_string()),
            None,
            None,
            crate::provider::ProviderResumeState::default(),
        )
        .unwrap();
    let request = crate::provider::LaunchProviderRequest::new(
        &session_id,
        "dev-stub",
        "dev-stub",
        "default",
        PRIMARY_MODEL,
    )
    .with_agent_id(&agent_id);
    let mut run = crate::provider::RuntimeProviderRun::new(
        "failed-run",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-primary".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-primary-runtime".to_string()),
        },
    );
    run.mark_running();
    runtime
        .with_app_side_effect(|app| {
            app.providers_mut().insert_run_for_test(run.clone());
            app.sessions_mut()
                .set_active_provider_run(&session_id, Some(run.id().to_string()))?;
            let attachment = crate::app::KernelSessionService::new(app).attach(
                crate::attachment::AttachRequest::new(
                    &session_id,
                    "substitute-test",
                    crate::attachment::ClientCapabilityLevel::FullTerminal,
                ),
            )?;
            let mut prompt = crate::session::PromptQueueItem::new(
                "failed-prompt",
                attachment.id(),
                &agent_id,
                "review this change",
                crate::session::PromptStatus::Queued,
            );
            if workflow {
                let definition = app.sessions_mut().create_workflow(&session_id, None)?;
                let node = app.sessions_mut().add_workflow_node(
                    &session_id,
                    definition.id(),
                    &agent_id,
                )?;
                let endpoint = app.sessions_mut().create_workflow_endpoint(
                    &session_id,
                    definition.id(),
                    node.id(),
                    None,
                )?;
                let workflow_run = app.sessions_mut().invoke_workflow_endpoint(
                    &session_id,
                    definition.id(),
                    endpoint.id(),
                    Some("review this change".to_string()),
                )?;
                prompt = prompt
                    .with_workflow_context(workflow_run.id(), workflow_run.node_runs()[0].id());
            }
            app.prompt_owner_submit_prepared_prompt(&session_id, prompt, false)
        })
        .await
        .unwrap();
    let mut turn = FailingTurn {
        runtime,
        session_id,
        agent_id,
        failed_run_id: run.id().to_string(),
        prompt_id: String::new(),
        worktree,
    };
    // Submission allocates the canonical active prompt id.
    turn.prompt_id = turn
        .active_prompt()
        .expect("the turn should be active")
        .id()
        .to_string();
    turn
}

#[tokio::test]
async fn codex_server_overloaded_reruns_the_turn_on_the_first_substitute() {
    let turn = failing_turn(&[("dev-stub", SUBSTITUTE_A, None)]).await;

    turn.fail_run(&turn.failed_run_id, SERVER_OVERLOADED).await;

    turn.assert_rerun_on(0, SUBSTITUTE_A);
    assert_eq!(
        turn.runtime
            .owned
            .provider_store
            .get_run(&turn.failed_run_id)
            .unwrap()
            .state(),
        ProviderRunState::Ended
    );
    assert_eq!(
        turn.rerun_notices(),
        vec![format!(
            "This turn runs on {SUBSTITUTE_A} because {PRIMARY_MODEL} failed: \
             model at capacity (server_overloaded)."
        )]
    );
    let failure_outputs = turn
        .runtime
        .owned
        .terminal_stream
        .output_records()
        .into_iter()
        .filter(|record| {
            record.provider_run_id == turn.failed_run_id
                && record.kind == crate::terminal::TerminalOutputKind::ProviderError
        })
        .count();
    assert_eq!(failure_outputs, 1, "the failed attempt stays visible once");
}

#[tokio::test]
async fn provider_crash_mid_turn_reruns_the_turn_on_the_substitute() {
    let turn = failing_turn(&[("dev-stub", SUBSTITUTE_A, None)]).await;

    turn.runtime
        .settle_unexpected_provider_run_exit(
            &turn.session_id,
            &turn.failed_run_id,
            &turn.agent_id,
            ProviderRunTermination::process_exit(1, crate::session::unix_epoch_ms()),
        )
        .await
        .expect("an unexpected exit should rerun the turn");

    turn.assert_rerun_on(0, SUBSTITUTE_A);
    assert_eq!(
        turn.rerun_notices(),
        vec![format!(
            "This turn runs on {SUBSTITUTE_A} because {PRIMARY_MODEL} failed: \
             provider process exited with status 1."
        )]
    );
}

#[tokio::test]
async fn a_failing_substitute_hands_the_same_turn_to_the_next_one() {
    let turn = failing_turn(&[
        ("dev-stub", SUBSTITUTE_A, None),
        ("dev-stub", SUBSTITUTE_B, None),
    ])
    .await;

    turn.fail_run(&turn.failed_run_id, SERVER_OVERLOADED).await;
    let first = turn.assert_rerun_on(0, SUBSTITUTE_A);
    turn.fail_run(&first, "OpenCode error: upstream request failed")
        .await;

    turn.assert_rerun_on(1, SUBSTITUTE_B);
    assert_eq!(
        turn.rerun_notices().last().map(String::as_str),
        Some(
            format!(
                "This turn runs on {SUBSTITUTE_B} because {SUBSTITUTE_A} failed: \
                 OpenCode error: upstream request failed."
            )
            .as_str()
        )
    );
}

#[tokio::test]
async fn an_exhausted_chain_fails_the_turn_visibly() {
    let turn = failing_turn(&[("dev-stub", SUBSTITUTE_A, None)]).await;

    turn.fail_run(&turn.failed_run_id, SERVER_OVERLOADED).await;
    let substitute = turn.assert_rerun_on(0, SUBSTITUTE_A);
    turn.fail_run(&substitute, "You have exceeded your usage limit.")
        .await;

    turn.assert_turn_failed();
    assert!(turn.live_run().is_none());
    assert!(turn.notices().contains(&format!(
        "No substitute is left for this turn: {SUBSTITUTE_A} failed: \
         You have exceeded your usage limit."
    )));
    let agent = turn
        .runtime
        .owned
        .agent_store
        .get_agent(&turn.agent_id)
        .unwrap();
    assert_eq!(agent.model(), Some(PRIMARY_MODEL));
}

#[tokio::test]
async fn without_substitutes_the_turn_fails_as_before() {
    let turn = failing_turn(&[]).await;

    turn.fail_run(&turn.failed_run_id, SERVER_OVERLOADED).await;

    turn.assert_turn_failed();
    assert!(turn.rerun_notices().is_empty());
    assert!(turn.live_run().is_none());
    let errors = turn
        .runtime
        .owned
        .terminal_stream
        .output_records()
        .into_iter()
        .filter(|record| record.kind == crate::terminal::TerminalOutputKind::ProviderError)
        .map(|record| String::from_utf8_lossy(&record.bytes).into_owned())
        .collect::<Vec<_>>();
    // The failed turn's request is dropped and marked as such (protocol 384).
    let not_carried_out = crate::agent::failed_request_notice(
        &crate::agent::failed_request_reason("dev-stub", SERVER_OVERLOADED),
    );
    assert_eq!(errors, vec![SERVER_OVERLOADED.to_string(), not_carried_out]);
}

#[tokio::test]
async fn a_user_cancel_is_not_a_provider_failure() {
    let turn = failing_turn(&[("dev-stub", SUBSTITUTE_A, None)]).await;
    let session = turn
        .runtime
        .owned
        .session_store
        .get_session(&turn.session_id)
        .unwrap();
    turn.runtime
        .owned
        .prompt_state_owner
        .begin_cancelling_active_prompt(&session, &turn.agent_id)
        .expect("the turn is cancelling");

    // The provider reports the interrupted turn as an error.
    turn.fail_run(&turn.failed_run_id, SERVER_OVERLOADED).await;

    assert!(turn.rerun_notices().is_empty());
    assert!(turn
        .live_run()
        .is_none_or(|run| run.turn_substitute().is_none()));
}

#[tokio::test]
async fn the_next_turn_starts_on_the_primary_again() {
    let turn = failing_turn(&[("dev-stub", SUBSTITUTE_A, None)]).await;
    turn.fail_run(&turn.failed_run_id, SERVER_OVERLOADED).await;
    let substitute = turn.assert_rerun_on(0, SUBSTITUTE_A);

    turn.runtime
        .settle_owned_provider_prompt(&turn.session_id, &substitute, true, false, true)
        .await
        .expect("the substitute completes the turn");

    assert!(turn.active_prompt().is_none());
    assert_eq!(
        turn.runtime
            .owned
            .provider_store
            .get_run(&substitute)
            .unwrap()
            .state(),
        ProviderRunState::Ended,
        "the substitute served only its turn"
    );
    let next_run = turn
        .runtime
        .with_app_side_effect(|app| {
            app.ensure_prompt_provider_run_for_agent(&turn.session_id, &turn.agent_id)
        })
        .await
        .expect("the next turn launches its provider");
    let next_run = turn
        .runtime
        .owned
        .provider_store
        .get_run(&next_run)
        .unwrap();
    assert_eq!(next_run.model(), PRIMARY_MODEL);
    assert!(next_run.turn_substitute().is_none());
}

#[tokio::test]
async fn a_finished_substitute_run_is_never_reused_for_another_turn() {
    let turn = failing_turn(&[("dev-stub", SUBSTITUTE_A, None)]).await;
    turn.fail_run(&turn.failed_run_id, SERVER_OVERLOADED).await;
    let substitute = turn.assert_rerun_on(0, SUBSTITUTE_A);
    // Settle the turn without the runtime's substitute cleanup.
    turn.runtime
        .owned
        .complete_local_prompt_without_advance(&turn.session_id, &turn.agent_id, Some(&substitute))
        .unwrap();
    assert_eq!(
        turn.live_run().map(|run| run.id().to_string()),
        Some(substitute.clone())
    );

    let next_run = turn
        .runtime
        .with_app_side_effect(|app| {
            app.ensure_prompt_provider_run_for_agent(&turn.session_id, &turn.agent_id)
        })
        .await
        .expect("the next turn launches its provider");

    assert_ne!(next_run, substitute);
    assert_eq!(
        turn.runtime
            .owned
            .provider_store
            .get_run(&next_run)
            .unwrap()
            .model(),
        PRIMARY_MODEL
    );
    assert_eq!(
        turn.runtime
            .owned
            .provider_store
            .get_run(&substitute)
            .unwrap()
            .state(),
        ProviderRunState::Ended
    );
}

#[tokio::test]
async fn unavailable_substitute_accounts_are_skipped_in_order() {
    let turn = failing_turn(&[
        ("opencode", "deepseek-v4-pro", Some("removed-account")),
        ("dev-stub", SUBSTITUTE_B, None),
    ])
    .await;

    turn.fail_run(&turn.failed_run_id, SERVER_OVERLOADED).await;

    turn.assert_rerun_on(1, SUBSTITUTE_B);
    assert!(turn.notices().iter().any(|notice| notice
        .starts_with("Skipping substitute 1 for agent")
        && notice.contains("no longer available")));
}

#[tokio::test]
async fn a_failed_workflow_turn_reruns_without_failing_its_workflow_run() {
    let turn = failing_turn_with(&[("dev-stub", SUBSTITUTE_A, None)], true).await;
    let workflow_run_id = turn
        .active_prompt()
        .and_then(|prompt| prompt.workflow_run_id().map(str::to_string))
        .expect("the turn belongs to a workflow run");

    turn.fail_run(&turn.failed_run_id, SERVER_OVERLOADED).await;

    let substitute = turn.assert_rerun_on(0, SUBSTITUTE_A);
    let substitute = turn
        .runtime
        .owned
        .provider_store
        .get_run(&substitute)
        .unwrap();
    assert!(
        substitute.workflow_tools_enabled(),
        "the rerun launches through the workflow path"
    );
    assert!(
        turn.runtime
            .owned
            .prompt_workspace_claims
            .contains(substitute.id()),
        "the node's worktree claim moves to the substitute run"
    );
    let session = turn
        .runtime
        .owned
        .session_store
        .get_session(&turn.session_id)
        .unwrap();
    let workflow_run = session
        .workflow_runs()
        .iter()
        .find(|run| run.id() == workflow_run_id)
        .expect("the workflow run is still live");
    assert!(workflow_run.failure_events().is_empty());
    assert_ne!(
        workflow_run.status(),
        crate::session::WorkflowRunStatus::Failed
    );
}

#[tokio::test]
async fn a_turn_queued_behind_a_substitute_turn_starts_on_the_primary() {
    let turn = failing_turn(&[("dev-stub", SUBSTITUTE_A, None)]).await;
    turn.fail_run(&turn.failed_run_id, SERVER_OVERLOADED).await;
    let substitute = turn.assert_rerun_on(0, SUBSTITUTE_A);
    let queued = turn
        .runtime
        .with_app_side_effect(|app| {
            let attachment = crate::app::KernelSessionService::new(app).attach(
                crate::attachment::AttachRequest::new(
                    &turn.session_id,
                    "next-turn-client",
                    crate::attachment::ClientCapabilityLevel::FullTerminal,
                ),
            )?;
            app.prompt_owner_submit_prepared_prompt(
                &turn.session_id,
                crate::session::PromptQueueItem::new(
                    "next-prompt",
                    attachment.id(),
                    &turn.agent_id,
                    "next turn",
                    crate::session::PromptStatus::Queued,
                ),
                false,
            )
        })
        .await
        .unwrap();
    assert!(matches!(
        queued,
        crate::session::PromptSubmissionOutcome::Queued { .. }
    ));

    turn.runtime
        .settle_owned_provider_prompt(&turn.session_id, &substitute, true, false, true)
        .await
        .expect("the substitute completes its turn");

    let next = turn.active_prompt().expect("the queued turn starts");
    assert_ne!(next.id(), turn.prompt_id);
    let next_run = next
        .durable_delivery_provider_run_id()
        .expect("the next turn is dispatched");
    let next_run = turn.runtime.owned.provider_store.get_run(next_run).unwrap();
    assert_eq!(next_run.model(), PRIMARY_MODEL);
    assert!(next_run.turn_substitute().is_none());
    assert_eq!(
        turn.runtime
            .owned
            .provider_store
            .get_run(&substitute)
            .unwrap()
            .state(),
        ProviderRunState::Ended
    );
}

/// The substitute receives the conversation it never saw, and the primary's
/// next turn receives the substitute's answer.
async fn assert_conversation_crosses_the_substitute(turn: &FailingTurn, substitute_model: &str) {
    turn.record_history(None, "Which file holds the parser?");
    turn.record_history(Some(&turn.failed_run_id), "The parser lives in parse.rs.");
    turn.record_history(None, "review this change");

    turn.fail_run(&turn.failed_run_id, SERVER_OVERLOADED).await;

    let substitute = turn.assert_rerun_on(0, substitute_model);
    assert!(
        turn.provider_inputs(&substitute)
            .contains("The parser lives in parse.rs."),
        "the substitute receives the conversation it never saw"
    );
    turn.record_history(
        Some(&substitute),
        "Proposed fix: rename parse_all to parse.",
    );
    turn.runtime
        .settle_owned_provider_prompt(&turn.session_id, &substitute, true, false, true)
        .await
        .expect("the substitute completes the turn");
    let next_run = turn
        .runtime
        .with_app_side_effect(|app| {
            app.ensure_prompt_provider_run_for_agent(&turn.session_id, &turn.agent_id)
        })
        .await
        .expect("the next turn launches its provider");
    let next_run = turn
        .runtime
        .owned
        .provider_store
        .get_run(&next_run)
        .unwrap();
    assert_eq!(next_run.model(), PRIMARY_MODEL);
    assert!(next_run.turn_substitute().is_none());

    let next_prompt = turn.runtime.owned.prompt_with_pending_context_handoff(
        &turn.session_id,
        &turn.agent_id,
        "substitute-test",
        &next_run,
        "implement that solution",
    );
    assert!(
        next_prompt.contains("Proposed fix: rename parse_all to parse."),
        "the primary's next turn carries the substitute's answer: {next_prompt}"
    );
}

#[tokio::test]
async fn the_substitute_and_the_next_primary_turn_see_the_conversation() {
    let turn = failing_turn(&[("dev-stub", SUBSTITUTE_A, None)]).await;
    assert_conversation_crosses_the_substitute(&turn, SUBSTITUTE_A).await;
}

#[tokio::test]
async fn a_substitute_differing_only_in_effort_still_hands_the_conversation_over() {
    let turn = failing_turn(&[]).await;
    turn.runtime
        .owned
        .agent_store
        .add_agent_substitute(
            &turn.agent_id,
            crate::agent::AgentSubstituteProfile::new(
                "dev-stub",
                PRIMARY_MODEL,
                Some("low".to_string()),
            ),
        )
        .expect("configured substitute");
    assert_conversation_crosses_the_substitute(&turn, PRIMARY_MODEL).await;
}

#[tokio::test]
async fn a_substitute_on_another_account_never_resumes_the_primary_session() {
    let turn = failing_turn(&[("dev-stub", PRIMARY_MODEL, Some("backup-account"))]).await;
    let primary_session =
        crate::provider::ProviderResumeState::from_codex_thread_id("primary-thread");
    turn.runtime
        .owned
        .agent_store
        .set_agent_runtime_profile_with_account_profile(
            &turn.agent_id,
            "dev-stub",
            Some(PRIMARY_MODEL.to_string()),
            None,
            None,
            primary_session.clone(),
        )
        .unwrap();

    turn.fail_run(&turn.failed_run_id, SERVER_OVERLOADED).await;

    let substitute = turn.assert_rerun_on(0, PRIMARY_MODEL);
    let substitute = turn
        .runtime
        .owned
        .provider_store
        .get_run(&substitute)
        .unwrap();
    assert_eq!(substitute.account_profile(), "backup-account");
    assert!(
        substitute.resume_state().is_empty(),
        "the substitute starts its own provider session"
    );
    assert_eq!(
        turn.runtime
            .owned
            .agent_store
            .get_agent(&turn.agent_id)
            .unwrap()
            .provider_resume_state(),
        &primary_session,
        "the configured profile keeps its session"
    );
}

impl FailingTurn {
    /// Queues "implement that solution" behind the active turn.
    async fn queue_follow_up(&self) {
        self.runtime
            .with_app_side_effect(|app| {
                let attachment = crate::app::KernelSessionService::new(app).attach(
                    crate::attachment::AttachRequest::new(
                        &self.session_id,
                        "follow-up-client",
                        crate::attachment::ClientCapabilityLevel::FullTerminal,
                    ),
                )?;
                app.prompt_owner_submit_prepared_prompt(
                    &self.session_id,
                    crate::session::PromptQueueItem::new(
                        "follow-up",
                        attachment.id(),
                        &self.agent_id,
                        "implement that solution",
                        crate::session::PromptStatus::Queued,
                    ),
                    false,
                )
            })
            .await
            .unwrap();
    }

    /// The queued follow-up runs on a fresh run of the configured profile,
    /// with the substitute's answer, and the substitute run is retired.
    fn assert_follow_up_on_the_primary(&self, substitute: &str) {
        let follow_up = self.active_prompt().expect("the follow-up starts");
        assert_eq!(follow_up.prompt(), "implement that solution");
        let primary_run = follow_up
            .durable_delivery_provider_run_id()
            .expect("the follow-up is dispatched")
            .to_string();
        assert_ne!(primary_run, substitute);
        let primary_run = self
            .runtime
            .owned
            .provider_store
            .get_run(&primary_run)
            .unwrap();
        assert_eq!(primary_run.model(), PRIMARY_MODEL);
        assert!(primary_run.turn_substitute().is_none());
        let delivered = self.provider_inputs(primary_run.id());
        assert!(
            delivered.contains("implement that solution")
                && delivered.contains("Proposed fix: rename parse_all to parse."),
            "the follow-up reaches the primary with the substitute's answer: {delivered}"
        );
        assert_eq!(
            self.runtime
                .owned
                .provider_store
                .get_run(substitute)
                .unwrap()
                .state(),
            ProviderRunState::Ended
        );
    }
}

#[tokio::test]
async fn a_queued_follow_up_reaches_the_primary_with_the_substitute_answer() {
    let turn = failing_turn(&[("dev-stub", SUBSTITUTE_A, None)]).await;
    turn.record_history(None, "review this change");
    turn.fail_run(&turn.failed_run_id, SERVER_OVERLOADED).await;
    let substitute = turn.assert_rerun_on(0, SUBSTITUTE_A);
    turn.record_history(
        Some(&substitute),
        "Proposed fix: rename parse_all to parse.",
    );
    turn.queue_follow_up().await;

    turn.runtime
        .settle_owned_provider_prompt(&turn.session_id, &substitute, true, false, true)
        .await
        .expect("the substitute completes the turn");

    turn.assert_follow_up_on_the_primary(&substitute);
}

/// The active turn running on a structured-I/O substitute, as a Codex or
/// OpenCode substitute submits through its runtime rather than a terminal.
/// A substitute profile cannot launch `slow-structured`, so its run is built
/// by hand like the failed attempt.
async fn structured_substitute_turn() -> (FailingTurn, String) {
    let turn = failing_turn(&[]).await;
    turn.runtime
        .retire_owned_provider_run_after_terminal_failure(&turn.session_id, &turn.failed_run_id)
        .await;
    let request = crate::provider::LaunchProviderRequest::new(
        &turn.session_id,
        "dev-stub",
        "slow-structured",
        "default",
        SUBSTITUTE_A,
    )
    .with_agent_id(&turn.agent_id)
    .with_turn_substitute(Some(TurnSubstitute {
        prompt_id: turn.prompt_id.clone(),
        tried: vec![crate::agent::AgentSubstituteProfile::new(
            "slow-structured",
            SUBSTITUTE_A,
            None,
        )],
    }));
    let mut run = crate::provider::RuntimeProviderRun::new(
        "structured-substitute",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-substitute".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-substitute-runtime".to_string()),
        },
    );
    run.mark_running();
    assert!(turn
        .runtime
        .owned
        .provider_store
        .run_uses_structured_prompt_io(&run));
    let substitute = run.id().to_string();
    turn.runtime
        .with_app_side_effect(|app| {
            app.providers_mut().insert_run_for_test(run.clone());
            app.sessions_mut()
                .set_active_provider_run(&turn.session_id, Some(substitute.clone()))
        })
        .await
        .unwrap();
    turn.runtime
        .owned
        .mark_active_prompt_delivery(
            &turn.session_id,
            &turn.agent_id,
            &turn.prompt_id,
            crate::session::DurablePromptDeliveryPhase::Delivered,
            Some(substitute.clone()),
            None,
        )
        .unwrap();
    turn.record_history(None, "review this change");
    turn.record_history(
        Some(&substitute),
        "Proposed fix: rename parse_all to parse.",
    );
    (turn, substitute)
}

#[tokio::test]
async fn a_follow_up_queued_behind_a_structured_substitute_reaches_the_primary() {
    let (turn, substitute) = structured_substitute_turn().await;
    turn.queue_follow_up().await;

    turn.runtime
        .settle_owned_provider_prompt(&turn.session_id, &substitute, true, false, true)
        .await
        .expect("the substitute completes the turn");

    turn.assert_follow_up_on_the_primary(&substitute);
}

#[tokio::test]
async fn a_follow_up_queued_behind_a_cancelled_structured_substitute_reaches_the_primary() {
    let (turn, substitute) = structured_substitute_turn().await;
    turn.queue_follow_up().await;
    let session = turn
        .runtime
        .owned
        .session_store
        .get_session(&turn.session_id)
        .unwrap();
    turn.runtime
        .owned
        .prompt_state_owner
        .begin_cancelling_active_prompt(&session, &turn.agent_id)
        .expect("the user cancels the substitute's turn");

    turn.runtime
        .settle_owned_provider_prompt(&turn.session_id, &substitute, true, false, true)
        .await
        .expect("the cancelled turn settles");

    turn.assert_follow_up_on_the_primary(&substitute);
}

#[tokio::test]
async fn a_follow_up_promoted_by_a_structured_abort_acknowledgement_reaches_the_primary() {
    let (turn, substitute) = structured_substitute_turn().await;
    turn.queue_follow_up().await;
    let session = turn
        .runtime
        .owned
        .session_store
        .get_session(&turn.session_id)
        .unwrap();
    turn.runtime
        .owned
        .prompt_state_owner
        .begin_cancelling_active_prompt(&session, &turn.agent_id)
        .expect("the user cancels the substitute's turn");
    let completion = turn
        .runtime
        .owned
        .provider_store
        .run_actor_completion_signal();
    let sequence = completion.sequence();
    turn.runtime
        .owned
        .provider_store
        .enqueue_structured_prompt_abort(turn.session_id.clone(), substitute.clone())
        .expect("the abort is sent");
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        completion.wait_for_change_after(sequence),
    )
    .await
    .expect("the provider acknowledges the abort");

    // The acknowledgement settles the cancelled turn; no terminal event follows.
    turn.runtime.reap_structured_prompt_jobs_and_dispatch();

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    while !turn
        .active_prompt()
        .and_then(|prompt| {
            prompt
                .durable_delivery_provider_run_id()
                .map(str::to_string)
        })
        .filter(|run| run != &substitute)
        .is_some_and(|run| {
            turn.provider_inputs(&run)
                .contains("implement that solution")
        })
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the follow-up is never dispatched"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    turn.assert_follow_up_on_the_primary(&substitute);
}

#[tokio::test]
async fn substitute_notices_never_carry_provider_credentials() {
    const SECRET: &str = "sk-live-0123456789abcdef";
    let failure =
        format!("Provider prompt dispatch failed: upstream failed Authorization: Bearer {SECRET}");
    let turn = failing_turn(&[("dev-stub", SUBSTITUTE_A, None)]).await;

    turn.fail_run(&turn.failed_run_id, &failure).await;
    let substitute = turn.assert_rerun_on(0, SUBSTITUTE_A);
    turn.fail_run(&substitute, &failure).await;

    let notices = turn.notices();
    assert!(notices
        .iter()
        .any(|notice| notice.starts_with("This turn runs on ")));
    assert!(notices
        .iter()
        .any(|notice| notice.starts_with("No substitute is left for this turn")));
    assert!(
        notices.iter().all(|notice| !notice.contains(SECRET)),
        "{notices:?}"
    );
}

impl FailingTurn {
    async fn edit_substitutes(&self, action: crate::local::AgentSubstituteAction) {
        self.runtime
            .update_agent_substitutes(
                &self.session_id,
                &self.agent_id,
                crate::session::DEFAULT_LOCAL_USER_ID,
                action,
            )
            .await
            .expect("the substitute list is edited during the turn");
    }
}

#[tokio::test]
async fn removing_the_running_substitute_still_leaves_the_next_one_for_its_failure() {
    let turn = failing_turn(&[
        ("dev-stub", SUBSTITUTE_A, None),
        ("dev-stub", SUBSTITUTE_B, None),
    ])
    .await;
    turn.fail_run(&turn.failed_run_id, SERVER_OVERLOADED).await;
    let first = turn.assert_rerun_on(0, SUBSTITUTE_A);

    turn.edit_substitutes(crate::local::AgentSubstituteAction::Remove { index: 0 })
        .await;
    turn.fail_run(&first, "OpenCode error: upstream request failed")
        .await;

    turn.assert_rerun_on(0, SUBSTITUTE_B);
}

#[tokio::test]
async fn reordering_substitutes_mid_turn_never_replays_a_tried_one() {
    let turn = failing_turn(&[
        ("dev-stub", SUBSTITUTE_A, None),
        ("dev-stub", SUBSTITUTE_B, None),
    ])
    .await;
    turn.fail_run(&turn.failed_run_id, SERVER_OVERLOADED).await;
    let first = turn.assert_rerun_on(0, SUBSTITUTE_A);

    turn.edit_substitutes(crate::local::AgentSubstituteAction::Move {
        from_index: 0,
        to_index: 1,
    })
    .await;
    turn.fail_run(&first, "OpenCode error: upstream request failed")
        .await;
    let second = turn.assert_rerun_on(0, SUBSTITUTE_B);
    turn.fail_run(&second, "OpenCode error: upstream request failed")
        .await;

    turn.assert_turn_failed();
    assert_eq!(
        turn.rerun_notices().len(),
        2,
        "{SUBSTITUTE_A} is not tried again"
    );
}
