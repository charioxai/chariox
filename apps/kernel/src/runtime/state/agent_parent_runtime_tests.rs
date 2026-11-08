//! MP-08 / MP-09 / MP-10 / MP-11: moving an agent re-limits its leased subtree on the worker
//! and the existing provider runs of its local subtree, so the next provider turn of either
//! starts within the new parent chain.
use super::*;
use crate::provider::{AgentExecutionMode as Mode, AgentPermissionLevel as Level};
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::Mutex;

const USER: &str = crate::session::DEFAULT_LOCAL_USER_ID;
const RELAY_URL: &str = "ws://127.0.0.1:1";
const WORKER_ID: &str = "worker-leased-limits";

/// Room agents `low` (Required/Plan) and `high` (Yolo/Build); `child` under `high` and
/// `grandchild` under `child`, both executing on one worker. `local` (with `child`'s settings)
/// under `high` and `local_child` (Yolo/Build) under `local` run here, each with a launched run;
/// so does `claude_like` (Yolo/Build) under `high`, whose provider takes settings only at launch.
struct LeasedTree {
    app: Arc<Mutex<crate::app::DaemonApp>>,
    runtime: KernelRuntimeState,
    relay_state: Arc<tokio::sync::RwLock<crate::transport::relay_client::RelayClientState>>,
    priority_rx: tokio::sync::mpsc::Receiver<chariox_relay::protocol::RelayEnvelope>,
    worker: crate::config::DaemonConfig,
    home_public_key: String,
    room: String,
    attachment: String,
    low: String,
    high: String,
    child: String,
    grandchild: String,
    local: String,
    local_child: String,
    claude_like: String,
}

fn limit(app: &crate::app::DaemonApp, agent: &str, mode: Mode, level: Level) {
    app.agents()
        .update_agent_config(agent, Some(Some(mode)), Some(Some(level)), None, None)
        .unwrap();
}

/// Takes new settings in the run it already has, as Codex does at its next turn start.
const CODEX_LIKE: &str = "managed-dev-stub";
/// Takes settings only when its process launches, as Claude does.
const CLAUDE_LIKE: &str = "dev-stub";

/// Launches the provider run of `agent` with `settings` through `adapter`.
fn launch(
    app: &mut crate::app::DaemonApp,
    room: &str,
    agent: &str,
    adapter: &str,
    (mode, level): (Mode, Level),
) {
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                room,
                adapter,
                "dev-stub",
                "default",
                "room-model",
            )
            .with_agent_id(agent)
            .with_execution_mode(mode)
            .with_permission_level(level),
        )
        .unwrap();
    app.update_provider_run_projection(run);
}

fn lease(app: &crate::app::DaemonApp, agent: &str) {
    app.agents()
        .bind_remote_execution(
            agent,
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: WORKER_ID.to_string(),
                worker_machine_id: "worker-machine-leased-limits".to_string(),
                execution_lease_id: format!("lease-{agent}"),
                leased_agent_id: format!("leased-{agent}"),
                active_worker_provider_run_id: None,
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
                worker_worktree_id: None,
            },
        )
        .unwrap();
}

async fn leased_tree(child_mode: Mode, child_level: Level) -> LeasedTree {
    leased_tree_under(Some((Mode::Build, Level::Yolo)), child_mode, child_level).await
}

