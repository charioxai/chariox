//! MP-08 / MP-09 / MP-10 / MP-11: an agent schedule trigger takes the Workflows panel's
//! schedule options. Requests are written as the clients send them on the wire.
use super::*;

#[inline(never)]
fn boxed<F: std::future::Future>(make_future: impl FnOnce() -> F) -> std::pin::Pin<Box<F>> {
    Box::pin(make_future())
}

struct Room {
    router: CommandRouter,
    session: crate::session::RuntimeSession,
    agent: crate::agent::AgentInstance,
    prompt_owner: crate::runtime::prompt_state::PromptStateOwner,
    sessions: crate::session::SessionStateStore,
    _worktree: crate::test_support::TestWorktree,
}

fn room(name: &str) -> Room {
    let worktree = crate::test_support::TestWorktree::new(name);
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should create");
    launch_test_provider(
        &mut app,
        session.id(),
        agent.id(),
        "dev-stub",
        "dev-stub",
        "schedule-model",
    );
    let prompt_owner = app.prompt_state_owner();
    let sessions = app.session_state_store();
    Room {
        sessions,
        router: CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4),
        session,
        agent,
        prompt_owner,
        _worktree: worktree,
    }
}

impl Room {
    async fn send(&self, request: serde_json::Value) -> Result<LocalDaemonResponse, DaemonError> {
        let request: LocalDaemonRequest =
            serde_json::from_value(request).map_err(|error| DaemonError::LocalTransport {
                operation: "decode request",
                message: error.to_string(),
            })?;
        let command = KernelCommand::from_local_request("schedule-options", None, None, &request);
        boxed(|| self.router.dispatch(command, request)).await
    }

    fn create(&self, source: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "CreateAgentTrigger": {
            "session_id": self.session.id(), "agent_id": self.agent.id(),
            "source": source, "prompt": "Check the build.",
        } })
    }

    async fn triggers(&self) -> Vec<serde_json::Value> {
        let session = self
            .router
            .runtime_state
            .session_snapshot(self.session.id())
            .await
            .expect("session snapshot");
        session
            .agent_triggers()
            .iter()
            .map(|trigger| serde_json::to_value(trigger).unwrap())
            .collect()
    }

    async fn fire(&self, now_ms: u64) {
        boxed(|| self.router.runtime_state.fire_due_agent_triggers(now_ms)).await;
    }

    fn own_prompts(&self, trigger_id: &str) -> (usize, usize) {
        let (active, queued) = self
            .prompt_owner
            .state_parts(&self.session, self.agent.id());
        (
            active
                .iter()
                .filter(|p| p.agent_trigger_id() == Some(trigger_id))
                .count(),
            queued
                .iter()
                .filter(|p| p.agent_trigger_id() == Some(trigger_id))
                .count(),
        )
    }

    fn finish_turn(&self) {
        self.prompt_owner
            .complete_active_prompt_only(&self.session, self.agent.id())
            .expect("the running turn should finish");
    }
}

fn created(response: LocalDaemonResponse) -> serde_json::Value {
    match response {
        LocalDaemonResponse::AgentTriggerCreated { trigger, .. } => {
            serde_json::to_value(trigger).unwrap()
        }
        response => panic!("expected a created trigger, got {response:?}"),
    }
}

fn next_run(trigger: &serde_json::Value) -> u64 {
    trigger["source"]["next_run_at_ms"].as_u64().unwrap()
}

