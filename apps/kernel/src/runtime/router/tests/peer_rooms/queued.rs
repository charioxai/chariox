//! A message that is not urgent waits in its receiver's queue and is delivered in the receiver's
//! next turn; only an urgent one steers a running turn or starts one. The sender can promote its
//! queued message, and the person cancels it or sends it from the receiver's queue.
use super::*;

const QUEUED_NOTE: &str =
    "queued for idle's next turn; send with urgent: true to start or steer now";

async fn spawn_idle(rooms: &Rooms) -> crate::agent::AgentInstance {
    let mut app = rooms.app.lock().await;
    spawn_with_level(
        &mut app,
        &rooms.peer_room,
        "idle",
        AgentPermissionLevel::Yolo,
    )
}

/// An idle agent that something will wake: its task is parked on a timer and it runs no turn, as
/// after `events.yield`. A message that is not urgent waits for that next turn. (An idle agent with
/// nothing armed has none coming, so the same message starts its turn: see
/// `a_message_to_an_idle_agent_with_nothing_armed_starts_its_turn`.) The task is made in the durable
/// store alone, so the session state the budget test reads holds no turn of the agent.
async fn spawn_waiting(rooms: &Rooms) -> crate::agent::AgentInstance {
    use crate::durable_state::agent_lifecycle::Operation;
    let agent = spawn_idle(rooms).await;
    rooms
        .app
        .lock()
        .await
        .durable_state_store()
        .agent_lifecycle(Operation::Begin {
            owner: crate::session::DEFAULT_LOCAL_USER_ID.into(),
            room: rooms.peer_room.clone(),
            agent: agent.id().into(),
            prompt: "armed-turn".into(),
            run: None,
            now: crate::session::unix_epoch_ms(),
        })
        .expect("the agent has a task");
    rooms.park_on_timer(&rooms.peer_room, agent.id()).await;
    agent
}

async fn send_to(rooms: &Rooms, agent: &str, extra: serde_json::Value) -> serde_json::Value {
    let mut args = serde_json::json!({
        "agent": agent,
        "room": rooms.peer_room,
        "message": "compare notes on the parser",
    });
    for (key, value) in extra.as_object().unwrap() {
        args[key] = value.clone();
    }
    rooms.send(args).await.expect("message across rooms")
}

async fn stripe(rooms: &Rooms) -> Vec<crate::session::QueuedAgentMessage> {
    rooms
        .router
        .runtime_state
        .session_snapshot(&rooms.peer_room)
        .await
        .expect("the room snapshot")
        .queued_agent_messages()
        .to_vec()
}

async fn states(rooms: &Rooms, agent: &str) -> Vec<String> {
    rooms
        .inbox(&rooms.peer_room, agent)
        .await
        .into_iter()
        .map(|event| event.state)
        .collect()
}

#[test]
fn a_message_to_an_idle_agent_queues_and_starts_no_turn() {
    run_peer_test("queued-idle", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let idle = spawn_waiting(&rooms).await;
        let sent = send_to(&rooms, "idle", serde_json::json!({})).await;
        assert_eq!(sent["status"], "queued");
        assert_eq!(sent["note"], QUEUED_NOTE);
        assert!(sent["message_id"].as_str().is_some_and(|id| !id.is_empty()));
        assert!(
            !rooms.turn_running(&rooms.peer_room, idle.id()).await,
            "a queued message starts nothing"
        );
        assert!(
            !rooms
                .message_was_delivered(&rooms.peer_room, idle.id())
                .await
        );
        let stripe = stripe(&rooms).await;
        assert_eq!(stripe.len(), 1);
        assert_eq!(stripe[0].agent_id, idle.id());
        assert_eq!(stripe[0].sender, "sender");
        assert_eq!(stripe[0].preview, "compare notes on the parser");
        assert!(!stripe[0].truncated);
        assert_eq!(states(&rooms, idle.id()).await, vec!["pending"]);
    });
}