/// `high` follows the room default when it has no settings of its own.
async fn leased_tree_under(
    high_settings: Option<(Mode, Level)>,
    child_mode: Mode,
    child_level: Level,
) -> LeasedTree {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some(RELAY_URL.to_string());
    config.relay_token = Some("relay-token".to_string());
    let home_public_key = config.relay_public_key.clone();
    let worker = crate::config::DaemonConfig::for_tests();
    let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
    let (session, low) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-leased-limits",
            std::env::current_dir()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        ))
        .unwrap();
    let room = session.id().to_string();
    let low = low.id().to_string();
    let spawn = |app: &mut crate::app::DaemonApp, alias: &str, parent: Option<&str>| {
        let mut request =
            crate::agent::CreateAgentRequest::new(&room, "dev-stub").with_alias(alias);
        if let Some(parent) = parent {
            crate::test_support::admit_room_test_turn(app, &room, parent);
            request = request.with_parent_agent_id(parent);
        }
        crate::app::KernelSessionService::new(app)
            .spawn_agent(request)
            .unwrap()
            .id()
            .to_string()
    };
    let high = spawn(&mut app, "high", None);
    let child = spawn(&mut app, "child", Some(&high));
    let grandchild = spawn(&mut app, "grandchild", Some(&child));
    limit(&app, &low, Mode::Plan, Level::Required);
    if let Some((mode, level)) = high_settings {
        limit(&app, &high, mode, level);
    }
    limit(&app, &child, child_mode, child_level);
    limit(&app, &grandchild, Mode::Build, Level::Yolo);
    // The local agents launch their runs with their own settings, on a provider that takes
    // settings per turn as Codex does.
    let local = spawn(&mut app, "local", Some(&high));
    limit(&app, &local, child_mode, child_level);
    launch(
        &mut app,
        &room,
        &local,
        CODEX_LIKE,
        (child_mode, child_level),
    );
    let local_child = spawn(&mut app, "local-child", Some(&local));
    limit(&app, &local_child, Mode::Build, Level::Yolo);
    launch(&mut app, &room, &local_child, CODEX_LIKE, OPEN);
    crate::test_support::admit_room_test_turn(&mut app, &room, &local_child);
    let claude_like = spawn(&mut app, "claude-like", Some(&high));
    limit(&app, &claude_like, Mode::Build, Level::Yolo);
    launch(&mut app, &room, &claude_like, CLAUDE_LIKE, OPEN);
    // The spawning turns end, so the room can raise `high` again.
    for agent in [&high, &child, &local, &local_child] {
        app.complete_active_prompt(&room, agent, None).unwrap();
    }
    lease(&app, &child);
    lease(&app, &grandchild);
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &room,
            "leased-limits-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap()
        .id()
        .to_string();
    let relay_state = app.relay_client_state();
    let app = Arc::new(Mutex::new(app));
    let runtime =
        crate::runtime::state::provider_launch_owned_state::owned_runtime_state(&app).await;
    let (outgoing_tx, priority_rx, _event_rx) =
        crate::transport::relay_client::RelayOutgoingSender::channel(8);
    {
        let mut relay = relay_state.write().await;
        relay.test_set_connected_sender(outgoing_tx, RELAY_URL);
        relay.remember_peer_public_key(WORKER_ID, worker.relay_public_key.clone());
    }
    LeasedTree {
        app,
        runtime,
        relay_state,
        priority_rx,
        worker,
        home_public_key,
        room,
        attachment,
        low,
        high,
        child,
        grandchild,
        local,
        local_child,
        claude_like,
    }
}

impl LeasedTree {
    /// Places `child` under `parent` while acting as the worker: answers each config update and
    /// returns what the worker was told per leased agent.
    async fn place_child(&mut self, parent: &str) -> BTreeMap<String, (Mode, Level)> {
        self.place(&self.child.clone(), parent).await
    }

    /// Places `agent` under `parent` while acting as the worker.
    async fn place(&mut self, agent: &str, parent: &str) -> BTreeMap<String, (Mode, Level)> {
        let placement = tokio::spawn({
            let runtime = self.runtime.clone();
            let request = crate::local::SetAgentParentRequest {
                session_id: self.room.clone(),
                agent_id: agent.to_string(),
                parent_agent_id: Some(parent.to_string()),
            };
            async move { runtime.set_agent_parent(USER, request).await.map(drop) }
        });
        self.answer_worker(placement).await
    }

