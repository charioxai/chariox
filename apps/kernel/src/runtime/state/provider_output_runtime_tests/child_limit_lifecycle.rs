//! A child that yielded on a wake, or whose task a background hold keeps working, is idle in the
//! roster with no prompt, yet it still holds one of its parent's sixteen places. Both admission
//! paths, spawn and reparent, read that from the durable lifecycle.
use super::*;
use crate::agent::AgentState;
use crate::durable_state::agent_lifecycle::{AgentWake, Operation, Outcome};

const USER: &str = crate::session::DEFAULT_LOCAL_USER_ID;
const PLACES: usize = 16;

struct Family {
    runtime: KernelRuntimeState,
    app: Arc<Mutex<DaemonApp>>,
    room: String,
    parent: String,
    children: Vec<String>,
    _worktree: crate::test_support::TestWorktree,
}

/// A room agent with sixteen children that have each had their turn: the roster shows them idle.
async fn family(label: &str) -> Family {
    let worktree = crate::test_support::TestWorktree::new(label);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, parent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    crate::test_support::admit_room_test_turn(&mut app, session.id(), parent.id());
    let mut children = Vec::new();
    for index in 0..PLACES {
        let child = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias(format!("child-{index}"))
                    .with_parent_agent_id(parent.id()),
            )
            .unwrap();
        app.agents().note_prompt_sent_at(child.id(), 1).unwrap();
        children.push(child.id().to_owned());
    }
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    Family {
        runtime,
        app,
        room: session.id().into(),
        parent: parent.id().into(),
        children,
        _worktree: worktree,
    }
}

impl Family {
    fn store(&self) -> &crate::durable_state::DurableKernelStateStore {
        &self.runtime.owned.durable_state_store
    }

