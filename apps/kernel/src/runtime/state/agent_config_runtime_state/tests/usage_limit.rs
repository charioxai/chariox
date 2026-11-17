//! A turn that ends on the provider's usage limit goes on when usage is back, by the kernel alone.

use super::*;
use crate::agent::UsageLimitStopState;
use crate::config::UsageLimitResume;
use crate::durable_state::agent_lifecycle::ExecutionState;

const OWNER: &str = crate::session::DEFAULT_LOCAL_USER_ID;
const WEEKLY: &str =
    "Claude StopFailure [rate_limit]: You've hit your weekly limit \u{b7} resets 4pm (UTC)";

struct Lane {
    agent_id: String,
    run_id: String,
    prompt_id: String,
}

struct Team {
    runtime: Option<KernelRuntimeState>,
    config: crate::config::DaemonConfig,
    session_id: String,
    lanes: Vec<Lane>,
    worktree: std::path::PathBuf,
}

impl Drop for Team {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.worktree);
    }
}

fn config(resume: UsageLimitResume) -> crate::config::DaemonConfig {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.agents.usage_limit_resume = Some(resume);
    config
}

/// `count` Claude agents of one room, each with a turn running on its own provider run.
async fn team(config: crate::config::DaemonConfig, count: usize) -> Team {
    let worktree = crate::test_support::test_workspace_root().join(format!(
        "chariox-usage-limit-{:016x}",
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&worktree).unwrap();
    let mut app = DaemonApp::bootstrap(config.clone()).expect("daemon bootstrap should succeed");
    let (session, first) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            worktree.to_string_lossy(),
            worktree.to_string_lossy(),
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let mut agent_ids = vec![first.id().to_string()];
    for _ in 1..count {
        let agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(crate::agent::CreateAgentRequest::new(
                &session_id,
                "dev-stub",
            ))
            .expect("agent should spawn");
        agent_ids.push(agent.id().to_string());
    }
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let mut lanes = Vec::new();
    for (index, agent_id) in agent_ids.iter().enumerate() {
        runtime
            .owned
            .agent_store
            .set_agent_runtime_profile_with_account_profile(
                agent_id,
                "dev-stub",
                Some("claude-haiku-5-5".to_string()),
                None,
                None,
                crate::provider::ProviderResumeState::default(),
            )
            .unwrap();
        let run_id = format!("limited-run-{index}");
        let request = crate::provider::LaunchProviderRequest::new(
            &session_id,
            "claude",
            "claude",
            "default",
            "claude-haiku-5-5",
        )
        .with_agent_id(agent_id);
        let mut run = crate::provider::RuntimeProviderRun::new(
            &run_id,
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::External,
                process_label: "test-claude".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: Some("test-claude-runtime".to_string()),
            },
        );
        run.mark_running();
        let (agent_for_prompt, run_for_app) = (agent_id.clone(), run.clone());
        runtime
            .with_app_side_effect(move |app| {
                app.providers_mut().insert_run_for_test(run_for_app.clone());
                let attachment = crate::app::KernelSessionService::new(app).attach(
                    crate::attachment::AttachRequest::new(
                        run_for_app.session_id(),
                        "usage-limit-test",
                        crate::attachment::ClientCapabilityLevel::FullTerminal,
                    ),
                )?;
                let prompt = crate::session::PromptQueueItem::new(
                    "stopped-prompt",
                    attachment.id(),
                    &agent_for_prompt,
                    "review this change",
                    crate::session::PromptStatus::Queued,
                );
                app.prompt_owner_submit_prepared_prompt(run_for_app.session_id(), prompt, false)
            })
            .await
            .unwrap();
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        let prompt_id = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .expect("the turn is active")
            .id()
            .to_string();
        runtime
            .owned
            .mark_active_prompt_delivery(
                &session_id,
                agent_id,
                &prompt_id,
                crate::session::DurablePromptDeliveryPhase::Delivered,
                Some(run_id.clone()),
                None,
            )
            .unwrap();
        lanes.push(Lane {
            agent_id: agent_id.clone(),
            run_id,
            prompt_id,
        });
    }
    Team {
        runtime: Some(runtime),
        config,
        session_id,
        lanes,
        worktree,
    }
}

impl Team {
    fn rt(&self) -> &KernelRuntimeState {
        self.runtime.as_ref().expect("the kernel runs")
    }

    fn agent(&self, lane: usize) -> crate::agent::AgentInstance {
        self.rt()
            .owned
            .agent_store
            .get_agent(&self.lanes[lane].agent_id)
            .unwrap()
    }

    fn stop(&self, lane: usize) -> Option<crate::agent::UsageLimitStop> {
        self.agent(lane).usage_limit_stop().cloned()
    }