    /// Sets `agent`'s own mode and level through UpdateAgentConfig while acting as the worker.
    async fn set_settings(
        &mut self,
        agent: &str,
        mode: Mode,
        level: Level,
    ) -> BTreeMap<String, (Mode, Level)> {
        let update = self.spawn_settings(agent, mode, level);
        self.answer_worker(update).await
    }

    fn spawn_settings(
        &self,
        agent: &str,
        mode: Mode,
        level: Level,
    ) -> tokio::task::JoinHandle<Result<(), DaemonError>> {
        tokio::spawn({
            let (runtime, room, agent) =
                (self.runtime.clone(), self.room.clone(), agent.to_owned());
            async move {
                runtime
                    .update_agent_config(
                        &room,
                        &agent,
                        USER,
                        Some(Some(mode)),
                        Some(Some(level)),
                        None,
                        None,
                    )
                    .await
                    .map(drop)
            }
        })
    }

    /// Sets the room default mode and level through UpdateSessionConfig while acting as the worker.
    async fn set_room_default(
        &mut self,
        mode: Mode,
        level: Level,
    ) -> BTreeMap<String, (Mode, Level)> {
        let update = tokio::spawn({
            let (runtime, room, attachment) = (
                self.runtime.clone(),
                self.room.clone(),
                self.attachment.clone(),
            );
            let values = [
                ("agents.mode".to_string(), mode.as_str().to_string()),
                ("agents.permissions".to_string(), level.as_str().to_string()),
            ]
            .into();
            async move {
                runtime
                    .update_session_config(&room, &attachment, values, false)
                    .await
                    .map(drop)
            }
        });
        self.answer_worker(update).await
    }

    /// Answers every config update the worker gets until `task` ends, and returns what the
    /// worker was last told per leased agent.
    async fn answer_worker(
        &mut self,
        mut task: tokio::task::JoinHandle<Result<(), DaemonError>>,
    ) -> BTreeMap<String, (Mode, Level)> {
        let mut told = BTreeMap::new();
        loop {
            tokio::select! {
                result = &mut task => {
                    result.unwrap().unwrap();
                    return told;
                }
                Some(envelope) = self.priority_rx.recv() => {
                    let request = self.read_request(envelope);
                    told.insert(request.1.clone(), (request.2, request.3));
                    self.answer(request).await;
                }
            }
        }
    }

    /// The next config update the worker gets, unanswered.
    async fn next_request(&mut self) -> WorkerRequest {
        let envelope =
            tokio::time::timeout(std::time::Duration::from_secs(2), self.priority_rx.recv())
                .await
                .expect("the worker must be told the leased team's limits")
                .unwrap();
        self.read_request(envelope)
    }

    fn read_request(&self, envelope: chariox_relay::protocol::RelayEnvelope) -> WorkerRequest {
        let chariox_relay::protocol::RelayEnvelope::DaemonPeerRequest {
            request_id,
            target,
            encrypted_request,
        } = envelope
        else {
            panic!("expected a peer request to the worker");
        };
        assert_eq!(target.daemon_id.as_deref(), Some(WORKER_ID));
        let plaintext = crate::transport::relay_crypto::decrypt_payload_for_private_key(
            &self.worker.relay_private_key,
            &encrypted_request,
        )
        .unwrap()
        .plaintext;
        let RelayPeerRequest::UpdateLeasedAgentConfig {
            leased_agent_id,
            execution_mode,
            permission_level,
        } = serde_json::from_slice(&plaintext).unwrap()
        else {
            panic!("expected a leased agent config update");
        };
        (
            request_id,
            leased_agent_id,
            execution_mode,
            permission_level,
        )
    }

