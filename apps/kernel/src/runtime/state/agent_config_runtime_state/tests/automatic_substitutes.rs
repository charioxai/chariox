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
    /// substitute run that serves only it.
    fn assert_rerun_on(&self, substitute_index: usize, model: &str) -> String {
        let prompt = self
            .active_prompt()
            .expect("a rerun keeps the failed turn active");
        assert_eq!(prompt.id(), self.prompt_id, "the same turn is rerun");
        let run = self.live_run().expect("the substitute run is live");
        assert_eq!(run.model(), model);
        assert_eq!(
            run.turn_substitute(),
            Some(&TurnSubstitute {
                prompt_id: self.prompt_id.clone(),
                substitute_index,
            })
        );
        assert_eq!(prompt.durable_delivery_provider_run_id(), Some(run.id()));
        let agent = self
            .runtime
            .owned
            .agent_store
            .get_agent(&self.agent_id)
            .unwrap();
        assert_eq!(agent.provider(), "dev-stub");
        assert_eq!(
            agent.model(),
            Some(PRIMARY_MODEL),
            "a substitute never becomes the agent's profile"
        );
        run.id().to_string()
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
    assert_eq!(errors, vec![SERVER_OVERLOADED.to_string()]);
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