    async fn hit_the_limit(&self, lane: usize) {
        self.hit_the_limit_saying(lane, WEEKLY).await;
    }

    async fn hit_the_limit_saying(&self, lane: usize, message: &str) {
        self.rt()
            .fail_owned_provider_prompt(&self.session_id, &self.lanes[lane].run_id, message, true)
            .await
            .expect("the limit settles the turn");
    }

    /// The limit as the provider's structured output carries it: the turn's error frame, not a
    /// failure the kernel settles by itself.
    async fn hit_the_limit_in_provider_output(&self, lane: usize) {
        self.rt()
            .apply_owned_structured_output_batch(
                &self.session_id,
                &self.lanes[lane].run_id,
                Vec::new(),
                crate::provider::ProviderPromptSignalBatch {
                    terminal_failure: Some(WEEKLY.to_string()),
                    explicit_provider_error: true,
                    prompt_completed: true,
                    ..Default::default()
                },
            )
            .await
            .expect("the limit frame settles the turn");
    }

    /// The failed turn's own line in the transcript, not the stop notice or a dropped queued request.
    fn failed_turn_lines(&self, lane: usize) -> Vec<String> {
        self.rt()
            .owned
            .operational_history_store
            .load_session_history_entries(&self.session_id, Some(&self.lanes[lane].agent_id))
            .unwrap()
            .into_iter()
            .filter(|entry| entry.text.starts_with("You've hit your"))
            .map(|entry| entry.text)
            .collect()
    }

    fn has_turn(&self, lane: usize) -> bool {
        let session = self
            .rt()
            .owned
            .session_store
            .get_session(&self.session_id)
            .unwrap();
        self.rt()
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &self.lanes[lane].agent_id)
            .is_some()
    }

    /// The cards the room shows for one agent, by id.
    fn cards(&self, lane: usize) -> Vec<String> {
        self.rt()
            .owned
            .session_store
            .get_session(&self.session_id)
            .unwrap()
            .active_interactions()
            .iter()
            .filter(|card| card.related_agent_id() == Some(self.lanes[lane].agent_id.as_str()))
            .map(|card| card.id().to_string())
            .collect()
    }

    /// The person's answer to a card, through the entry every client uses.
    async fn answer(&self, card: &str, choice: &str) {
        self.rt()
            .resolve_terminal_runtime_interaction(&self.session_id, card, choice, None, Some(OWNER))
            .await
            .expect("the card takes the answer");
    }

    /// Waits for the kernel's own task that applies an answer.
    async fn until(&self, lane: usize, reached: impl Fn(&crate::agent::UsageLimitStop) -> bool) {
        for _ in 0..200 {
            if self.stop(lane).is_some_and(|stop| reached(&stop)) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!(
            "the stop did not reach the expected state: {:?}",
            self.stop(lane)
        );
    }

    fn task_state(&self, lane: usize) -> Option<ExecutionState> {
        self.rt()
            .owned
            .durable_state_store
            .agent_turn_task(
                &self.session_id,
                &self.lanes[lane].agent_id,
                &self.lanes[lane].prompt_id,
            )
            .unwrap()
            .map(|task| task.state)
    }
}

#[tokio::test]
async fn a_turn_that_ends_on_the_limit_asks_by_default_and_names_when_usage_is_back() {
    let team = team(config(UsageLimitResume::Ask), 1).await;

    team.hit_the_limit(0).await;

    let stop = team.stop(0).expect("the agent records the stop");
    assert_eq!(stop.state, UsageLimitStopState::Asking);
    assert_eq!(stop.prompt_id, team.lanes[0].prompt_id);
    assert_eq!(stop.provider, "claude");
    assert!(
        stop.resumes_at_ms.is_some(),
        "the provider's own words name the time"
    );
    assert!(!team.has_turn(0));
}

#[tokio::test]
async fn an_armed_stop_says_when_the_kernel_resumes_instead_of_asking_to_send_again() {
    let team = team(config(UsageLimitResume::Automatic), 1).await;

    team.hit_the_limit(0).await;

    let lines = team.failed_turn_lines(0);
    assert_eq!(lines.len(), 1, "one failed-turn line: {lines:?}");
    assert!(
        lines[0].ends_with("Resumes automatically at 16:00 UTC."),
        "{}",
        lines[0]
    );
    assert!(!lines[0].contains("send the request again"), "{}", lines[0]);
}