    /// The worker applies `request`.
    async fn answer(&self, (request_id, leased_agent_id, mode, level): WorkerRequest) {
        let response = RelayPeerResponse::LeasedAgentConfigUpdated {
            leased_agent: leased_agent(&leased_agent_id, mode, level),
        };
        let encrypted = crate::transport::relay_crypto::encrypt_payload_for_peer(
            &self.worker.relay_private_key,
            &self.home_public_key,
            &serde_json::to_vec(&response).unwrap(),
        )
        .unwrap();
        crate::transport::relay_client::resolve_pending_peer_response_for_test(
            &self.relay_state,
            request_id,
            WORKER_ID.to_string(),
            encrypted,
        )
        .await;
    }

    /// What the existing provider run of local `agent` starts its next turn with: the Codex
    /// turn start reads these from the run it reuses.
    fn next_turn(&self, agent: &str) -> (Mode, Level) {
        let run = self
            .runtime
            .owned
            .provider_store
            .get_run_for_agent(&self.room, agent)
            .expect("the local agent keeps its launched provider run");
        assert_ne!(run.state(), crate::provider::ProviderRunState::Ended);
        (run.execution_mode(), run.permission_level())
    }

    /// The live provider run of `agent`, if it has one.
    fn live_run(&self, agent: &str) -> Option<crate::provider::RuntimeProviderRun> {
        self.runtime
            .owned
            .provider_store
            .get_run_for_agent(&self.room, agent)
            .filter(|run| run.state() != crate::provider::ProviderRunState::Ended)
    }

    /// Sends idle `agent` an ordinary prompt through runtime submission and returns the run its
    /// turn starts on: the run it reused, or the one it launched and waits for.
    async fn submit(&self, agent: &str, prompt_id: &str) -> crate::provider::RuntimeProviderRun {
        self.submit_prompt(agent, prompt_id).await;
        self.live_run(agent)
            .expect("an ordinary prompt has a provider run to start on")
    }

    async fn submit_prompt(
        &self,
        agent: &str,
        prompt_id: &str,
    ) -> crate::app::KernelPromptSubmission {
        self.runtime
            .submit_prepared_prompt(crate::app::KernelPreparedPromptSubmission {
                session_id: self.room.clone(),
                prompt: crate::session::PromptQueueItem::new(
                    prompt_id,
                    &self.attachment,
                    agent,
                    "continue",
                    crate::session::PromptStatus::Queued,
                ),
                force_queue: false,
                refresh_projection: true,
            })
            .await
            .unwrap()
    }

    /// The settings `agent` asked for itself.
    fn own_settings(&self, agent: &str) -> (Option<Mode>, Option<Level>) {
        let agent = self.runtime.owned.agent_store.get_agent(agent).unwrap();
        (
            agent.execution_mode_override(),
            agent.permission_level_override(),
        )
    }

    fn local_runs(&self) -> [(Mode, Level); 2] {
        [
            self.next_turn(&self.local),
            self.next_turn(&self.local_child),
        ]
    }

    fn told(&self, plan: [(&str, Mode, Level); 2]) -> BTreeMap<String, (Mode, Level)> {
        plan.into_iter()
            .map(|(agent, mode, level)| (format!("leased-{agent}"), (mode, level)))
            .collect()
    }
}

/// Request id, leased agent id, mode and level of one config update to the worker.
type WorkerRequest = (String, String, Mode, Level);