#[test]
fn a_message_to_an_idle_agent_with_nothing_armed_starts_its_turn() {
    run_peer_test("queued-idle-nothing-armed", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let idle = spawn_idle(&rooms).await;
        let sent = send_to(&rooms, "idle", serde_json::json!({})).await;
        assert_eq!(sent["status"], "starting");
        assert_eq!(
            sent["note"],
            "idle was idle with nothing armed, so this message starts its turn now; no urgent flag needed"
        );
        assert!(
            rooms
                .wait_for_message_delivery(&rooms.peer_room, idle.id())
                .await
        );
        assert!(stripe(&rooms).await.is_empty(), "nothing is left queued");
    });
}

#[test]
fn a_message_to_a_running_agent_queues_and_does_not_steer_it() {
    run_peer_test("queued-running", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let sent = send_to(&rooms, "peer", serde_json::json!({})).await;
        assert_eq!(sent["status"], "queued");
        assert!(rooms.turn_running(&rooms.peer_room, rooms.peer.id()).await);
        assert!(
            !rooms
                .message_was_delivered(&rooms.peer_room, rooms.peer.id())
                .await
        );
        assert_eq!(states(&rooms, rooms.peer.id()).await, vec!["pending"]);
        assert_eq!(stripe(&rooms).await.len(), 1);
    });
}

#[test]
fn an_urgent_message_to_an_idle_agent_starts_its_turn() {
    run_peer_test("queued-urgent-idle", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let idle = spawn_idle(&rooms).await;
        let sent = send_to(&rooms, "idle", serde_json::json!({"urgent": true})).await;
        assert_eq!(sent["status"], "started");
        assert!(sent["prompt_id"].as_str().is_some());
        assert!(
            rooms
                .wait_for_message_delivery(&rooms.peer_room, idle.id())
                .await
        );
        assert!(stripe(&rooms).await.is_empty());
    });
}

#[test]
fn the_person_cancels_a_queued_message_from_the_queue() {
    run_peer_test("queued-person-cancel", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let idle = spawn_waiting(&rooms).await;
        send_to(&rooms, "idle", serde_json::json!({})).await;
        let id = stripe(&rooms).await[0].id.clone();
        let cancelled = rooms
            .router
            .runtime_state
            .cancel_queued_prompt(&rooms.peer_room, idle.id(), "client", &id)
            .await
            .expect("the person cancels it");
        assert_eq!(cancelled.prompt.id(), id);
        assert!(stripe(&rooms).await.is_empty());
        assert_eq!(states(&rooms, idle.id()).await, vec!["expired"]);
        assert!(
            !rooms
                .message_was_delivered(&rooms.peer_room, idle.id())
                .await
        );
        let again = rooms
            .router
            .runtime_state
            .cancel_queued_prompt(&rooms.peer_room, idle.id(), "client", &id)
            .await;
        assert!(again.is_err(), "it is gone: a second cancel says so");
    });
}

#[test]
fn the_person_sends_a_queued_message_now_and_the_turn_starts() {
    run_peer_test("queued-person-send", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let idle = spawn_waiting(&rooms).await;
        send_to(&rooms, "idle", serde_json::json!({})).await;
        let id = stripe(&rooms).await[0].id.clone();
        rooms
            .router
            .runtime_state
            .steer_queued_prompt_with_external_authority(
                &rooms.peer_room,
                idle.id(),
                "client",
                &id,
                None,
            )
            .await
            .expect("the person sends it now");
        assert!(
            rooms
                .wait_for_message_delivery(&rooms.peer_room, idle.id())
                .await
        );
        assert!(stripe(&rooms).await.is_empty());
    });
}

#[test]
fn the_sender_promotes_its_queued_message_in_place_and_it_is_delivered_once() {
    run_peer_test("queued-promote", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let idle = spawn_waiting(&rooms).await;
        let sent = send_to(&rooms, "idle", serde_json::json!({})).await;
        let promoted = rooms.promote(sent["message_id"].as_str().unwrap()).await;
        assert_eq!(promoted["status"], "promoted");
        assert!(
            rooms
                .wait_for_message_delivery(&rooms.peer_room, idle.id())
                .await
        );
        assert!(stripe(&rooms).await.is_empty(), "the stripe is gone");
        assert_eq!(
            rooms.inbox(&rooms.peer_room, idle.id()).await.len(),
            1,
            "the same message, not a second one"
        );
        let again = rooms.promote(sent["message_id"].as_str().unwrap()).await;
        assert_eq!(again["status"], "already_delivered");
        assert_eq!(rooms.inbox(&rooms.peer_room, idle.id()).await.len(), 1);
    });
}

