//! MP-08 / MP-09 / MP-10 / MP-11: moving an agent re-limits its leased subtree on the worker,
//! so the next provider turn there launches within the new parent chain.
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
/// `grandchild` under `child`, both executing on one worker.
struct LeasedTree {
    runtime: KernelRuntimeState,
    relay_state: Arc<tokio::sync::RwLock<crate::transport::relay_client::RelayClientState>>,
    priority_rx: tokio::sync::mpsc::Receiver<chariox_relay::protocol::RelayEnvelope>,
    worker: crate::config::DaemonConfig,
    home_public_key: String,
    room: String,
    low: String,
    high: String,
    child: String,
    grandchild: String,
}

fn limit(app: &crate::app::DaemonApp, agent: &str, mode: Mode, level: Level) {
    app.agents()
        .update_agent_config(agent, Some(Some(mode)), Some(Some(level)), None, None)
        .unwrap();
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
    limit(&app, &high, Mode::Build, Level::Yolo);
    limit(&app, &child, child_mode, child_level);
    limit(&app, &grandchild, Mode::Build, Level::Yolo);
    lease(&app, &child);
    lease(&app, &grandchild);
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
        runtime,
        relay_state,
        priority_rx,
        worker,
        home_public_key,
        room,
        low,
        high,
        child,
        grandchild,
    }
}

impl LeasedTree {
    /// Places `child` under `parent` while acting as the worker: answers each config update and
    /// returns what the worker was told per leased agent.
    async fn place_child(&mut self, parent: &str) -> BTreeMap<String, (Mode, Level)> {
        let placement = tokio::spawn({
            let runtime = self.runtime.clone();
            let request = crate::local::SetAgentParentRequest {
                session_id: self.room.clone(),
                agent_id: self.child.clone(),
                parent_agent_id: Some(parent.to_string()),
            };
            async move { runtime.set_agent_parent(USER, request).await }
        });
        let mut told = BTreeMap::new();
        while told.len() < 2 {
            let envelope = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                self.priority_rx.recv(),
            )
            .await
            .unwrap_or_else(|_| {
                panic!("the worker must be told the moved subtree's limits; it was told {told:?}")
            })
            .unwrap();
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
            let response = RelayPeerResponse::LeasedAgentConfigUpdated {
                leased_agent: leased_agent(&leased_agent_id, execution_mode, permission_level),
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
            told.insert(leased_agent_id, (execution_mode, permission_level));
        }
        placement.await.unwrap().unwrap();
        told
    }

    fn told(&self, plan: [(&str, Mode, Level); 2]) -> BTreeMap<String, (Mode, Level)> {
        plan.into_iter()
            .map(|(agent, mode, level)| (format!("leased-{agent}"), (mode, level)))
            .collect()
    }
}

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