fn leased_agent(id: &str, mode: Mode, level: Level) -> crate::execution_lease::LeasedAgent {
    crate::execution_lease::LeasedAgent {
        id: id.to_string(),
        lease_id: format!("lease-{id}"),
        home_agent_id: id.trim_start_matches("leased-").to_string(),
        provider: "dev-stub".to_string(),
        account_profile: "default".to_string(),
        model: None,
        effort: None,
        execution_mode: Some(mode),
        permission_level: Some(level),
        backing_session_id: "worker-session".to_string(),
        backing_agent_id: format!("worker-{id}"),
        backing_attachment_id: "worker-attachment".to_string(),
        projected_prompt_ids: Vec::new(),
        projected_completion_keys: Vec::new(),
        projected_output_history_keys: Vec::new(),
        projected_provider_run: None,
        active_home_prompt_id: None,
        active_home_prompt_started_at_ms: None,
        applied_home_steer_ids: Vec::new(),
        home_steer_receipts: Vec::new(),
        replayable_completion: None,
        created_at_ms: 1,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_leased_subtree_moved_under_a_lower_root_runs_at_its_limit_on_the_worker() {
    let mut tree = leased_tree(Mode::Build, Level::Yolo).await;
    let low = tree.low.clone();
    let (child, grandchild) = (tree.child.clone(), tree.grandchild.clone());
    let told = tree.place_child(&low).await;
    assert_eq!(
        told,
        tree.told([
            (&child, Mode::Plan, Level::Required),
            (&grandchild, Mode::Plan, Level::Required),
        ]),
        "a Yolo/Build leased subtree under a Required/Plan root must launch at Required/Plan"
    );

    // Moved back, the subtree runs at its own settings again.
    let high = tree.high.clone();
    let told = tree.place_child(&high).await;
    assert_eq!(
        told,
        tree.told([
            (&child, Mode::Build, Level::Yolo),
            (&grandchild, Mode::Build, Level::Yolo),
        ])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_higher_root_never_raises_a_leased_agent_above_its_own_settings() {
    let mut tree = leased_tree(Mode::Plan, Level::Required).await;
    let (low, high) = (tree.low.clone(), tree.high.clone());
    let (child, grandchild) = (tree.child.clone(), tree.grandchild.clone());
    tree.place_child(&low).await;
    let told = tree.place_child(&high).await;
    assert_eq!(
        told,
        tree.told([
            (&child, Mode::Plan, Level::Required),
            (&grandchild, Mode::Plan, Level::Required),
        ]),
        "a Required/Plan agent and the team below it stay at Required/Plan under a Yolo/Build root"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lowering_a_parent_limits_its_leased_team_on_the_worker() {
    let mut tree = leased_tree(Mode::Build, Level::Yolo).await;
    let high = tree.high.clone();
    let (child, grandchild) = (tree.child.clone(), tree.grandchild.clone());
    let told = tree.set_settings(&high, Mode::Plan, Level::Required).await;
    assert_eq!(
        told,
        tree.told([
            (&child, Mode::Plan, Level::Required),
            (&grandchild, Mode::Plan, Level::Required),
        ]),
        "a Yolo/Build leased team under a parent lowered to Required/Plan must launch at Required/Plan"
    );

    // Raised again, the team runs at its own settings again.
    let told = tree.set_settings(&high, Mode::Build, Level::Yolo).await;
    assert_eq!(
        told,
        tree.told([
            (&child, Mode::Build, Level::Yolo),
            (&grandchild, Mode::Build, Level::Yolo),
        ])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn raising_a_parent_never_raises_a_leased_agent_above_its_own_settings() {
    let mut tree = leased_tree(Mode::Plan, Level::Required).await;
    let high = tree.high.clone();
    let (child, grandchild) = (tree.child.clone(), tree.grandchild.clone());
    tree.set_settings(&high, Mode::Plan, Level::Required).await;
    let told = tree.set_settings(&high, Mode::Build, Level::Yolo).await;
    assert_eq!(
        told,
        tree.told([
            (&child, Mode::Plan, Level::Required),
            (&grandchild, Mode::Plan, Level::Required),
        ]),
        "a Required/Plan agent and the team below it stay at Required/Plan under a raised parent"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lowering_the_room_default_limits_a_leased_team_whose_parent_follows_it() {
    let mut tree = leased_tree_under(None, Mode::Build, Level::Yolo).await;
    let (child, grandchild) = (tree.child.clone(), tree.grandchild.clone());
    let told = tree.set_room_default(Mode::Plan, Level::Required).await;
    assert_eq!(
        told,
        tree.told([
            (&child, Mode::Plan, Level::Required),
            (&grandchild, Mode::Plan, Level::Required),
        ]),
        "a leased team under a parent on the lowered room default must launch at Required/Plan"
    );

    let told = tree.set_room_default(Mode::Build, Level::Yolo).await;
    assert_eq!(
        told,
        tree.told([
            (&child, Mode::Build, Level::Yolo),
            (&grandchild, Mode::Build, Level::Yolo),
        ])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_worker_told_a_limit_that_changed_meanwhile_is_told_the_current_one() {
    let mut tree = leased_tree(Mode::Build, Level::Yolo).await;
    let high = tree.high.clone();
    let (child, grandchild) = (tree.child.clone(), tree.grandchild.clone());
    let lowering = tree.spawn_settings(&high, Mode::Plan, Level::Required);
    let first = tree.next_request().await;
    // `high` is raised again while the worker still applies the lowered limit.
    tree.runtime
        .owned
        .agent_store
        .update_agent_config(
            &high,
            Some(Some(Mode::Build)),
            Some(Some(Level::Yolo)),
            None,
            None,
        )
        .unwrap();
    let mut told = BTreeMap::from([(first.1.clone(), (first.2, first.3))]);
    tree.answer(first).await;
    told.extend(tree.answer_worker(lowering).await);
    assert_eq!(
        told,
        tree.told([
            (&child, Mode::Build, Level::Yolo),
            (&grandchild, Mode::Build, Level::Yolo),
        ]),
        "the worker must end at what the parent chain allows now, not at a limit computed before"
    );
}

const LIMITED: (Mode, Level) = (Mode::Plan, Level::Required);
const OPEN: (Mode, Level) = (Mode::Build, Level::Yolo);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_local_agent_raised_beneath_a_lower_parent_keeps_its_run_at_the_parent_limit() {
    let mut tree = leased_tree(Mode::Plan, Level::Required).await;
    let (low, local) = (tree.low.clone(), tree.local.clone());
    tree.place(&local, &low).await;
    assert_eq!(tree.local_runs(), [LIMITED, LIMITED]);
    tree.set_settings(&local, Mode::Build, Level::Yolo).await;
    assert_eq!(
        tree.next_turn(&local),
        LIMITED,
        "a local agent raised to Yolo/Build beneath a Required/Plan parent must keep running at Required/Plan"
    );
    assert_eq!(
        tree.own_settings(&local),
        (Some(Mode::Build), Some(Level::Yolo)),
        "the agent keeps the settings it asked for"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lowering_a_parent_limits_the_existing_runs_of_its_local_team() {
    let mut tree = leased_tree(Mode::Build, Level::Yolo).await;
    let high = tree.high.clone();
    assert_eq!(tree.local_runs(), [OPEN, OPEN]);
    tree.set_settings(&high, Mode::Plan, Level::Required).await;
    assert_eq!(
        tree.local_runs(),
        [LIMITED, LIMITED],
        "the next turns of a local team under a parent lowered to Required/Plan must run at Required/Plan"
    );

    // Raised again, the team runs at its own settings again.
    tree.set_settings(&high, Mode::Build, Level::Yolo).await;
    assert_eq!(tree.local_runs(), [OPEN, OPEN]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lowering_the_room_default_limits_the_runs_of_a_local_team_whose_parent_follows_it() {
    let mut tree = leased_tree_under(None, Mode::Build, Level::Yolo).await;
    tree.set_room_default(Mode::Build, Level::Yolo).await;
    assert_eq!(tree.local_runs(), [OPEN, OPEN]);
    tree.set_room_default(Mode::Plan, Level::Required).await;
    assert_eq!(
        tree.local_runs(),
        [LIMITED, LIMITED],
        "a local team under a parent on the lowered room default must run at Required/Plan"
    );

    tree.set_room_default(Mode::Build, Level::Yolo).await;
    assert_eq!(tree.local_runs(), [OPEN, OPEN]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_launched_local_team_moved_under_a_lower_root_runs_at_its_limit() {
    let mut tree = leased_tree(Mode::Build, Level::Yolo).await;
    let (low, high, local) = (tree.low.clone(), tree.high.clone(), tree.local.clone());
    tree.place(&local, &low).await;
    assert_eq!(
        tree.local_runs(),
        [LIMITED, LIMITED],
        "a launched Yolo/Build local team moved under a Required/Plan root must run at Required/Plan"
    );

    tree.place(&local, &high).await;
    assert_eq!(tree.local_runs(), [OPEN, OPEN]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_higher_root_never_raises_a_local_run_above_its_agents_own_settings() {
    let mut tree = leased_tree(Mode::Plan, Level::Required).await;
    let (low, high, local) = (tree.low.clone(), tree.high.clone(), tree.local.clone());
    tree.place(&local, &low).await;
    tree.place(&local, &high).await;
    assert_eq!(
        tree.local_runs(),
        [LIMITED, LIMITED],
        "a Required/Plan local agent and the team below it stay at Required/Plan under a Yolo/Build root"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_local_and_leased_paths_limit_the_same_tree_alike() {
    let mut tree = leased_tree(Mode::Build, Level::Yolo).await;
    let high = tree.high.clone();
    let (child, grandchild) = (tree.child.clone(), tree.grandchild.clone());
    for (mode, level) in [LIMITED, (Mode::Build, Level::Required), OPEN] {
        let told = tree.set_settings(&high, mode, level).await;
        // `local` and `local_child` sit where `child` and `grandchild` do, with their settings.
        assert_eq!(
            told,
            tree.told([
                (&child, tree.local_runs()[0].0, tree.local_runs()[0].1),
                (&grandchild, tree.local_runs()[1].0, tree.local_runs()[1].1),
            ]),
            "a local and a leased agent at one place in the tree must run under one limit"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_idle_run_that_takes_settings_only_at_launch_relaunches_at_a_lowered_limit() {
    let mut tree = leased_tree(Mode::Build, Level::Yolo).await;
    let (high, claude_like) = (tree.high.clone(), tree.claude_like.clone());
    let launched = tree.live_run(&claude_like).unwrap();
    assert_eq!(
        (launched.execution_mode(), launched.permission_level()),
        OPEN
    );
    tree.set_settings(&high, Mode::Plan, Level::Required).await;
    assert!(
        tree.live_run(&claude_like).is_none(),
        "an idle process that cannot take a lowered limit must end"
    );
    let next = tree.submit(&claude_like, "next-turn").await;
    assert_ne!(next.id(), launched.id());
    assert_eq!(
        (next.execution_mode(), next.permission_level()),
        LIMITED,
        "its next turn must launch at the parent's Required/Plan"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_running_turn_keeps_its_process_and_the_next_dispatch_moves_the_run() {
    let mut tree = leased_tree(Mode::Build, Level::Yolo).await;
    let (room, high) = (tree.room.clone(), tree.high.clone());
    let (claude_like, local) = (tree.claude_like.clone(), tree.local.clone());
    {
        let mut app = tree.app.lock().await;
        crate::test_support::admit_room_test_turn(&mut app, &room, &claude_like);
        crate::test_support::admit_room_test_turn(&mut app, &room, &local);
    }
    let launched = tree.live_run(&claude_like).unwrap();
    tree.set_settings(&high, Mode::Plan, Level::Required).await;
    assert_eq!(
        tree.live_run(&claude_like).map(|run| run.id().to_owned()),
        Some(launched.id().to_owned()),
        "a lowered limit must never end a turn in progress"
    );
    assert_eq!(
        tree.next_turn(&local),
        LIMITED,
        "a run that takes settings per turn takes a lowered limit at once"
    );
    tree.app
        .lock()
        .await
        .complete_active_prompt(&room, &claude_like, None)
        .unwrap();
    let next = tree.submit(&claude_like, "next-turn").await;
    assert_ne!(next.id(), launched.id());
    assert_eq!(
        (next.execution_mode(), next.permission_level()),
        LIMITED,
        "the turn after it launches at the lowered limit"
    );

    // A raise waits for the turn to end, then the run takes it in place.
    let running = tree.live_run(&local).unwrap();
    tree.set_settings(&high, Mode::Build, Level::Yolo).await;
    assert_eq!(
        tree.next_turn(&local),
        LIMITED,
        "a running turn must not be raised"
    );
    tree.app
        .lock()
        .await
        .complete_active_prompt(&room, &local, None)
        .unwrap();
    let next = tree.submit(&local, "next-turn-local").await;
    assert_eq!(next.id(), running.id());
    assert_eq!((next.execution_mode(), next.permission_level()), OPEN);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ordinary_prompt_after_a_turn_that_outlived_a_lowered_limit_launches_at_it() {
    let mut tree = leased_tree(Mode::Build, Level::Yolo).await;
    let (room, high, claude_like) = (
        tree.room.clone(),
        tree.high.clone(),
        tree.claude_like.clone(),
    );
    crate::test_support::admit_room_test_turn(&mut *tree.app.lock().await, &room, &claude_like);
    let launched = tree.live_run(&claude_like).unwrap();
    tree.set_settings(&high, Mode::Plan, Level::Required).await;
    tree.app
        .lock()
        .await
        .complete_active_prompt(&room, &claude_like, None)
        .unwrap();
    // No Project environment and no queued successor: only the tree limit is out of date.
    let next = tree.submit(&claude_like, "after-lowered-limit").await;
    assert_ne!(
        next.id(),
        launched.id(),
        "an idle Yolo/Build process must not start a turn under a Required/Plan parent"
    );
    assert_eq!((next.execution_mode(), next.permission_level()), LIMITED);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ordinary_prompt_after_a_turn_that_outlived_a_raise_takes_it_in_place() {
    let mut tree = leased_tree(Mode::Build, Level::Yolo).await;
    let (room, high) = (tree.room.clone(), tree.high.clone());
    let (local, claude_like) = (tree.local.clone(), tree.claude_like.clone());
    tree.set_settings(&high, Mode::Plan, Level::Required).await;
    crate::test_support::admit_room_test_turn(&mut *tree.app.lock().await, &room, &local);
    let running = tree.live_run(&local).unwrap();
    tree.set_settings(&high, Mode::Build, Level::Yolo).await;
    tree.app
        .lock()
        .await
        .complete_active_prompt(&room, &local, None)
        .unwrap();
    // Another agent's turn leaves the run of `local` where it is.
    tree.submit(&claude_like, "another-agent").await;
    assert_eq!(tree.next_turn(&local), LIMITED);
    let next = tree.submit(&local, "after-raise").await;
    assert_eq!(next.id(), running.id());
    assert_eq!(
        (next.execution_mode(), next.permission_level()),
        OPEN,
        "the turn after a raise must run at the raised limit"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_prompt_sent_during_a_turn_never_ends_it_for_a_changed_limit() {
    let mut tree = leased_tree(Mode::Build, Level::Yolo).await;
    let (room, high, claude_like) = (
        tree.room.clone(),
        tree.high.clone(),
        tree.claude_like.clone(),
    );
    crate::test_support::admit_room_test_turn(&mut *tree.app.lock().await, &room, &claude_like);
    let launched = tree.live_run(&claude_like).unwrap();
    tree.set_settings(&high, Mode::Plan, Level::Required).await;
    let queued = tree.submit_prompt(&claude_like, "during-turn").await;
    assert!(matches!(
        queued.outcome,
        crate::session::PromptSubmissionOutcome::Queued { .. }
    ));
    assert_eq!(
        tree.live_run(&claude_like).map(|run| run.id().to_owned()),
        Some(launched.id().to_owned()),
        "a prompt queued behind a running turn must never end that turn"
    );
}