#[test]
fn sending_the_same_key_again_as_urgent_promotes_the_queued_message() {
    run_peer_test("queued-resend-urgent", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let idle = spawn_waiting(&rooms).await;
        let key = serde_json::json!({"idempotency_key": "notes-1"});
        let queued = send_to(&rooms, "idle", key.clone()).await;
        let plain = send_to(&rooms, "idle", key).await;
        assert_eq!(plain, queued, "a repeat is answered as the first send was");
        assert_eq!(plain["sequence"], queued["sequence"]);
        let promoted = send_to(
            &rooms,
            "idle",
            serde_json::json!({"idempotency_key": "notes-1", "urgent": true}),
        )
        .await;
        assert_eq!(promoted["status"], "started");
        assert_eq!(promoted["sequence"], queued["sequence"]);
        assert!(
            rooms
                .wait_for_message_delivery(&rooms.peer_room, idle.id())
                .await
        );
        assert_eq!(rooms.inbox(&rooms.peer_room, idle.id()).await.len(), 1);
    });
}

#[test]
fn an_urgent_resend_of_a_message_already_read_says_it_was_delivered() {
    run_peer_test("queued-resend-after-read", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let idle = spawn_idle(&rooms).await;
        let key = serde_json::json!({"idempotency_key": "notes-2"});
        let queued = send_to(&rooms, "idle", key).await;
        // The recipient read and acknowledged it in a turn that gave it no prompt of its own.
        rooms
            .router
            .runtime_state
            .ack_agent_inbox(
                &rooms.peer_room,
                idle.id(),
                &serde_json::json!({"sequence": queued["sequence"], "handled": true}),
                1,
            )
            .await
            .expect("the recipient acknowledges it");
        let resent = send_to(
            &rooms,
            "idle",
            serde_json::json!({"idempotency_key": "notes-2", "urgent": true}),
        )
        .await;
        assert_eq!(
            resent["status"], "already_delivered",
            "a consumed message is never reported as still queued: {resent}"
        );
        assert_eq!(resent["sequence"], queued["sequence"]);
    });
}

#[test]
fn only_the_sender_promotes_a_queued_message() {
    run_peer_test("queued-promote-sender-only", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let idle = spawn_waiting(&rooms).await;
        let sent = send_to(&rooms, "idle", serde_json::json!({})).await;
        let (stranger_token, _) = {
            let mut app = rooms.app.lock().await;
            let stranger = spawn_with_level(
                &mut app,
                &rooms.home,
                "stranger",
                AgentPermissionLevel::Yolo,
            );
            running_turn(&mut app, &rooms.home, stranger.id())
        };
        let refused = rooms
            .router
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(
                &stranger_token,
                crate::transport::runtime_tools::PROMOTE_AGENT_MESSAGE_TOOL,
                serde_json::json!({"message_id": sent["message_id"]}),
            )
            .await;
        assert!(
            refused.is_err() || !refused.unwrap().ok,
            "another agent cannot promote it"
        );
        assert!(
            !rooms
                .message_was_delivered(&rooms.peer_room, idle.id())
                .await
        );
        assert_eq!(stripe(&rooms).await.len(), 1);
    });
}