#[tokio::test]
async fn every_schedule_mode_creates_a_cron_trigger_due_at_the_wall_clock_time_of_its_timezone() {
    use chrono::{Datelike, TimeZone, Timelike};
    let room = room("agent-schedule-modes");
    // The panel's modes as the shared builder writes them.
    for (expression, timezone) in [
        ("0 0 * * * *", "UTC"),
        ("0 30 9 * * *", "Europe/Berlin"),
        ("0 30 9 * * 1-5", "America/New_York"),
        ("15 0 18 * * 2,4", "Asia/Tokyo"),
        ("0 0 7 31 * *", "Australia/Sydney"),
        ("*/20 * * * * *", "UTC"),
    ] {
        let before = crate::session::unix_epoch_ms();
        let trigger = created(
            room.send(room.create(serde_json::json!({
                "kind": "schedule", "repeat": "every",
                "trigger": { "kind": "cron", "expression": expression, "timezone": timezone },
                "overlap_policy": "skip",
            })))
            .await
            .unwrap_or_else(|error| panic!("{expression} {timezone}: {error}")),
        );
        assert_eq!(
            trigger["source"]["trigger"],
            serde_json::json!({ "kind": "cron", "expression": expression, "timezone": timezone })
        );
        assert_eq!(
            trigger["source"]["max_runs"],
            serde_json::json!(crate::session::DEFAULT_WORKFLOW_SCHEDULE_MAX_RUNS),
            "a blank max runs takes the workflow schedule default"
        );
        let due = next_run(&trigger);
        assert!(due > before, "{expression}: due in the future");
        let tz: chrono_tz::Tz = timezone.parse().unwrap();
        let local = tz.timestamp_millis_opt(due as i64).unwrap();
        let fields: Vec<&str> = expression.split(' ').collect();
        let matches = |field: &str, value: u32| {
            field == "*"
                || field
                    .strip_prefix("*/")
                    .map(|step| value % step.parse::<u32>().unwrap() == 0)
                    .unwrap_or_else(|| {
                        field.split(',').any(|part| match part.split_once('-') {
                            Some((low, high)) => {
                                (low.parse().unwrap()..=high.parse().unwrap()).contains(&value)
                            }
                            None => part.parse::<u32>().unwrap() == value,
                        })
                    })
        };
        assert!(matches(fields[0], local.second()), "{expression} second");
        assert!(matches(fields[1], local.minute()), "{expression} minute");
        assert!(matches(fields[2], local.hour()), "{expression} hour");
        assert!(matches(fields[3], local.day()), "{expression} day");
        assert!(
            matches(fields[5], local.weekday().num_days_from_sunday()),
            "{expression} weekday"
        );
    }
    let interval = created(
        room.send(room.create(serde_json::json!({
            "kind": "schedule", "repeat": "every",
            "trigger": { "kind": "interval", "every_seconds": 900 }, "max_runs": null,
        })))
        .await
        .unwrap(),
    );
    assert_eq!(interval["source"].get("max_runs"), None, "null never stops");
    assert_eq!(interval["source"]["overlap_policy"], "skip");
    assert_eq!(room.triggers().await.len(), 7);
}

#[tokio::test]
async fn a_schedule_the_panel_would_refuse_is_refused_and_nothing_is_created() {
    let room = room("agent-schedule-validation");
    let cron = |expression: &str, timezone: &str| serde_json::json!({ "kind": "cron", "expression": expression, "timezone": timezone });
    for (source, message) in [
        (
            serde_json::json!({ "kind": "schedule", "repeat": "every", "trigger": cron("0 9 * * *", "UTC") }),
            "invalid cron expression `0 9 * * *`",
        ),
        (
            serde_json::json!({ "kind": "schedule", "repeat": "every", "trigger": cron("a b c d e f", "UTC") }),
            "invalid cron expression `a b c d e f`",
        ),
        (
            serde_json::json!({ "kind": "schedule", "repeat": "every", "trigger": cron("0 0 9 32 * *", "UTC") }),
            "invalid cron expression `0 0 9 32 * *`",
        ),
        (
            serde_json::json!({ "kind": "schedule", "repeat": "every", "trigger": cron("0 0 25 * * *", "UTC") }),
            "invalid cron expression `0 0 25 * * *`",
        ),
        (
            serde_json::json!({ "kind": "schedule", "repeat": "every", "trigger": cron("0 0 9 * * *", "Mars/Olympus") }),
            "invalid IANA timezone `Mars/Olympus`",
        ),
        (
            serde_json::json!({ "kind": "schedule", "repeat": "every", "trigger": { "kind": "interval", "every_seconds": 0 } }),
            "interval schedules require every_seconds greater than zero",
        ),
        (
            serde_json::json!({ "kind": "schedule", "repeat": "every", "trigger": { "kind": "interval", "every_seconds": 60 }, "max_runs": 0 }),
            "Max runs must be a positive integer or null.",
        ),
        (
            serde_json::json!({ "kind": "schedule", "repeat": "every", "trigger": { "kind": "interval", "every_seconds": 60 }, "max_runs": -2 }),
            "invalid value",
        ),
        (
            serde_json::json!({ "kind": "schedule", "repeat": "once", "trigger": { "kind": "interval", "every_seconds": 60 }, "max_runs": 3 }),
            "takes no max runs",
        ),
        (
            serde_json::json!({ "kind": "schedule", "repeat": "every", "trigger": { "kind": "interval", "every_seconds": 60 }, "overlap_policy": "replace" }),
            "unknown variant `replace`",
        ),
        (
            serde_json::json!({ "kind": "schedule", "repeat": "every", "trigger": { "kind": "interval", "every_seconds": 60 }, "queue_ref": "default" }),
            "unknown field `queue_ref`",
        ),
    ] {
        let error = room
            .send(room.create(source.clone()))
            .await
            .expect_err(&format!("{source} must be refused"));
        assert!(
            error.to_string().contains(message),
            "{source}: `{error}` should say `{message}`"
        );
    }
    let mut unknown = room.create(serde_json::json!({
        "kind": "schedule", "repeat": "every", "trigger": { "kind": "interval", "every_seconds": 60 },
    }));
    unknown["CreateAgentTrigger"]["name"] = serde_json::json!("nightly");
    let error = room
        .send(unknown)
        .await
        .expect_err("unknown fields are refused");
    assert!(
        error.to_string().contains("unknown field `name`"),
        "{error}"
    );
    assert!(room.triggers().await.is_empty(), "nothing was created");
}