#[tokio::test]
async fn an_armed_stop_from_the_providers_error_frame_says_when_the_kernel_resumes() {
    let team = team(config(UsageLimitResume::Automatic), 1).await;

    team.hit_the_limit_in_provider_output(0).await;

    assert!(matches!(
        team.stop(0).unwrap().state,
        UsageLimitStopState::Armed { .. }
    ));
    let lines = team.failed_turn_lines(0);
    assert_eq!(lines.len(), 1, "one failed-turn line: {lines:?}");
    assert!(
        lines[0].ends_with("Resumes automatically at 16:00 UTC."),
        "{}",
        lines[0]
    );
    assert!(!lines[0].contains("send the request again"), "{}", lines[0]);
}

#[tokio::test]
async fn asking_keeps_the_line_that_asks_to_send_again_in_the_providers_error_frame() {
    let team = team(config(UsageLimitResume::Ask), 1).await;

    team.hit_the_limit_in_provider_output(0).await;

    let lines = team.failed_turn_lines(0);
    assert_eq!(lines.len(), 1, "one failed-turn line: {lines:?}");
    assert!(lines[0].contains("send the request again"), "{}", lines[0]);
}

#[tokio::test]
async fn an_armed_stop_without_a_named_reset_says_it_resumes_when_usage_is_back() {
    let team = team(config(UsageLimitResume::Automatic), 1).await;

    team.hit_the_limit_saying(
        0,
        "Claude StopFailure [rate_limit]: You've hit your usage limit",
    )
    .await;

    assert!(matches!(
        team.stop(0).unwrap().state,
        UsageLimitStopState::Armed { .. }
    ));
    let lines = team.failed_turn_lines(0);
    assert_eq!(lines.len(), 1, "one failed-turn line: {lines:?}");
    assert!(
        lines[0].ends_with("Resumes automatically when usage is back."),
        "{}",
        lines[0]
    );
}

#[tokio::test]
async fn asking_keeps_the_line_that_asks_to_send_the_request_again() {
    let team = team(config(UsageLimitResume::Ask), 1).await;

    team.hit_the_limit(0).await;

    let lines = team.failed_turn_lines(0);
    assert_eq!(lines.len(), 1, "one failed-turn line: {lines:?}");
    assert!(lines[0].contains("send the request again"), "{}", lines[0]);
    assert!(!lines[0].contains("Resumes automatically"), "{}", lines[0]);
}

/// The kernel stops and starts again over the same durable state.
async fn restarted(team: &mut Team) -> KernelRuntimeState {
    drop(team.runtime.take());
    // The old kernel's own tasks (a dispatched turn) end before it lets go of the durable state.
    let mut starts = 0;
    let app = loop {
        match DaemonApp::bootstrap(team.config.clone()) {
            Ok(app) => break app,
            Err(error) if starts < 200 => {
                starts += 1;
                tracing::debug!(%error, "the old kernel still holds the durable state");
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
            Err(error) => panic!("the kernel starts again: {error:?}"),
        }
    };
    owned_runtime_state(&Arc::new(Mutex::new(app))).await
}

/// One scheduler tick, and every resume it sent.
async fn tick(runtime: &KernelRuntimeState, now: u64) {
    for send in runtime.tick_usage_limit_stops(now).await.unwrap() {
        send.await.unwrap();
    }
}

fn count_resuming(team: &Team) -> usize {
    (0..team.lanes.len())
        .filter(|lane| {
            matches!(
                team.stop(*lane).map(|stop| stop.state),
                Some(UsageLimitStopState::Resuming { .. })
            )
        })
        .count()
}

#[tokio::test]
async fn with_no_agent_awake_the_kernel_resumes_every_stopped_agent_at_the_reset_staggered() {
    let team = team(config(UsageLimitResume::Automatic), 3).await;
    for lane in 0..3 {
        team.hit_the_limit(lane).await;
    }
    let reset = team
        .stop(0)
        .unwrap()
        .resumes_at_ms
        .expect("the provider named a time");
    for lane in 0..3 {
        assert_eq!(
            team.stop(lane).unwrap().state,
            UsageLimitStopState::Armed { at_ms: reset },
            "every agent on the account is armed for the one reset time"
        );
        assert!(!team.has_turn(lane));
    }

    // Before the reset nothing starts.
    tick(team.rt(), reset - 1).await;
    assert_eq!(count_resuming(&team), 0);

    // At the reset one agent resumes; the others wait their turn, a stagger apart.
    tick(team.rt(), reset).await;
    assert_eq!(count_resuming(&team), 1);
    tick(team.rt(), reset + 1_000).await;
    assert_eq!(
        count_resuming(&team),
        1,
        "the stagger holds the second agent back"
    );
    let stagger = super::super::usage_limit_runtime::RESUME_STAGGER_MS;
    tick(team.rt(), reset + stagger).await;
    assert_eq!(count_resuming(&team), 2);
    tick(team.rt(), reset + 2 * stagger).await;
    assert_eq!(count_resuming(&team), 3);

    // Each agent goes on with a turn of its own in its own conversation.
    for lane in 0..3 {
        assert!(team.has_turn(lane), "agent {lane} has its turn back");
    }
}

#[tokio::test]
async fn a_restart_before_the_reset_loses_nothing() {
    let team = team(config(UsageLimitResume::Automatic), 2).await;
    for lane in 0..2 {
        team.hit_the_limit(lane).await;
    }
    let reset = team.stop(0).unwrap().resumes_at_ms.unwrap();

    let mut team = team;
    let after = restarted(&mut team).await;

    for lane in 0..2 {
        let agent = after
            .owned
            .agent_store
            .get_agent(&team.lanes[lane].agent_id)
            .unwrap();
        assert_eq!(
            agent.usage_limit_stop().map(|stop| stop.state.clone()),
            Some(UsageLimitStopState::Armed { at_ms: reset }),
            "the stop is still armed on the restarted kernel"
        );
    }
    tick(&after, reset).await;
    let session = after
        .owned
        .session_store
        .get_session(&team.session_id)
        .unwrap();
    let running = team
        .lanes
        .iter()
        .filter(|lane| {
            after
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &lane.agent_id)
                .is_some()
        })
        .count();
    assert_eq!(
        running, 1,
        "the restarted kernel resumes at the reset, staggered"
    );
    let stagger = super::super::usage_limit_runtime::RESUME_STAGGER_MS;
    tick(&after, reset + stagger).await;
    let session = after
        .owned
        .session_store
        .get_session(&team.session_id)
        .unwrap();
    for lane in &team.lanes {
        assert!(
            after
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &lane.agent_id)
                .is_some(),
            "agent {} resumed after the stagger",
            lane.agent_id
        );
    }
}