#[test]
fn a_promotion_after_the_person_cancelled_says_so() {
    run_peer_test("queued-promote-after-cancel", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let idle = spawn_waiting(&rooms).await;
        let sent = send_to(&rooms, "idle", serde_json::json!({})).await;
        let id = stripe(&rooms).await[0].id.clone();
        rooms
            .router
            .runtime_state
            .cancel_queued_prompt(&rooms.peer_room, idle.id(), "client", &id)
            .await
            .expect("the person cancels it");
        let promoted = rooms.promote(sent["message_id"].as_str().unwrap()).await;
        assert_eq!(promoted["status"], "withdrawn");
        assert!(
            !rooms
                .message_was_delivered(&rooms.peer_room, idle.id())
                .await
        );
    });
}

/// The Room's state as `GetSessionState` answers it: its size in bytes and its JSON.
async fn session_state(rooms: &Rooms, command: &str) -> (usize, serde_json::Value) {
    let request = LocalDaemonRequest::GetSessionState(crate::local::GetSessionStateRequest {
        session_id: rooms.peer_room.clone(),
    });
    let command = KernelCommand::from_local_request(command, None, None, &request);
    let response = rooms
        .router
        .dispatch(command, request)
        .await
        .expect("the room state");
    let bytes = serde_json::to_vec(&response).unwrap().len();
    (bytes, serde_json::to_value(&response).unwrap())
}

/// What a client opening the Room pays: the state plus the latest turns.
async fn attach_view_size(rooms: &Rooms, client: &str) -> usize {
    let request = LocalDaemonRequest::AttachSessionView(crate::local::AttachSessionViewRequest {
        session_id: rooms.peer_room.clone(),
        client_id: client.into(),
        capability_level: crate::attachment::ClientCapabilityLevel::FullTerminal,
        preferred_agent_id: None,
        latest_prompt_count: None,
    });
    let command = KernelCommand::from_local_request(client, None, None, &request);
    let response = rooms
        .router
        .dispatch(command, request)
        .await
        .expect("the attach view");
    assert!(matches!(
        response,
        crate::local::LocalDaemonResponse::SessionViewAttached { .. }
    ));
    serde_json::to_vec(&response).unwrap().len()
}

/// Sends `count` long messages to each of the two agents, numbered from `first`.
async fn send_long_messages(rooms: &Rooms, first: usize, count: usize) {
    for (index, agent) in std::iter::repeat_n(["idle", "peer"], count)
        .flatten()
        .enumerate()
    {
        let message = format!("{:02} {}", first * 2 + index, "x".repeat(3997));
        send_to(rooms, agent, serde_json::json!({ "message": message })).await;
    }
}

/// A Room whose agents queue many long messages still answers its state in a small read: the session
/// shows a preview of the first few per agent, and the recipient reads the whole text from its inbox.
/// What the queue adds is what the budgets bound: the Room's state without any queued message is
/// the fixed cost of its agents (a turn, a parked task and its wake), which does not grow with the
/// messages. So each budget is read on the growth the messages cause, and that growth must not grow
/// with their number: twice the messages cost the same.
#[test]
fn many_long_queued_messages_keep_the_session_state_small() {
    run_peer_test("queued-bounded", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let idle = spawn_waiting(&rooms).await;
        let (state_empty, _) = session_state(&rooms, "cmd-queued-empty").await;
        let attach_empty = attach_view_size(&rooms, "perf-budget-empty").await;

        send_long_messages(&rooms, 0, 35).await;
        let (state_full, response) = session_state(&rooms, "cmd-queued-bounded").await;
        let queued = response["SessionState"]["session"]["queued_agent_messages"]
            .as_array()
            .expect("the queued messages");
        let budget = crate::transport::perf_budget::budget("session_state_bytes");
        let growth = state_full - state_empty;
        assert!(
            growth <= budget,
            "35 queued messages per agent add {growth} bytes ({state_empty} without them, {state_full} with), over kernel.session_state_bytes {budget}"
        );
        for agent in [idle.id(), rooms.peer.id()] {
            let shown: Vec<_> = queued
                .iter()
                .filter(|message| message["agent_id"] == agent)
                .collect();
            assert_eq!(shown.len(), 10, "{agent} shows its next ten messages");
            assert!(shown[0]["preview"].as_str().unwrap().starts_with("0"));
            assert_eq!(shown[0]["preview"].as_str().unwrap().chars().count(), 200);
            assert_eq!(shown[0]["truncated"], true);
        }
        assert_eq!(rooms.inbox(&rooms.peer_room, idle.id()).await.len(), 35);

        // A client opening the Room pays one attach view: the state above plus the latest turns.
        let attach_full = attach_view_size(&rooms, "perf-budget").await;
        let attach_growth = attach_full - attach_empty;
        let budget = crate::transport::perf_budget::budget("attach_view_bytes");
        assert!(
            attach_growth <= budget,
            "35 queued messages per agent add {attach_growth} bytes to the attach view ({attach_empty} without them, {attach_full} with), over kernel.attach_view_bytes {budget}"
        );

        // Twice the messages cost the same: the state shows a preview of the first ten per agent.
        send_long_messages(&rooms, 35, 35).await;
        let (state_double, _) = session_state(&rooms, "cmd-queued-double").await;
        let double_growth = state_double - state_empty;
        assert!(
            double_growth <= growth + GROWTH_TOLERANCE,
            "70 queued messages per agent add {double_growth} bytes, 35 add {growth}: the state grows with the number of messages"
        );
        assert_eq!(rooms.inbox(&rooms.peer_room, idle.id()).await.len(), 70);
    });
}

