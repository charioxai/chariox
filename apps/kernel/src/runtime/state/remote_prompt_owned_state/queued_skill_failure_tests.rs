//! G-CELLS Q1 / Q3 / Q4: a queued Claude turn whose selected skill changed after it was
//! queued fails visibly when the previous turn completes, and the queue keeps draining.
use super::*;
use crate::app::KernelPreparedPromptSubmission;
use crate::durable_state::agent_lifecycle::ExecutionState;
use crate::session::{PromptQueueItem, PromptStatus};
use crate::skill::{
    catalog::SkillCatalogSet, service::SkillCatalogService, storage::SkillCatalogStorage, v2::*,
};

fn submit(
    runtime: &KernelRuntimeState,
    session_id: &str,
    prompt: PromptQueueItem,
) -> PromptQueueItem {
    let submission = runtime
        .owned
        .submit_local_prepared_prompt(&KernelPreparedPromptSubmission {
            session_id: session_id.into(),
            prompt,
            force_queue: false,
            refresh_projection: true,
        })
        .unwrap()
        .unwrap();
    match submission.outcome {
        crate::session::PromptSubmissionOutcome::Started { prompt }
        | crate::session::PromptSubmissionOutcome::Queued { prompt } => prompt,
    }
}

fn publish(
    service: &SkillCatalogService,
    revision: u64,
    action: SkillCatalogAction,
) -> SkillCatalogSet {
    let mutation = SkillCatalogMutation {
        scope: SkillScope::Chariox,
        expected_revision: revision,
        actions: vec![action],
    };
    let prepared = service.prepare(&mutation).unwrap();
    service.publish_authorized(&mutation, &prepared).unwrap()
}

/// A local Claude stream-json run whose native discovery reads only test directories.
async fn claude_stream_json_turn(
    files: &crate::test_support::TestWorktree,
) -> (
    KernelRuntimeState,
    String,
    String,
    String,
    crate::provider::RuntimeProviderRun,
) {
    let mut config = crate::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(files.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "local-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude",
        "default",
        "sonnet",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "local-claude-run",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "claude:stream-json".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: [
                ("HOME".into(), files.path().display().to_string()),
                (
                    "CLAUDE_CONFIG_DIR".into(),
                    files.path().join("claude").display().to_string(),
                ),
            ]
            .into(),
            pty_env_remove: Vec::new(),
            working_directory: Some(files.path().into()),
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    let runtime = super::tests::owned_runtime_state(&Arc::new(Mutex::new(app))).await;
    runtime
        .owned
        .skill_inventories
        .set_test_version(run.id(), "2.1.212");
    (
        runtime,
        session.id().into(),
        agent.id().into(),
        attachment.id().into(),
        run,
    )
}

const TURNED_OFF: &str = "skill `review` was turned off after this prompt was queued; turn it back on in Skills or resend without it";

struct SettledBehindBusy {
    runtime: KernelRuntimeState,
    session_id: String,
    agent_id: String,
    busy: PromptQueueItem,
    /// The prompt queued after the skill turn, when the test queued one.
    after: Option<PromptQueueItem>,
    _files: crate::test_support::TestWorktree,
}

/// l12val2's repro: Claude is busy, a `$review` chip is queued, the owner turns the
/// skill Off, then Claude's first turn completes and the queued turn starts.
async fn skill_turn_starts_after_its_skill_was_turned_off(
    queue_another: bool,
) -> SettledBehindBusy {
    let files = crate::test_support::TestWorktree::new("queued-skill-off");
    let (runtime, session_id, agent_id, attachment, run) = claude_stream_json_turn(&files).await;
    let user_config = runtime.owned.config_projection.snapshot();
    let storage = SkillCatalogStorage::new(
        user_config.user_config_path.parent().unwrap(),
        runtime.owned.durable_state_store.clone(),
    )
    .unwrap();
    let service = SkillCatalogService::new(storage);
    let catalog = publish(
        &service,
        0,
        SkillCatalogAction::Install {
            source: SkillInstallSource::Content {
                skill_md: "---\nname: review\ndescription: Review safely\ndisable-model-invocation: true\n---\nReview source.\n".into(),
            },
            collision: SkillCollisionAction::Reject,
            package_id: None,
        },
    );
    let skill = catalog.catalogs[0].skills[0].clone();
    let busy = submit(
        &runtime,
        &session_id,
        PromptQueueItem::new(
            "busy",
            &attachment,
            &agent_id,
            "sleep",
            PromptStatus::Queued,
        ),
    );
    let chip = SkillSelection::Chariox {
        skill_id: skill.id.clone(),
        name: skill.name.clone(),
        catalog_revision: catalog.revision,
    };
    submit(
        &runtime,
        &session_id,
        PromptQueueItem::new(
            "with-skill",
            &attachment,
            &agent_id,
            "review it",
            PromptStatus::Queued,
        )
        .with_skill_selections(vec![chip]),
    );
    let after = queue_another.then(|| {
        submit(
            &runtime,
            &session_id,
            PromptQueueItem::new(
                "after",
                &attachment,
                &agent_id,
                "then this",
                PromptStatus::Queued,
            ),
        )
    });
    // The dispatcher delivered the busy turn to the run and bound its task.
    runtime
        .owned
        .mark_active_prompt_delivery(
            &session_id,
            &agent_id,
            busy.id(),
            crate::session::DurablePromptDeliveryPhase::Delivered,
            Some(run.id().to_string()),
            None,
        )
        .unwrap();
    runtime
        .owned
        .bind_agent_task_run(&session_id, &agent_id, busy.id(), run.id())
        .unwrap();
    publish(
        &service,
        catalog.revision,
        SkillCatalogAction::SetState {
            skill_id: skill.id.clone(),
            state: Some(SkillState::Off),
        },
    );
    // Claude streamed its answer and its result, then went quiet.
    runtime.owned.prompt_activity.write().insert(
        run.id().to_string(),
        crate::app::ActivePromptState {
            last_output_at: std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(1)),
            saw_response_content: true,
            completion_recorded: true,
            settlement_requested: true,
            active_tools: Default::default(),
            active_tool_started_at_ms: Default::default(),
        },
    );
    runtime
        .settle_owned_provider_prompt(&session_id, run.id(), true, false, false)
        .await
        .unwrap();
    SettledBehindBusy {
        runtime,
        session_id,
        agent_id,
        busy,
        after,
        _files: files,
    }
}