#[tokio::test]
async fn the_provider_refusing_at_the_reset_is_retried_a_few_times_and_then_asks() {
    let team = team(config(UsageLimitResume::Automatic), 1).await;
    team.hit_the_limit(0).await;
    let mut at = team.stop(0).unwrap().resumes_at_ms.unwrap();
    let waits = [60_000u64, 300_000, 900_000];
    let stagger = super::super::usage_limit_runtime::RESUME_STAGGER_MS;
    let mut ticked = 0;
    for (refused, wait) in waits.iter().enumerate() {
        // The scheduler's clock is the test's; it never runs back.
        ticked = at.max(ticked + stagger);
        tick(team.rt(), ticked).await;
        assert!(matches!(
            team.stop(0).unwrap().state,
            UsageLimitStopState::Resuming { .. }
        ));
        let live = team
            .rt()
            .owned
            .provider_store
            .get_run_for_agent(&team.session_id, &team.lanes[0].agent_id)
            .expect("the resume runs");
        // The provider still refuses.
        team.rt()
            .fail_owned_provider_prompt(&team.session_id, live.id(), WEEKLY, true)
            .await
            .unwrap();
        let stop = team.stop(0).unwrap();
        assert_eq!(stop.refused as usize, refused + 1);
        let UsageLimitStopState::Armed { at_ms } = stop.state else {
            panic!("armed again after refusal {}", refused + 1);
        };
        assert!(at_ms >= crate::session::unix_epoch_ms() + wait - 5_000);
        at = at_ms;
    }
    ticked = at.max(ticked + stagger);
    tick(team.rt(), ticked).await;
    let live = team
        .rt()
        .owned
        .provider_store
        .get_run_for_agent(&team.session_id, &team.lanes[0].agent_id)
        .unwrap();
    team.rt()
        .fail_owned_provider_prompt(&team.session_id, live.id(), WEEKLY, true)
        .await
        .unwrap();
    assert_eq!(
        team.stop(0).unwrap().state,
        UsageLimitStopState::Asking,
        "after the last retry the person is asked again"
    );
}