#[tokio::test]
async fn queue_one_pending_run_keeps_exactly_one_firing_waiting_behind_the_running_one() {
    let room = room("agent-schedule-overlap-queue");
    let trigger = created(
        room.send(room.create(serde_json::json!({
            "kind": "schedule", "repeat": "every", "overlap_policy": "queue",
            "trigger": { "kind": "interval", "every_seconds": 60 },
        })))
        .await
        .unwrap(),
    );
    let id = trigger["id"].as_str().unwrap();
    let due = next_run(&trigger);
    room.fire(due).await;
    assert_eq!(room.own_prompts(id), (1, 0), "the first firing runs");
    room.fire(due + 60_000).await;
    assert_eq!(
        room.own_prompts(id),
        (1, 1),
        "one firing waits behind the running one"
    );
    for tick in 2..=5 {
        room.fire(due + tick * 60_000).await;
        assert_eq!(room.own_prompts(id), (1, 1), "never a second pending run");
    }
    let stats = &room.triggers().await[0];
    assert_eq!(stats["fired"], 2);
    assert_eq!(stats["coalesced"], 4);
}

#[tokio::test]
async fn skip_when_running_skips_every_firing_while_the_previous_one_is_handled() {
    let room = room("agent-schedule-overlap-skip");
    let trigger = created(
        room.send(room.create(serde_json::json!({
            "kind": "schedule", "repeat": "every", "overlap_policy": "skip",
            "trigger": { "kind": "interval", "every_seconds": 60 },
        })))
        .await
        .unwrap(),
    );
    let id = trigger["id"].as_str().unwrap();
    let due = next_run(&trigger);
    room.fire(due).await;
    for tick in 1..=3 {
        room.fire(due + tick * 60_000).await;
        assert_eq!(
            room.own_prompts(id),
            (1, 0),
            "a running firing skips the next"
        );
    }
    room.finish_turn();
    room.fire(due + 4 * 60_000).await;
    assert_eq!(
        room.own_prompts(id),
        (1, 0),
        "the next firing runs once it ended"
    );
    let stats = &room.triggers().await[0];
    assert_eq!(stats["fired"], 2);
    assert_eq!(stats["coalesced"], 3);
}

#[tokio::test]
async fn max_runs_stops_the_trigger_done_and_the_count_survives_a_restart() {
    let room = room("agent-schedule-max-runs");
    let trigger = created(
        room.send(room.create(serde_json::json!({
            "kind": "schedule", "repeat": "every", "max_runs": 2,
            "trigger": { "kind": "interval", "every_seconds": 60 },
        })))
        .await
        .unwrap(),
    );
    let id = trigger["id"].as_str().unwrap().to_string();
    let due = next_run(&trigger);
    room.fire(due).await;
    room.finish_turn();

    // Restart between the firings: the durable session is all that is kept.
    let saved = room
        .router
        .runtime_state
        .session_snapshot(room.session.id())
        .await
        .unwrap();
    let restored: crate::session::RuntimeSession =
        serde_json::from_value(serde_json::to_value(&saved).unwrap()).unwrap();
    room.prompt_owner.restore_session_state(&restored);
    room.sessions.restore_session(restored);
    assert_eq!(room.triggers().await[0]["fired"], 1);

    room.fire(due + 60_000).await;
    room.finish_turn();
    for tick in 2..=6 {
        room.fire(due + tick * 60_000).await;
        assert_eq!(room.own_prompts(&id), (0, 0), "a done trigger never fires");
    }
    let done = &room.triggers().await[0];
    assert_eq!(done["id"], id.as_str(), "a done trigger stays to be seen");
    assert_eq!(done["fired"], 2);
    assert_eq!(done["source"]["max_runs"], 2);
    assert_eq!(
        room.sessions.read().next_scheduled_runtime_wake_at_ms(),
        None,
        "nothing wakes the kernel for a done trigger"
    );
}