    /// Starts the child's task the way prompt dispatch does.
    fn begin(
        &self,
        child: &str,
        prompt: &str,
    ) -> crate::durable_state::agent_lifecycle::AgentTaskExecution {
        let Outcome::Task(task) = self
            .store()
            .agent_lifecycle(Operation::Begin {
                owner: "local".into(),
                room: self.room.clone(),
                agent: child.into(),
                prompt: prompt.into(),
                run: Some(format!("run-{prompt}")),
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap()
        else {
            panic!("begin returns the task");
        };
        task
    }

    /// The child's turn ended after it armed a wake and yielded on it.
    fn yield_on_a_wake(&self, child: &str) {
        let prompt = format!("prompt-{child}");
        let task = self.begin(child, &prompt);
        let wake: AgentWake = serde_json::from_value(serde_json::json!({
            "id": format!("wake-{child}"), "task_id": task.task_id, "room_id": self.room,
            "agent_id": child, "registration_id": format!("completion-wake-{child}"),
            "kind": "timer", "label": "wait", "state": "scheduled",
            "created_at_ms": crate::session::unix_epoch_ms(),
            "verified_at_ms": null, "next_due_ms": crate::session::unix_epoch_ms() + 600_000,
            "interval_ms": null, "command": [], "match_text": null, "matched_at_ms": null,
            "pid": null, "exit_code": null, "fire_count": 0, "missed_fires": 0,
            "last_fired_at_ms": null, "last_sequence": null, "last_delivery": null,
            "last_delivered_at_ms": null, "last_acknowledged_at_ms": null, "alerted_sequence": null
        }))
        .unwrap();
        self.store()
            .agent_lifecycle(Operation::ArmWake {
                task: task.task_id.clone(),
                prompt: prompt.clone(),
                wake: Box::new(wake),
            })
            .unwrap();
        let now = crate::session::unix_epoch_ms();
        self.store()
            .agent_lifecycle(Operation::Yield {
                task: task.task_id.clone(),
                prompt: prompt.clone(),
                registrations: vec![format!("completion-wake-{child}")],
                cursor: 0,
                deadline: now + 600_000,
                reason: "Wait for the timer".into(),
                now,
            })
            .unwrap();
        self.store()
            .agent_lifecycle(Operation::Settle {
                room: self.room.clone(),
                agent: child.into(),
                prompt,
                run: format!("run-prompt-{child}"),
                has_answer: true,
                cancelled: false,
                now,
            })
            .unwrap();
    }

    /// The child's turn is over but a background command keeps its task working.
    fn hold_in_the_background(&self, child: &str) {
        self.begin(child, &format!("prompt-{child}"));
    }

    /// The roster shows every child idle and no prompt is running or queued.
    async fn assert_children_look_idle(&self) {
        let app = self.app.lock().await;
        let session = app.sessions().get_session(&self.room).unwrap();
        for child in app.agents().get_session_agents(&self.room) {
            if child.id() == self.parent {
                continue;
            }
            assert_ne!(child.state(), AgentState::Working, "{}", child.id());
            assert!(session.active_prompt_for_agent(child.id()).is_none());
            assert!(session
                .queued_prompts_for_agent(child.id())
                .is_none_or(|queued| queued.is_empty()));
        }
    }

    async fn spawn_one_more(&self) -> Result<crate::agent::AgentInstance, DaemonError> {
        let mut app = self.app.lock().await;
        crate::app::KernelSessionService::new(&mut app).spawn_agent(
            crate::agent::CreateAgentRequest::new(&self.room, "dev-stub")
                .with_alias("seventeenth")
                .with_parent_agent_id(&self.parent),
        )
    }

    async fn room_agent(&self, alias: &str) -> String {
        let mut app = self.app.lock().await;
        crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(&self.room, "dev-stub").with_alias(alias),
            )
            .unwrap()
            .id()
            .to_owned()
    }

    async fn place_under_parent(&self, agent: &str) -> Result<(), DaemonError> {
        self.runtime
            .set_agent_parent(
                USER,
                crate::local::SetAgentParentRequest {
                    session_id: self.room.clone(),
                    agent_id: agent.into(),
                    parent_agent_id: Some(self.parent.clone()),
                },
            )
            .await
            .map(|_| ())
    }
}

fn assert_cap(result: Result<impl std::fmt::Debug, DaemonError>) {
    let error = result.expect_err("sixteen unfinished children are the cap");
    assert!(
        error.to_string().contains("at most 16"),
        "refused for another reason: {error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn children_waiting_on_a_wake_keep_their_places_for_a_spawn() {
    let family = family("child-limit-wake-spawn").await;
    for child in &family.children {
        family.yield_on_a_wake(child);
    }
    family.assert_children_look_idle().await;
    assert_cap(family.spawn_one_more().await);
}

#[tokio::test(flavor = "multi_thread")]
async fn children_in_a_background_hold_keep_their_places_for_a_spawn() {
    let family = family("child-limit-hold-spawn").await;
    for child in &family.children {
        family.hold_in_the_background(child);
    }
    family.assert_children_look_idle().await;
    assert_cap(family.spawn_one_more().await);
}

#[tokio::test(flavor = "multi_thread")]
async fn children_waiting_on_a_wake_keep_their_places_for_a_reparent() {
    let family = family("child-limit-wake-reparent").await;
    for child in &family.children {
        family.yield_on_a_wake(child);
    }
    family.assert_children_look_idle().await;
    let loose = family.room_agent("loose").await;
    assert_cap(family.place_under_parent(&loose).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn children_in_a_background_hold_keep_their_places_for_a_reparent() {
    let family = family("child-limit-hold-reparent").await;
    for child in &family.children {
        family.hold_in_the_background(child);
    }
    family.assert_children_look_idle().await;
    let loose = family.room_agent("loose").await;
    assert_cap(family.place_under_parent(&loose).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn children_that_finished_leave_their_places_to_a_spawn_and_a_reparent() {
    let family = family("child-limit-finished").await;
    family.assert_children_look_idle().await;
    let loose = family.room_agent("loose").await;
    family.place_under_parent(&loose).await.unwrap();
    family.spawn_one_more().await.unwrap();
}