#[tokio::test]
async fn a_stopped_task_with_work_owed_goes_on_as_the_same_task() {
    let mut config = config(UsageLimitResume::Automatic);
    config.room_agent_tools = true;
    let team = team(config, 1).await;
    let agent = team.agent(0);
    let session = team
        .rt()
        .owned
        .session_store
        .get_session(&team.session_id)
        .unwrap();
    // The prompt carries an obligation, as a worker's does for its parent.
    team.rt()
        .owned
        .admit_agent_task(&crate::app::KernelPreparedPromptSubmission {
            session_id: team.session_id.clone(),
            prompt: team
                .rt()
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, agent.id())
                .unwrap(),
            force_queue: false,
            refresh_projection: false,
        })
        .unwrap();
    team.rt()
        .owned
        .durable_state_store
        .agent_lifecycle(
            crate::durable_state::agent_lifecycle::Operation::RegisterObligation {
                owner: agent.owner_user_id().into(),
                room: team.session_id.clone(),
                agent: agent.id().into(),
                prompt: team.lanes[0].prompt_id.clone(),
                run: None,
                id: "owed".into(),
                kind: "message".into(),
                resource: None,
                now: crate::session::unix_epoch_ms(),
            },
        )
        .unwrap();
    let task_id = team
        .rt()
        .owned
        .durable_state_store
        .agent_turn_task(&team.session_id, agent.id(), &team.lanes[0].prompt_id)
        .unwrap()
        .unwrap()
        .task_id;

    team.hit_the_limit(0).await;
    assert_eq!(team.task_state(0), Some(ExecutionState::Blocked));
    let reset = team.stop(0).unwrap().resumes_at_ms.unwrap();
    tick(team.rt(), reset).await;

    let task = team
        .rt()
        .owned
        .durable_state_store
        .agent_tasks(Some(&team.session_id), Some(agent.id()))
        .unwrap()
        .into_iter()
        .find(|task| task.task_id == task_id)
        .unwrap();
    assert_eq!(
        task.state,
        ExecutionState::Working,
        "the same task works again"
    );
    assert!(team.has_turn(0));
}

#[tokio::test]
async fn asking_waits_for_the_person_and_an_agent_cannot_take_the_decision() {
    let team = team(config(UsageLimitResume::Ask), 1).await;
    team.hit_the_limit(0).await;
    let reset = team.stop(0).unwrap().resumes_at_ms.unwrap();

    // Nothing fires without the person's decision, however late.
    tick(team.rt(), reset + 3_600_000).await;
    assert_eq!(team.stop(0).unwrap().state, UsageLimitStopState::Asking);

    // A parent cannot arm it under Ask me.
    let refused = team
        .rt()
        .resolve_usage_limit_stop(
            &team.session_id,
            &team.lanes[0].agent_id,
            OWNER,
            Some("a-parent"),
            crate::local::UsageLimitStopAction::ResumeWhenBack,
        )
        .await
        .expect_err("Ask me is the person's decision");
    assert!(
        refused
            .to_string()
            .contains("the person chose Ask me for usage limits"),
        "{refused}"
    );

    // The person arms it, and the kernel fires it at the reset.
    team.rt()
        .resolve_usage_limit_stop(
            &team.session_id,
            &team.lanes[0].agent_id,
            OWNER,
            None,
            crate::local::UsageLimitStopAction::ResumeWhenBack,
        )
        .await
        .unwrap();
    assert!(matches!(
        team.stop(0).unwrap().state,
        UsageLimitStopState::Armed { .. }
    ));
    // A parent may take an armed resume back.
    team.rt()
        .resolve_usage_limit_stop(
            &team.session_id,
            &team.lanes[0].agent_id,
            OWNER,
            Some("a-parent"),
            crate::local::UsageLimitStopAction::Cancel,
        )
        .await
        .unwrap();
    assert_eq!(team.stop(0).unwrap().state, UsageLimitStopState::Declined);
    tick(team.rt(), reset + 3_600_000).await;
    assert!(!team.has_turn(0), "a cancelled resume never fires");
}

/// A worker's prompt carries an obligation, so its stopped task is the one the stop must bring back.
fn owe_work(team: &Team, lane: usize) -> String {
    let agent = team.agent(lane);
    let session = team
        .rt()
        .owned
        .session_store
        .get_session(&team.session_id)
        .unwrap();
    team.rt()
        .owned
        .admit_agent_task(&crate::app::KernelPreparedPromptSubmission {
            session_id: team.session_id.clone(),
            prompt: team
                .rt()
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, agent.id())
                .unwrap(),
            force_queue: false,
            refresh_projection: false,
        })
        .unwrap();
    team.rt()
        .owned
        .durable_state_store
        .agent_lifecycle(
            crate::durable_state::agent_lifecycle::Operation::RegisterObligation {
                owner: agent.owner_user_id().into(),
                room: team.session_id.clone(),
                agent: agent.id().into(),
                prompt: team.lanes[lane].prompt_id.clone(),
                run: None,
                id: "owed".into(),
                kind: "message".into(),
                resource: None,
                now: crate::session::unix_epoch_ms(),
            },
        )
        .unwrap();
    team.rt()
        .owned
        .durable_state_store
        .agent_turn_task(&team.session_id, agent.id(), &team.lanes[lane].prompt_id)
        .unwrap()
        .unwrap()
        .task_id
}

