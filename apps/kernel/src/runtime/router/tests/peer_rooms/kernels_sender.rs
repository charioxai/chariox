//! Peers across kernels, end to end: a room agent on kernel A spawns, messages, steers, cancels
//! and answers an ordinary agent of a room on kernel B through its own tools, and B answers it
//! the same way. The two kernels share an owner; the relay between them is the in-process link.
use super::*;
use crate::runtime::state::peer_agent_outbound::peer_agent_link_for_tests;
use crate::transport::relay_peer::RelayPeerRequest;

mod created_peers;
mod follow;
mod machine_files;

const OWNER: &str = "owner-p5b";

/// One kernel: its id, name and one room with a running room agent.
struct Kernel {
    id: String,
    name: String,
    app: Arc<Mutex<DaemonApp>>,
    router: CommandRouter,
    room: String,
    agent: crate::agent::AgentInstance,
    token: String,
    prompt: String,
}

fn kernel(id: &str, level: AgentPermissionLevel, signed_in: bool) -> Kernel {
    kernel_with_room_of(id, level, signed_in, OWNER)
}

/// A kernel signed in as `OWNER` (or not) whose room and agent are `room_owner`'s: `local` for
/// a room made before the kernel was enrolled.
fn kernel_with_room_of(
    id: &str,
    level: AgentPermissionLevel,
    signed_in: bool,
    room_owner: &str,
) -> Kernel {
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    config.daemon_id = id.into();
    if signed_in {
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            user_id: OWNER.into(),
            ..Default::default()
        });
    }
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let room = create_room(&mut app, &format!("{id}-room"), Some(room_owner));
    let agent = owned_agent(&mut app, &room, "worker", room_owner, level);
    let (token, prompt) = running_turn(&mut app, &room, agent.id());
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(app.clone(), 4);
    Kernel {
        id: id.into(),
        name: format!("{id}-name"),
        app,
        router,
        room,
        agent,
        token,
        prompt,
    }
}

fn owned_agent(
    app: &mut DaemonApp,
    room: &str,
    alias: &str,
    owner: &str,
    level: AgentPermissionLevel,
) -> crate::agent::AgentInstance {
    crate::app::KernelSessionService::new(app)
        .spawn_agent(
            CreateAgentRequest::new(room, "dev-stub")
                .with_alias(alias)
                .with_owner_user_id(owner)
                .with_permission_level_override(level),
        )
        .expect("agent should spawn")
}

/// `kernel` answers peer requests from the other kernels of this test under its id and name.
fn link(kernel: &Kernel) {
    let router = kernel.router.clone();
    peer_agent_link_for_tests::register(
        &kernel.id,
        &kernel.name,
        Arc::new(
            move |from: String, owner: String, request: RelayPeerRequest| {
                let router = router.clone();
                Box::pin(async move { router.relay_peer_agent(&from, &owner, request).await })
            },
        ),
    );
}

impl Kernel {
    async fn send(&self, args: serde_json::Value) -> Result<serde_json::Value, String> {
        self.send_as(&self.token, &self.prompt, args).await
    }

    async fn send_as(
        &self,
        token: &str,
        prompt: &str,
        mut args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        args["origin_prompt_id"] = prompt.into();
        match self
            .router
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(
                token,
                crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL,
                args,
            )
            .await
        {
            Ok(result) if result.ok => Ok(result.payload),
            Ok(result) => Err(result.payload.to_string()),
            Err(error) => Err(error.to_string()),
        }
    }

    async fn promote(&self, message_id: &str) -> Result<serde_json::Value, String> {
        match self
            .router
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(
                &self.token,
                crate::transport::runtime_tools::PROMOTE_AGENT_MESSAGE_TOOL,
                serde_json::json!({ "message_id": message_id }),
            )
            .await
        {
            Ok(result) if result.ok => Ok(result.payload),
            Ok(result) => Err(result.payload.to_string()),
            Err(error) => Err(error.to_string()),
        }
    }

    async fn command(&self, command: String) -> Result<serde_json::Value, String> {
        self.command_as(&self.token, command).await
    }

    async fn command_as(&self, token: &str, command: String) -> Result<serde_json::Value, String> {
        let result = self
            .router
            .dispatch_authenticated_runtime_tool_call(
                token,
                "chariox.room.run_command",
                serde_json::json!({ "command": command }),
            )
            .await
            .map_err(|error| error.to_string())?;
        if result.ok {
            Ok(result.payload)
        } else {
            Err(result.payload.to_string())
        }
    }