impl SettledBehindBusy {
    /// The skill turn's settled record, as every client's session state shows it.
    fn failed_turn(&self) -> crate::git_observer::CompletedGitTurnActionProjection {
        let session = self
            .runtime
            .owned
            .session_snapshot(&self.session_id)
            .unwrap();
        let turn = self.runtime.agent_activity_for_session(&session)[&self.agent_id]
            .last_completed_turn
            .clone()
            .expect("the failed turn stays visible");
        assert_ne!(
            turn.prompt_id,
            self.busy.id(),
            "the skill turn settled after the busy one"
        );
        assert_eq!(
            turn.settlement_status,
            crate::git_observer::CompletedTurnSettlementStatus::Failed,
            "{turn:?}"
        );
        let outcome = serde_json::to_value(&turn.outcome).unwrap();
        assert!(
            outcome["reason"]
                .as_str()
                .is_some_and(|text| text.contains(TURNED_OFF)),
            "{outcome}"
        );
        turn
    }
}

// Q1: the turn fails with the named reason, its task settles Blocked with it, and the
// agent is idle instead of holding a working task.
#[tokio::test]
async fn a_queued_turn_whose_skill_was_turned_off_fails_with_the_reason() {
    let settled = skill_turn_starts_after_its_skill_was_turned_off(false).await;
    let turn = settled.failed_turn();
    let task = settled
        .runtime
        .owned
        .durable_state_store
        .agent_tasks(Some(&settled.session_id), Some(&settled.agent_id))
        .unwrap()
        .into_iter()
        .find(|task| task.prompt_id == turn.prompt_id)
        .expect("the started skill turn has a task");
    assert_eq!(task.state, ExecutionState::Blocked, "{task:?}");
    assert!(task.reason.contains(TURNED_OFF), "{task:?}");
    let session = settled
        .runtime
        .owned
        .session_snapshot(&settled.session_id)
        .unwrap();
    assert!(session.active_prompt_for_agent(&settled.agent_id).is_none());
    assert!(session.queued_prompts().is_empty());
}

// Q4: the prompt queued behind the failed turn starts; the failed turn keeps its reason.
#[tokio::test]
async fn the_queue_drains_past_a_queued_turn_whose_skill_was_turned_off() {
    let settled = skill_turn_starts_after_its_skill_was_turned_off(true).await;
    settled.failed_turn();
    let session = settled
        .runtime
        .owned
        .session_snapshot(&settled.session_id)
        .unwrap();
    assert_eq!(
        session
            .active_prompt_for_agent(&settled.agent_id)
            .map(|prompt| prompt.id()),
        settled.after.as_ref().map(|prompt| prompt.id()),
        "the next queued prompt starts"
    );
}