fn task_of(
    team: &Team,
    task_id: &str,
) -> crate::durable_state::agent_lifecycle::AgentTaskExecution {
    team.rt()
        .owned
        .durable_state_store
        .agent_tasks(Some(&team.session_id), None)
        .unwrap()
        .into_iter()
        .find(|task| task.task_id == task_id)
        .unwrap()
}

#[tokio::test]
async fn asking_raises_one_card_that_the_person_answers_and_the_blocked_task_card_stays_hidden() {
    let mut config = config(UsageLimitResume::Ask);
    config.room_agent_tools = true;
    let team = team(config, 1).await;
    let task_id = owe_work(&team, 0);

    team.hit_the_limit(0).await;

    assert_eq!(task_of(&team, &task_id).state, ExecutionState::Blocked);
    let cards = team.cards(0);
    assert_eq!(
        cards.len(),
        1,
        "one decision, not the blocked task's card too: {cards:?}"
    );
    assert!(cards[0].starts_with("usage-limit-"), "{cards:?}");

    // The person's answer goes through the card, and the room's card is gone with it.
    team.answer(&cards[0], "resume_when_back").await;
    team.until(0, |stop| {
        matches!(stop.state, UsageLimitStopState::Armed { .. })
    })
    .await;
    assert!(
        team.cards(0).is_empty(),
        "an answered decision leaves no card"
    );
    let reset = team.stop(0).unwrap().resumes_at_ms.unwrap();
    tick(team.rt(), reset).await;
    assert_eq!(task_of(&team, &task_id).state, ExecutionState::Working);
}

#[tokio::test]
async fn a_decision_card_that_lost_its_reply_channel_is_raised_again() {
    let team = team(config(UsageLimitResume::Ask), 1).await;
    team.hit_the_limit(0).await;
    let first = team.cards(0);
    assert_eq!(first.len(), 1);
    // A restart leaves the card in the room with nobody to answer it.
    let lost = team
        .rt()
        .owned
        .pending_interactions
        .write()
        .get(&first[0])
        .and_then(|pending| pending.responder.lock().unwrap().take());
    assert!(lost.is_some());

    tick(team.rt(), crate::session::unix_epoch_ms()).await;
    let cards = team.cards(0);
    assert_eq!(cards, first, "the same decision is raised again");
    team.answer(&cards[0], "not_now").await;
    team.until(0, |stop| stop.state == UsageLimitStopState::Declined)
        .await;
    assert!(team.cards(0).is_empty());
}

#[tokio::test]
async fn a_task_that_hits_the_limit_again_after_its_resume_is_brought_back_as_the_same_task() {
    let mut config = config(UsageLimitResume::Automatic);
    config.room_agent_tools = true;
    let team = team(config, 1).await;
    let task_id = owe_work(&team, 0);
    let stagger = super::super::usage_limit_runtime::RESUME_STAGGER_MS;

    team.hit_the_limit(0).await;
    let reset = team.stop(0).unwrap().resumes_at_ms.unwrap();
    tick(team.rt(), reset).await;
    assert_eq!(task_of(&team, &task_id).state, ExecutionState::Working);

    // The task's continuation is a new turn; the provider refuses it too.
    let session = team
        .rt()
        .owned
        .session_store
        .get_session(&team.session_id)
        .unwrap();
    let continuation = team
        .rt()
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &team.lanes[0].agent_id)
        .expect("the task's continuation runs")
        .id()
        .to_string();
    assert_ne!(continuation, team.lanes[0].prompt_id);
    let live = team
        .rt()
        .owned
        .provider_store
        .get_run_for_agent(&team.session_id, &team.lanes[0].agent_id)
        .expect("the continuation runs");
    team.rt()
        .fail_owned_provider_prompt(&team.session_id, live.id(), WEEKLY, true)
        .await
        .unwrap();
    let stop = team.stop(0).unwrap();
    assert_eq!(
        stop.prompt_id, continuation,
        "the stop follows the turn that failed"
    );
    assert_eq!(task_of(&team, &task_id).state, ExecutionState::Blocked);
    let UsageLimitStopState::Armed { at_ms } = stop.state else {
        panic!("armed again after the refusal");
    };

    tick(team.rt(), at_ms.max(reset + stagger)).await;
    assert_eq!(
        task_of(&team, &task_id).state,
        ExecutionState::Working,
        "the same task works again, not a new prompt beside a blocked one"
    );
}