    async fn messages(
        &self,
        agent: &str,
    ) -> Vec<crate::durable_state::agent_lifecycle::InboxEvent> {
        self.app
            .lock()
            .await
            .durable_state_store()
            .agent_inbox(&self.room, agent, 0)
            .unwrap()
            .into_iter()
            .filter(|event| event.kind == "message")
            .collect()
    }

    async fn turn_running(&self, agent: &str) -> bool {
        self.app
            .lock()
            .await
            .sessions()
            .get_session(&self.room)
            .unwrap()
            .active_prompt_for_agent(agent)
            .is_some_and(|prompt| prompt.status() == PromptStatus::Running)
    }
}

/// Kernel A (the sender) and kernel B (the target), both linked.
fn two_kernels(
    test: &str,
    sender: AgentPermissionLevel,
    target: AgentPermissionLevel,
) -> (Kernel, Kernel) {
    let a = kernel(&format!("{test}-a"), sender, true);
    let b = kernel(&format!("{test}-b"), target, true);
    link(&a);
    link(&b);
    (a, b)
}

fn unlink(kernels: &[&Kernel]) {
    for kernel in kernels {
        peer_agent_link_for_tests::unregister(&kernel.id);
    }
}

#[test]
fn a_room_agent_messages_steers_and_cancels_a_peer_on_another_kernel_by_its_name() {
    run_peer_test("peer-kernel-send", || async {
        let (a, b) = two_kernels(
            "peer-kernel-send",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        let sent = a
            .send(serde_json::json!({
                "agent": "worker",
                "room": b.room,
                "kernel": b.name,
                "message": "parser.rs is mine for the next hour",
                "idempotency_key": "message-1",
            }))
            .await
            .expect("a message reaches a peer on another kernel");
        assert_eq!(sent["kernel"], b.id.as_str());
        assert_eq!(sent["agent"], b.agent.id());
        let steer = a
            .send(serde_json::json!({
                "agent": "worker",
                "room": b.room,
                "kernel": b.id,
                "message": "stop editing parser.rs",
                "urgent": true,
            }))
            .await;
        assert!(steer.is_ok(), "{steer:?}");
        let inbox = b.messages(b.agent.id()).await;
        assert_eq!(inbox.len(), 2, "{inbox:?}");
        assert!(!inbox[0].urgent && inbox[1].urgent);
        for event in &inbox {
            assert_eq!(event.payload["sender_kernel_id"], a.id.as_str());
            assert_eq!(event.payload["sender_agent_id"], a.agent.id());
            assert_eq!(event.payload["sender_room_id"], a.room.as_str());
        }

        assert!(b.turn_running(b.agent.id()).await);
        let cancel = a
            .command(format!(
                "agent cancel worker --room {} --kernel {}",
                b.room, b.name
            ))
            .await;
        assert!(cancel.is_ok(), "{cancel:?}");
        assert!(!b.turn_running(b.agent.id()).await, "the peer's turn stops");
        assert!(
            b.app.lock().await.agents().get_agent(b.agent.id()).is_ok(),
            "the peer stays"
        );
        unlink(&[&a, &b]);
    });
}

#[test]
fn a_retried_message_to_a_peer_on_another_kernel_is_reported_duplicate_and_lands_once() {
    run_peer_test("peer-kernel-duplicate", || async {
        let (a, b) = two_kernels(
            "peer-kernel-duplicate",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        let message = || {
            serde_json::json!({
                "agent": "worker",
                "room": b.room,
                "kernel": b.name,
                "message": "parser.rs is mine for the next hour",
                "idempotency_key": "retry-1",
            })
        };
        let first = a
            .send(message())
            .await
            .expect("the first send is delivered");
        assert_eq!(first["status"], "queued");
        let again = a.send(message()).await.expect("the retry is answered");
        assert_eq!(again["status"], "duplicate", "{again:?}");
        assert_eq!(b.messages(b.agent.id()).await.len(), 1);
        unlink(&[&a, &b]);
    });
}

#[test]
fn a_reply_request_to_another_kernel_is_refused_before_anything_is_sent() {
    run_peer_test("peer-kernel-reply-request", || async {
        let (a, b) = two_kernels(
            "peer-kernel-reply-request",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        let message = |reply_requested: bool| {
            serde_json::json!({
                "agent": "worker",
                "room": b.room,
                "kernel": b.name,
                "message": "which tests cover parser.rs?",
                "reply_requested": reply_requested,
                "idempotency_key": "ask-1",
            })
        };
        let refused = a.send(message(true)).await.unwrap_err();
        assert!(
            refused.contains(
                "a reply to a message on another kernel is not supported yet: ask the agent there to message you back"
            ),
            "{refused}"
        );
        assert!(b.messages(b.agent.id()).await.is_empty());
        // The other kernel never saw the call: the same key is a first delivery, not a duplicate.
        let sent = a
            .send(message(false))
            .await
            .expect("a plain message is sent");
        assert_eq!(sent["status"], "queued", "{sent:?}");
        assert_eq!(b.messages(b.agent.id()).await.len(), 1);
        unlink(&[&a, &b]);
    });
}

#[test]
fn a_peer_on_another_kernel_answers_by_messaging_the_sender_back() {
    run_peer_test("peer-kernel-reply", || async {
        let (a, b) = two_kernels(
            "peer-kernel-reply",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        a.send(serde_json::json!({
            "agent": "worker",
            "room": b.room,
            "kernel": b.name,
            "message": "which tests cover parser.rs? message me back",
        }))
        .await
        .expect("the question reaches the peer");
        let asked = b.messages(b.agent.id()).await;
        assert_eq!(asked.len(), 1);
        let (kernel, room, agent) = (
            asked[0].payload["sender_kernel_id"]
                .as_str()
                .unwrap()
                .to_owned(),
            asked[0].payload["sender_room_id"]
                .as_str()
                .unwrap()
                .to_owned(),
            asked[0].payload["sender_agent_id"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
        b.send(serde_json::json!({
            "agent": agent,
            "room": room,
            "kernel": kernel,
            "message": "parser_tests.rs and lexer_tests.rs",
        }))
        .await
        .expect("the reply goes back to the sender's kernel");
        let replies = a.messages(a.agent.id()).await;
        assert_eq!(replies.len(), 1, "{replies:?}");
        assert_eq!(replies[0].payload["sender_kernel_id"], b.id.as_str());
        assert_eq!(replies[0].payload["sender_agent_id"], b.agent.id());

        // No outbox: with the sender's kernel gone, the reply fails where the replier sees it.
        unlink(&[&a]);
        let lost = b
            .send(serde_json::json!({
                "agent": a.agent.id(),
                "room": a.room,
                "kernel": a.id,
                "message": "one more thing",
            }))
            .await
            .unwrap_err();
        assert!(lost.contains("kernel_unreachable"), "{lost}");
        unlink(&[&b]);
    });
}

#[test]
fn a_room_agent_spawns_one_ordinary_peer_in_a_room_on_another_kernel() {
    run_peer_test("peer-kernel-spawn", || async {
        let (a, b) = two_kernels(
            "peer-kernel-spawn",
            AgentPermissionLevel::Required,
            AgentPermissionLevel::Yolo,
        );
        let spawned = a
            .command(format!(
                "agent spawn reviewer --room {} --kernel {} --no-follow --permission yolo",
                b.room, b.name
            ))
            .await
            .expect("a peer spawns on the other kernel");
        let spawned = &spawned["response"]["spawned"];
        assert_eq!(spawned["kernel"], b.id.as_str());
        assert_eq!(spawned["room"], b.room.as_str());
        let peer = b
            .app
            .lock()
            .await
            .agents()
            .get_agent(spawned["agent"].as_str().unwrap())
            .unwrap();
        assert_eq!(peer.alias(), Some("reviewer"));
        assert!(
            peer.parent_agent_id().is_none(),
            "an ordinary agent of that room"
        );
        let level = {
            let app = b.app.lock().await;
            crate::session::effective_agent_execution_config_in_tree(
                &app.sessions().get_session(&b.room).unwrap(),
                &peer,
                &app.agents().get_session_agents(&b.room),
            )
            .permission_level
        };
        assert_eq!(
            level,
            AgentPermissionLevel::Required,
            "never more authority than its creator"
        );
        unlink(&[&a, &b]);
    });
}

/// `kernel` acts on every request but its answer to the first spawn is lost on the way back.
fn link_losing_the_first_spawn_answer(kernel: &Kernel) {
    let router = kernel.router.clone();
    let lost = Arc::new(std::sync::atomic::AtomicBool::new(false));
    peer_agent_link_for_tests::register(
        &kernel.id,
        &kernel.name,
        Arc::new(
            move |from: String, owner: String, request: RelayPeerRequest| {
                let router = router.clone();
                let lost = lost.clone();
                Box::pin(async move {
                    let spawn = matches!(request, RelayPeerRequest::SpawnRoomPeer { .. });
                    let answer = router.relay_peer_agent(&from, &owner, request).await;
                    if spawn && !lost.swap(true, std::sync::atomic::Ordering::SeqCst) {
                        return Err(crate::runtime::room_tool_admission::denied(
                            "kernel_unreachable: the answer was lost",
                        ));
                    }
                    answer
                })
            },
        ),
    );
}

#[test]
fn a_retried_spawn_on_another_kernel_returns_the_same_peer() {
    run_peer_test("peer-kernel-spawn-retry", || async {
        let (a, b) = two_kernels(
            "peer-kernel-spawn-retry",
            AgentPermissionLevel::Required,
            AgentPermissionLevel::Yolo,
        );
        link_losing_the_first_spawn_answer(&b);
        let spawn = format!(
            "agent spawn reviewer --room {} --kernel {} --no-follow",
            b.room, b.name
        );
        let lost = a.command(spawn.clone()).await.unwrap_err();
        assert!(lost.contains("kernel_unreachable"), "{lost}");
        let retried = a
            .command(spawn)
            .await
            .expect("the retry answers the peer the lost answer named");
        assert_eq!(retried["response"]["spawned"]["duplicate"], true);
        let reviewers = |agents: Vec<crate::agent::AgentInstance>, alias: &str| {
            agents
                .into_iter()
                .filter(|agent| agent.alias() == Some(alias))
                .map(|agent| agent.id().to_string())
                .collect::<Vec<_>>()
        };
        let on_b = reviewers(
            b.app.lock().await.agents().get_session_agents(&b.room),
            "reviewer",
        );
        assert_eq!(on_b.len(), 1, "one peer for one spawn call: {on_b:?}");
        assert_eq!(retried["response"]["spawned"]["agent"], on_b[0].as_str());

        // A command id names the call across turns: the same id is the same peer.
        let keyed = format!(
            "agent spawn tester --room {} --kernel {} --no-follow --command-id spawn-tester-1",
            b.room, b.name
        );
        let first = a.command(keyed.clone()).await.expect("a keyed spawn");
        assert_eq!(first["response"]["spawned"]["duplicate"], false);
        let again = a.command(keyed).await.expect("the same keyed spawn");
        assert_eq!(again["response"]["spawned"]["duplicate"], true);
        assert_eq!(
            again["response"]["spawned"]["agent"],
            first["response"]["spawned"]["agent"]
        );
        let testers = reviewers(
            b.app.lock().await.agents().get_session_agents(&b.room),
            "tester",
        );
        assert_eq!(testers.len(), 1, "{testers:?}");

        // A command id names a cross-kernel call only; a peer on this kernel keeps the refusal.
        let local = a
            .command(format!(
                "agent spawn other --room {} --kernel {} --no-follow --command-id spawn-local-1",
                a.room, a.name
            ))
            .await
            .unwrap_err();
        assert!(local.contains("--command-id"), "{local}");
        unlink(&[&a, &b]);
    });
}

#[test]
fn a_room_on_another_kernel_is_named_by_its_alias_there() {
    run_peer_test("peer-kernel-room-alias", || async {
        let (a, b) = two_kernels(
            "peer-kernel-room-alias",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        let qa = {
            let mut app = b.app.lock().await;
            let request = CreateSessionRequest::new(
                "peer-kernel-room-alias-qa",
                std::env::temp_dir().to_string_lossy(),
            )
            .with_alias("qa-room")
            .with_owner_user_id(OWNER);
            let (room, _) = crate::app::KernelSessionService::new(&mut app)
                .create_session(request)
                .expect("room should create");
            room.id().to_string()
        };
        let spawned = a
            .command(format!(
                "agent spawn reviewer --room qa-room --kernel {} --no-follow",
                b.name
            ))
            .await
            .expect("a peer spawns in the room the alias names");
        let spawned = &spawned["response"]["spawned"];
        assert_eq!(spawned["room"], qa.as_str());
        let reviewers = b
            .app
            .lock()
            .await
            .agents()
            .get_session_agents(&qa)
            .into_iter()
            .filter(|agent| agent.alias() == Some("reviewer"))
            .count();
        assert_eq!(reviewers, 1);

        a.command(format!(
            "agent message reviewer \"take the parser\" --room qa-room --kernel {}",
            b.name
        ))
        .await
        .expect("a message reaches the peer in the room the alias names");
        let inbox = b
            .app
            .lock()
            .await
            .durable_state_store()
            .agent_inbox(&qa, spawned["agent"].as_str().unwrap(), 0)
            .unwrap()
            .into_iter()
            .filter(|event| event.kind == "message")
            .count();
        assert_eq!(inbox, 1);
        unlink(&[&a, &b]);
    });
}

#[test]
fn every_cross_kernel_refusal_is_typed_and_tells_nothing_about_what_the_caller_cannot_address() {
    run_peer_test("peer-kernel-refusals", || async {
        let (a, b) = two_kernels(
            "peer-kernel-refusals",
            AgentPermissionLevel::Required,
            AgentPermissionLevel::Yolo,
        );
        let foreign = {
            let mut app = b.app.lock().await;
            let room = create_room(&mut app, "foreign-on-b", Some("another-owner"));
            owned_agent(
                &mut app,
                &room,
                "worker",
                "another-owner",
                AgentPermissionLevel::Required,
            );
            room
        };
        let message = |kernel: &str, room: &str, agent: &str| serde_json::json!({"agent": agent, "room": room, "kernel": kernel, "message": "hello"});
        let refused = |result: Result<serde_json::Value, String>| result.unwrap_err();

        let unknown_kernel = refused(a.send(message("no-such-kernel", &b.room, "worker")).await);
        assert!(
            unknown_kernel.contains("kernel_unreachable"),
            "{unknown_kernel}"
        );
        let unknown_room = refused(a.send(message(&b.name, "no-such-room", "worker")).await);
        let foreign_room = refused(a.send(message(&b.name, &foreign, "worker")).await);
        assert!(unknown_room.contains("room_unavailable"), "{unknown_room}");
        assert_eq!(unknown_room, foreign_room, "not yours reads as unknown");
        let unknown_agent = refused(a.send(message(&b.name, &b.room, "nobody")).await);
        assert!(
            unknown_agent.contains("agent_unavailable"),
            "{unknown_agent}"
        );

        let steer = refused(
            a.send(serde_json::json!({"agent": "worker", "room": b.room, "kernel": b.name, "message": "stop", "urgent": true}))
                .await,
        );
        assert!(steer.contains("not_permitted"), "{steer}");
        let cancel = a
            .command(format!(
                "agent cancel worker --room {} --kernel {}",
                b.room, b.name
            ))
            .await
            .unwrap_err();
        assert!(cancel.contains("not_permitted"), "{cancel}");
        assert!(b.turn_running(b.agent.id()).await);

        let attachments = refused(
            a.send(serde_json::json!({
                "agent": "worker", "room": b.room, "kernel": b.name, "message": "see file",
                "attachments": [{"url": "file:///tmp/a.txt", "mime": "text/plain"}],
            }))
            .await,
        );
        assert!(
            attachments.contains("attachments_local_only"),
            "{attachments}"
        );

        // A kernel that is not signed in to the owner's Cloud account cannot vouch for its caller.
        let unsigned = kernel("peer-kernel-refusals-c", AgentPermissionLevel::Yolo, false);
        let owner = refused(unsigned.send(message(&b.name, &b.room, "worker")).await);
        assert!(owner.contains("owner_unverified"), "{owner}");
        assert!(
            b.messages(b.agent.id()).await.is_empty(),
            "nothing was delivered"
        );
        unlink(&[&a, &b]);
    });
}

#[test]
fn a_guest_reaches_only_the_agents_of_its_own_room() {
    run_peer_test("peer-guest", || async {
        let (a, b) = two_kernels(
            "peer-guest",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        // On kernel A: another room of the owner with an agent, and a guest of the home room
        // who owns one agent there and drives the owner's other agent.
        let (other_room, guest_owned, guest_driven) = {
            let mut app = a.app.lock().await;
            let other_room = create_room(&mut app, "kernel-a-other", Some(OWNER));
            owned_agent(
                &mut app,
                &other_room,
                "neighbour",
                OWNER,
                AgentPermissionLevel::Required,
            );
            let mut room = app.sessions().get_session(&a.room).unwrap();
            room.add_member(
                "guest-user",
                Some(OWNER.into()),
                crate::session::CollaborationLevel::Full,
            );
            app.sessions_mut().restore_session(room);
            let owned = owned_agent(
                &mut app,
                &a.room,
                "guest-own",
                "guest-user",
                AgentPermissionLevel::Yolo,
            );
            let owned_turn = running_turn(&mut app, &a.room, owned.id());
            let driven = owned_agent(
                &mut app,
                &a.room,
                "guest-driven",
                OWNER,
                AgentPermissionLevel::Yolo,
            );
            let driven_turn = guest_turn(&mut app, &a.room, driven.id(), "guest-user");
            (other_room, (owned, owned_turn), (driven, driven_turn))
        };
        let unknown = a
            .send(serde_json::json!({"agent": "nobody", "room": other_room, "message": "hello"}))
            .await
            .unwrap_err();
        let unknown_on_b = a
            .send(serde_json::json!({"agent": "worker", "room": "no-such-room", "kernel": b.name, "message": "hello"}))
            .await
            .unwrap_err();
        for (guest, (token, prompt)) in [&guest_driven, &guest_owned] {
            let alias = guest.alias().unwrap();
            let local = a
                .send_as(
                    token,
                    prompt,
                    serde_json::json!({"agent": "neighbour", "room": other_room, "message": "hello"}),
                )
                .await
                .unwrap_err();
            assert_eq!(local, unknown, "{alias}: another room reads as unknown");
            let across = a
                .send_as(
                    token,
                    prompt,
                    serde_json::json!({"agent": "worker", "room": b.room, "kernel": b.name, "message": "hello"}),
                )
                .await
                .unwrap_err();
            assert_eq!(
                across, unknown_on_b,
                "{alias}: a room on another kernel reads as unknown"
            );
            let cancel = a
                .command_as(token, format!("agent cancel neighbour --room {other_room}"))
                .await
                .unwrap_err();
            assert!(
                cancel.contains("agent is unavailable in that room"),
                "{alias}: {cancel}"
            );
            let spawn = a
                .command_as(token, format!("agent spawn helper --room {other_room}"))
                .await
                .unwrap_err();
            assert!(spawn.contains("does not exist"), "{alias}: {spawn}");
            let spawn_on_b = a
                .command_as(
                    token,
                    format!(
                        "agent spawn helper --room {} --kernel {} --no-follow",
                        b.room, b.name
                    ),
                )
                .await
                .unwrap_err();
            assert!(
                spawn_on_b.contains("room_unavailable"),
                "{alias}: {spawn_on_b}"
            );
            // Its own room stays open to it.
            a.send_as(
                token,
                prompt,
                serde_json::json!({"agent": "worker", "message": "hello from the guest"}),
            )
            .await
            .expect("a guest reaches the agents of its own room");
        }
        assert!(b.messages(b.agent.id()).await.is_empty());
        // The owner's own agent still reaches both.
        a.send(serde_json::json!({"agent": "neighbour", "room": other_room, "message": "hello"}))
            .await
            .expect("the owner's agent reaches another room");
        unlink(&[&a, &b]);
    });
}

/// A running turn of `agent` that the room member `user` started.
fn guest_turn(app: &mut DaemonApp, room: &str, agent: &str, user: &str) -> (String, String) {
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                room, "dev-stub", "dev-stub", "default", "model",
            )
            .with_agent_id(agent),
        )
        .unwrap();
    app.update_provider_run_projection(run);
    let attachment = crate::app::KernelSessionService::new(app)
        .attach(crate::attachment::AttachRequest::for_user(
            room,
            format!("guest-client:{agent}"),
            crate::attachment::ClientCapabilityLevel::FullTerminal,
            user,
        ))
        .unwrap();
    app.submit_prompt(
        room,
        attachment.id(),
        Some(agent),
        "work on parser.rs",
        Vec::new(),
    )
    .unwrap();
    let session = app.sessions().get_session(room).unwrap();
    let prompt = session
        .active_prompt_for_agent(agent)
        .expect("the guest's turn is running");
    assert_eq!(prompt.source_user_id(), Some(user));
    let token = app
        .providers()
        .get_run_for_agent(room, agent)
        .unwrap()
        .runtime_mcp_auth_token()
        .expect("runtime MCP auth")
        .to_string();
    (token, prompt.id().to_string())
}

#[test]
fn the_message_and_steer_room_commands_reach_a_peer_on_another_kernel_with_typed_refusals() {
    run_peer_test("peer-kernel-room-commands", || async {
        let (a, b) = two_kernels(
            "peer-kernel-room-commands",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        let refused = a
            .command(format!(
                "agent message worker \"parser.rs is mine\" --room {} --kernel {} --reply-requested",
                b.room, b.name
            ))
            .await
            .unwrap_err();
        assert!(
            refused.contains(
                "a reply to a message on another kernel is not supported yet: ask the agent there to message you back"
            ),
            "{refused}"
        );
        a.command(format!(
            "agent message worker \"parser.rs is mine\" --room {} --kernel {}",
            b.room, b.name
        ))
        .await
        .expect("a message reaches the peer");
        a.command(format!(
            "agent steer worker \"stop editing\" --room {} --kernel {}",
            b.room, b.id
        ))
        .await
        .expect("a steer reaches the peer");
        let inbox = b.messages(b.agent.id()).await;
        assert_eq!(inbox.len(), 2, "{inbox:?}");
        assert!(!inbox[0].urgent && inbox[0].payload["no_reply"] == true);
        assert!(inbox[1].urgent && inbox[1].payload["no_reply"] == true);
        for event in &inbox {
            assert_eq!(event.payload["sender_kernel_id"], a.id.as_str());
            assert_eq!(event.payload["sender_agent_id"], a.agent.id());
        }
        assert!(b.turn_running(b.agent.id()).await, "no user prompt");

        let unreachable = a
            .command(format!(
                "agent steer worker stop --room {} --kernel no-such-kernel",
                b.room
            ))
            .await
            .unwrap_err();
        assert!(unreachable.contains("kernel_unreachable"), "{unreachable}");
        let no_room = a
            .command(format!(
                "agent message worker hi --room no-such-room --kernel {}",
                b.name
            ))
            .await
            .unwrap_err();
        assert!(no_room.contains("room_unavailable"), "{no_room}");
        let no_room_for_kernel = a
            .command("agent message worker hi --kernel elsewhere".to_string())
            .await
            .unwrap_err();
        assert!(
            no_room_for_kernel.contains("usage:"),
            "{no_room_for_kernel}"
        );
        unlink(&[&a, &b]);
    });
}

#[test]
fn a_message_to_a_peer_on_another_kernel_queues_there_and_its_sender_promotes_it() {
    run_peer_test("peer-kernel-promote", || async {
        let (a, b) = two_kernels(
            "peer-kernel-promote",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        let queued = a
            .send(serde_json::json!({
                "agent": "worker",
                "room": b.room,
                "kernel": b.name,
                "message": "parser.rs is mine for the next hour",
                "idempotency_key": "queued-1",
            }))
            .await
            .expect("a message reaches a peer on another kernel");
        assert_eq!(queued["status"], "queued");
        let id = queued["message_id"]
            .as_str()
            .expect("the queued send names its message");
        let inbox = b.messages(b.agent.id()).await;
        assert_eq!(inbox.len(), 1);
        assert!(!inbox[0].urgent, "it waits for the peer's next turn");
        assert_eq!(inbox[0].state, "pending");

        let promoted = a
            .promote(id)
            .await
            .expect("its sender promotes it on the other kernel");
        assert_eq!(promoted["status"], "promotion_sent");
        let inbox = b.messages(b.agent.id()).await;
        assert_eq!(inbox.len(), 1, "the same message, never a second one");
        assert!(inbox[0].urgent);
        assert_ne!(inbox[0].state, "pending", "the steer was taken at once");

        // Sending the same key again as urgent is the same promotion.
        let again = a
            .send(serde_json::json!({
                "agent": "worker",
                "room": b.room,
                "kernel": b.name,
                "message": "parser.rs is mine for the next hour",
                "idempotency_key": "queued-1",
                "urgent": true,
            }))
            .await
            .expect("the resend is answered");
        assert_eq!(again["status"], "duplicate");
        assert_eq!(b.messages(b.agent.id()).await.len(), 1);

        // A promotion of what the other kernel never held is refused there, not sent as a message.
        let unknown = a
            .promote(&format!(
                "peer:{}:{}:{}:never-sent",
                b.id,
                b.room,
                b.agent.id()
            ))
            .await;
        assert!(unknown.is_err(), "{unknown:?}");
        assert_eq!(b.messages(b.agent.id()).await.len(), 1);
        unlink(&[&a, &b]);
    });
}