/// The measurement of a Room's state varies by a few bytes (ids and times differ in length).
const GROWTH_TOLERANCE: usize = 64;

#[test]
fn an_unwatched_room_view_attachment_lapses() {
    run_peer_test("unwatched-room-view-attachment", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let request =
            LocalDaemonRequest::AttachSessionView(crate::local::AttachSessionViewRequest {
                session_id: rooms.peer_room.clone(),
                client_id: "unwatched-room-view".into(),
                capability_level: crate::attachment::ClientCapabilityLevel::FullTerminal,
                preferred_agent_id: None,
                latest_prompt_count: None,
            });
        let command =
            KernelCommand::from_local_request("unwatched-room-view", None, None, &request);
        let crate::local::LocalDaemonResponse::SessionViewAttached { attachment, .. } =
            rooms.router.dispatch(command, request).await.unwrap()
        else {
            panic!("the Room view should attach");
        };
        let after_lapse = crate::session::unix_epoch_ms() + 31_000;
        rooms
            .router
            .runtime_state
            .sweep_stale_terminal_attachments(after_lapse)
            .await;
        assert!(matches!(
            rooms
                .router
                .record_terminal_attachment_heartbeat(
                    attachment.session_id(),
                    attachment.id(),
                    after_lapse
                )
                .await,
            Err(crate::error::DaemonError::AttachmentNotFound { .. })
        ));
    });
}