#[tokio::test]
async fn a_resume_taken_and_never_sent_is_sent_after_a_restart() {
    let mut team = team(config(UsageLimitResume::Automatic), 1).await;
    team.hit_the_limit(0).await;
    let stop = team.stop(0).unwrap();
    let claimed_at = stop.resumes_at_ms.unwrap();
    // The kernel wrote the claim and went away before it admitted any turn.
    let taken = crate::agent::UsageLimitStop {
        state: serde_json::from_value(
            serde_json::json!({"state": "resuming", "since_ms": claimed_at}),
        )
        .unwrap(),
        ..stop
    };
    team.rt()
        .store_usage_limit_stop(&team.agent(0), Some(taken), "")
        .await
        .unwrap();

    let after = restarted(&mut team).await;
    let session = after
        .owned
        .session_store
        .get_session(&team.session_id)
        .unwrap();
    assert!(
        after
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &team.lanes[0].agent_id)
            .is_none(),
        "no turn was admitted before the restart"
    );
    let later = claimed_at + 60_000;
    tick(&after, later).await;
    tick(&after, later + 1_000).await;
    let session = after
        .owned
        .session_store
        .get_session(&team.session_id)
        .unwrap();
    assert!(
        after
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &team.lanes[0].agent_id)
            .is_some(),
        "the resume the dead kernel took is sent, not cleared as done"
    );
}

#[tokio::test]
async fn a_resume_that_cannot_dispatch_stays_armed_for_the_next_tick() {
    let mut config = config(UsageLimitResume::Automatic);
    config.room_agent_tools = true;
    let team = team(config, 2).await;
    let task_id = owe_work(&team, 0);
    let stagger = super::super::usage_limit_runtime::RESUME_STAGGER_MS;
    team.hit_the_limit(0).await;
    let reset = team.stop(0).unwrap().resumes_at_ms.unwrap();
    // The agent ended up archived: its task cannot be resumed until it is back.
    team.rt()
        .owned
        .agent_store
        .archive_agent(
            &team.lanes[0].agent_id,
            true,
            &mut team.rt().owned.session_store.write(),
        )
        .unwrap();

    tick(team.rt(), reset).await;
    assert_eq!(
        task_of(&team, &task_id).state,
        ExecutionState::Blocked,
        "nothing was dispatched"
    );
    assert!(
        matches!(
            team.stop(0).unwrap().state,
            UsageLimitStopState::Armed { .. }
        ),
        "the failed resume is armed again, not consumed: {:?}",
        team.stop(0)
    );

    team.rt()
        .owned
        .agent_store
        .archive_agent(
            &team.lanes[0].agent_id,
            false,
            &mut team.rt().owned.session_store.write(),
        )
        .unwrap();
    tick(team.rt(), reset + 2 * stagger).await;
    assert_eq!(task_of(&team, &task_id).state, ExecutionState::Working);
}

/// The kernel admitted the resume and went away before it wrote that the resume was sent; the
/// restarted kernel and the time the stop is reached at come back.
async fn restarted_after_an_unrecorded_admission(team: &mut Team) -> (KernelRuntimeState, u64) {
    team.hit_the_limit(0).await;
    let reset = team.stop(0).unwrap().resumes_at_ms.unwrap();
    // The kernel resumes at the clock it saves a turn start with, so the claim is at that clock.
    let claimed_at = crate::session::unix_epoch_ms();
    let due = crate::agent::UsageLimitStop {
        state: UsageLimitStopState::Armed { at_ms: claimed_at },
        ..team.stop(0).unwrap()
    };
    team.rt()
        .store_usage_limit_stop(&team.agent(0), Some(due), "")
        .await
        .unwrap();
    tick(team.rt(), claimed_at).await;
    let admitted = team.stop(0).unwrap();
    let UsageLimitStopState::Resuming { since_ms, .. } = admitted.state else {
        panic!("the resume was claimed: {admitted:?}");
    };
    let unsent = crate::agent::UsageLimitStop {
        state: UsageLimitStopState::Resuming {
            since_ms,
            sent: false,
        },
        ..admitted
    };
    team.rt()
        .store_usage_limit_stop(&team.agent(0), Some(unsent), "")
        .await
        .unwrap();
    (restarted(team).await, reset)
}

fn agent_has_turn(team: &Team, runtime: &KernelRuntimeState) -> bool {
    let session = runtime
        .owned
        .session_store
        .get_session(&team.session_id)
        .unwrap();
    runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &team.lanes[0].agent_id)
        .is_some()
}

/// The turn ends the way the kernel settles one: the prompt leaves the agent and its settlement
/// is recorded.
fn finish_the_turn(team: &Team, runtime: &KernelRuntimeState, settled_at_ms: u64) {
    let session = runtime
        .owned
        .session_store
        .get_session(&team.session_id)
        .unwrap();
    let completed = runtime
        .owned
        .prompt_state_owner
        .complete_active_prompt_only(&session, &team.lanes[0].agent_id)
        .expect("a turn ends");
    runtime
        .owned
        .operational_history_store
        .record_prompt_settlement(
            false,
            &team.session_id,
            &team.lanes[0].agent_id,
            completed.id(),
            None,
            settled_at_ms,
            "completed",
            None,
        );
}