#[tokio::test]
async fn a_cron_trigger_is_disabled_enabled_from_now_deleted_and_survives_a_restart() {
    let room = room("agent-schedule-lifecycle");
    let trigger = created(
        room.send(room.create(serde_json::json!({
            "kind": "schedule", "repeat": "every",
            "trigger": { "kind": "cron", "expression": "0 * * * * *", "timezone": "Europe/Paris" },
        })))
        .await
        .unwrap(),
    );
    let id = trigger["id"].as_str().unwrap().to_string();
    let due = next_run(&trigger);
    let update = |enabled: bool| {
        serde_json::json!({ "UpdateAgentTrigger": {
            "session_id": room.session.id(), "trigger_id": id, "enabled": enabled,
        } })
    };
    room.send(update(false)).await.unwrap();
    room.fire(due + 3_600_000).await;
    assert_eq!(
        room.own_prompts(&id),
        (0, 0),
        "a disabled trigger does not fire"
    );

    let saved = room
        .router
        .runtime_state
        .session_snapshot(room.session.id())
        .await
        .unwrap();
    let restored: crate::session::RuntimeSession =
        serde_json::from_value(serde_json::to_value(&saved).unwrap()).unwrap();
    room.sessions.restore_session(restored);
    let kept = &room.triggers().await[0];
    assert_eq!(kept["enabled"], false);
    assert_eq!(kept["source"]["trigger"]["expression"], "0 * * * * *");
    assert_eq!(kept["source"]["trigger"]["timezone"], "Europe/Paris");

    let before = crate::session::unix_epoch_ms();
    room.send(update(true)).await.unwrap();
    let enabled = &room.triggers().await[0];
    let next = next_run(enabled);
    assert!(
        next > before && next <= before + 60_000,
        "the next minute from now"
    );
    assert_eq!(next % 60_000, 0, "on a whole minute");
    room.fire(next).await;
    assert_eq!(room.own_prompts(&id), (1, 0));

    room.send(serde_json::json!({ "DeleteAgentTrigger": {
        "session_id": room.session.id(), "trigger_id": id,
    } }))
    .await
    .unwrap();
    assert!(room.triggers().await.is_empty());
}

#[tokio::test]
async fn the_schedule_preview_gives_the_next_three_runs_across_a_dst_change_and_creates_nothing() {
    use chrono::TimeZone;
    let room = room("agent-schedule-preview");
    // The night Berlin moves to summer time: 09:30 local is 08:30Z before it and 07:30Z after.
    let after = chrono::Utc
        .with_ymd_and_hms(2027, 3, 27, 0, 0, 0)
        .unwrap()
        .timestamp_millis();
    let at = |day: u32, hour: u32| {
        chrono::Utc
            .with_ymd_and_hms(2027, 3, day, hour, 30, 0)
            .unwrap()
            .timestamp_millis() as u64
    };
    for (trigger, expected) in [
        (
            serde_json::json!({ "kind": "cron", "expression": "0 30 9 * * *", "timezone": "Europe/Berlin" }),
            vec![at(27, 8), at(28, 7), at(29, 7)],
        ),
        (
            serde_json::json!({ "kind": "cron", "expression": "0 30 9 * * 1-5", "timezone": "Europe/Berlin" }),
            vec![at(29, 7), at(30, 7), at(31, 7)],
        ),
        (
            serde_json::json!({ "kind": "interval", "every_seconds": 900 }),
            vec![
                after as u64 + 900_000,
                after as u64 + 1_800_000,
                after as u64 + 2_700_000,
            ],
        ),
    ] {
        let response = room
            .send(serde_json::json!({ "PreviewWorkflowSchedule": {
                "trigger": trigger, "after_ms": after, "count": 3,
            } }))
            .await
            .unwrap();
        let LocalDaemonResponse::WorkflowSchedulePreviewed { preview } = response else {
            panic!("expected a preview, got {response:?}");
        };
        assert_eq!(preview.next_run_at_ms, expected, "{trigger}");
    }
    let refused = room
        .send(serde_json::json!({ "PreviewWorkflowSchedule": {
            "trigger": { "kind": "cron", "expression": "0 30 9 * * *", "timezone": "Europe/Atlantis" },
            "after_ms": after, "count": 3,
        } }))
        .await
        .expect_err("a bad timezone cannot be previewed");
    assert!(refused
        .to_string()
        .contains("invalid IANA timezone `Europe/Atlantis`"));
    assert!(
        room.triggers().await.is_empty(),
        "a preview creates nothing"
    );
}