#[test]
fn a_room_with_long_task_history_fits_the_state_budget() {
    run_peer_test("long-room-state", || async {
        use crate::durable_state::agent_lifecycle::Operation;

        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        let room = &rooms.peer_room;
        let mut app = rooms.app.lock().await;
        let present = app
            .agents()
            .get_session_agents(room)
            .into_iter()
            .filter(crate::agent::AgentInstance::is_room_agent)
            .count();
        for index in present..20 {
            spawn_with_level(
                &mut app,
                room,
                &format!("long-room-agent-{index}"),
                AgentPermissionLevel::Yolo,
            );
        }
        let history = app.durable_state_store();
        for index in 0..1000 {
            let prompt = format!("finished-{index}");
            history
                .agent_lifecycle(Operation::Begin {
                    owner: crate::session::DEFAULT_LOCAL_USER_ID.into(),
                    room: room.clone(),
                    agent: rooms.peer.id().into(),
                    prompt: prompt.clone(),
                    run: Some("fixture-run".into()),
                    now: index,
                })
                .unwrap();
            history
                .agent_lifecycle(Operation::Settle {
                    room: room.clone(),
                    agent: rooms.peer.id().into(),
                    prompt,
                    run: "fixture-run".into(),
                    has_answer: true,
                    cancelled: false,
                    now: index,
                })
                .unwrap();
        }
        for index in 0..24 {
            app.sessions_mut()
                .create_workflow(
                    room,
                    Some(format!("workflow-{index}-{}", "x".repeat(24_000))),
                )
                .unwrap();
        }
        drop(app);

        let request = LocalDaemonRequest::GetSessionState(crate::local::GetSessionStateRequest {
            session_id: room.clone(),
        });
        let command = KernelCommand::from_local_request("long-room-state", None, None, &request);
        let response = rooms
            .router
            .dispatch(command, request)
            .await
            .expect("read Room state");
        let bytes = serde_json::to_vec(&response).unwrap().len();
        let budget = crate::transport::perf_budget::budget("long_room_state_bytes");
        assert!(
            bytes <= budget,
            "Room state is {bytes} bytes, over kernel.long_room_state_bytes {budget}"
        );
        let value = serde_json::to_value(response).unwrap();
        let state = &value["SessionState"];
        assert_eq!(state["session"]["agents"].as_array().unwrap().len(), 20);
        assert_eq!(state["room_workflows"]["workflow_count"], 24);
        assert!(state["session"].get("workflows").is_none());
        assert!(state["session"].get("attachment_ids").is_none());
        assert!(state.get("disk_usage").is_none());
        assert_eq!(
            history
                .agent_tasks(Some(room), Some(rooms.peer.id()))
                .unwrap()
                .into_iter()
                .filter(|task| task.state
                    == crate::durable_state::agent_lifecycle::ExecutionState::Done)
                .count(),
            1000
        );
    });
}

#[test]
fn receipt_unconfirmed_inline_message_waits_for_local_provider_acceptance() {
    run_peer_test("receipt-inline-local-held", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        rooms.use_receipt_provider("slow-structured").await;
        let key = serde_json::json!({"urgent":true,"idempotency_key":"inline-held"});
        let sent = send_to(&rooms, "peer", key.clone()).await;
        let sequence = sent["sequence"].as_u64().unwrap();
        let (event, read) = rooms.inline_message_row(sequence).await;
        assert!(
            read,
            "the message text was included in the wake before acceptance"
        );
        assert_eq!(event.state, "submitting");
        assert_eq!(
            sent["status"], "queued",
            "inline text is not provider acceptance: {sent}"
        );
        assert!(sent["note"]
            .as_str()
            .unwrap()
            .contains("not been confirmed"));
        let resent = send_to(&rooms, "peer", key.clone()).await;
        assert_eq!(resent["sequence"], sequence);
        assert_eq!(resent["status"], "queued");
        rooms.wait_for_message_state(sequence, "accepted").await;
        let accepted = send_to(&rooms, "peer", key).await;
        assert_eq!(accepted["sequence"], sequence);
        assert!(matches!(
            accepted["status"].as_str(),
            Some("steered" | "started")
        ));
        assert_eq!(
            rooms.inbox(&rooms.peer_room, rooms.peer.id()).await.len(),
            1
        );
    });
}

#[test]
fn receipt_unconfirmed_inline_message_keeps_a_lost_local_ack_unconfirmed() {
    run_peer_test("receipt-inline-local-lost-ack", || async {
        let rooms = Rooms::new(AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
        rooms.use_receipt_provider("lost-ack-structured").await;
        let key = serde_json::json!({"urgent":true,"idempotency_key":"inline-lost"});
        let sent = send_to(&rooms, "peer", key.clone()).await;
        let sequence = sent["sequence"].as_u64().unwrap();
        rooms.wait_for_message_state(sequence, "uncertain").await;
        let (event, read) = rooms.inline_message_row(sequence).await;
        assert!(read, "the inline wake was prepared");
        assert_eq!(event.state, "uncertain");
        assert_eq!(
            sent["status"], "queued",
            "a lost acknowledgement is unconfirmed: {sent}"
        );
        let resent = send_to(&rooms, "peer", key).await;
        assert_eq!(resent["sequence"], sequence);
        assert_eq!(resent["status"], "queued");
        assert_eq!(
            rooms.inbox(&rooms.peer_room, rooms.peer.id()).await.len(),
            1
        );
    });
}