fn assert_no_second_resume(team: &Team, runtime: &KernelRuntimeState) {
    assert!(
        !agent_has_turn(team, runtime),
        "no second resume after the continuation finished"
    );
    assert!(
        runtime
            .owned
            .agent_store
            .get_agent(&team.lanes[0].agent_id)
            .unwrap()
            .usage_limit_stop()
            .is_none(),
        "the agent is back, its stop is cleared"
    );
}

#[tokio::test]
async fn a_resume_admitted_before_the_kernel_recorded_it_is_not_sent_again_after_a_restart() {
    let mut team = team(config(UsageLimitResume::Automatic), 1).await;
    let (after, reset) = restarted_after_an_unrecorded_admission(&mut team).await;
    assert!(
        agent_has_turn(&team, &after),
        "the admitted continuation runs again on the restarted kernel"
    );
    tick(&after, reset + 60_000).await;
    // The continuation finishes, and the kernel must not send it a second time.
    finish_the_turn(&team, &after, reset + 90_000);
    tick(&after, reset + 120_000).await;
    tick(&after, reset + 180_000).await;
    assert_no_second_resume(&team, &after);
}

#[tokio::test]
async fn a_continuation_that_finished_before_the_first_tick_after_a_restart_is_not_sent_again() {
    finished_before_the_first_tick_is_not_sent_again(config(UsageLimitResume::Automatic)).await;
}

/// Settlements are not saved without operational history; the receipt is the kernel's own.
#[tokio::test]
async fn a_continuation_that_finished_is_not_sent_again_when_history_is_not_captured() {
    let mut config = config(UsageLimitResume::Automatic);
    config.user_config.history.operational.enabled = false;
    finished_before_the_first_tick_is_not_sent_again(config).await;
}

async fn finished_before_the_first_tick_is_not_sent_again(config: crate::config::DaemonConfig) {
    let mut team = team(config, 1).await;
    let (after, reset) = restarted_after_an_unrecorded_admission(&mut team).await;
    finish_the_turn(&team, &after, reset + 30_000);
    tick(&after, reset + 60_000).await;
    tick(&after, reset + 120_000).await;
    tick(&after, reset + 180_000).await;
    assert_no_second_resume(&team, &after);
}

#[tokio::test]
async fn a_task_continuation_that_finished_before_the_first_tick_after_a_restart_is_not_sent_again()
{
    let mut config = config(UsageLimitResume::Automatic);
    config.room_agent_tools = true;
    let mut team = team(config, 1).await;
    let task_id = owe_work(&team, 0);
    let (after, reset) = restarted_after_an_unrecorded_admission(&mut team).await;
    let task = after
        .owned
        .durable_state_store
        .agent_tasks(Some(&team.session_id), None)
        .unwrap()
        .into_iter()
        .find(|task| task.task_id == task_id)
        .unwrap();
    assert_ne!(
        task.state,
        ExecutionState::Blocked,
        "the owner's resume reached the task"
    );
    if agent_has_turn(&team, &after) {
        finish_the_turn(&team, &after, reset + 30_000);
    }
    tick(&after, reset + 60_000).await;
    tick(&after, reset + 120_000).await;
    tick(&after, reset + 180_000).await;
    assert_no_second_resume(&team, &after);
}

#[tokio::test]
async fn a_decision_board_that_is_full_does_not_hold_back_another_agents_resume() {
    let mut config = config(UsageLimitResume::Ask);
    config.room_agent_tools = true;
    // The owner's board takes eight decisions; the ninth agent's card cannot be raised.
    let team = team(config, 10).await;
    let mut by_id: Vec<usize> = (0..10).collect();
    by_id.sort_by(|a, b| team.lanes[*a].agent_id.cmp(&team.lanes[*b].agent_id));
    for lane in &by_id {
        team.hit_the_limit(*lane).await;
    }
    let (uncarded, last) = (by_id[8], by_id[9]);
    assert!(team.cards(uncarded).is_empty(), "the board is full");
    // The person chose to resume the last agent, whom the scheduler reaches after the stuck one.
    team.rt()
        .resolve_usage_limit_stop(
            &team.session_id,
            &team.lanes[last].agent_id,
            OWNER,
            None,
            crate::local::UsageLimitStopAction::ResumeWhenBack,
        )
        .await
        .unwrap();
    let reset = team.stop(last).unwrap().resumes_at_ms.unwrap();

    tick(team.rt(), reset).await;
    assert!(
        team.has_turn(last),
        "the agent that was told to resume goes on"
    );
}
