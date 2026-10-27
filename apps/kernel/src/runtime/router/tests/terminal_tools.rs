//! The agent's terminal tools through the runtime MCP dispatch: open, type,
//! wait for the shell's own finish report, the one-driver rule, and the
//! agent's permission level.

use super::*;
use crate::provider::AgentPermissionLevel;
use crate::transport::runtime_tools::RuntimeToolResult;

fn run<F: std::future::Future<Output = ()> + 'static>(name: &str, test: fn() -> F) {
    std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(64 * 1024 * 1024)
                .enable_all()
                .build()
                .expect("test runtime")
                .block_on(test());
        })
        .expect("test thread")
        .join()
        .expect("test thread should not panic");
}

struct Room {
    router: CommandRouter,
    session_id: String,
    first: (String, String),
    second: (String, String),
    /// An agent made by another member of the Room: (agent id, token).
    member: (String, String),
    _worktree: crate::test_support::TestWorktree,
}

/// A Room with two agents whose runs hold runtime tokens: (agent id, token).
fn room(label: &str, permission: Option<AgentPermissionLevel>) -> Room {
    let worktree = crate::test_support::TestWorktree::new(label);
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon boots");
    let session_id = app
        .sessions_mut()
        .create_session(worktree.session_request())
        .expect("session")
        .id()
        .to_string();
    let mut agents = Vec::new();
    for alias in ["first", "second", "member"] {
        let mut request = CreateAgentRequest::new(&session_id, "dev-stub").with_alias(alias);
        if alias == "member" {
            request = request.with_owner_user_id("room-member");
        }
        if let Some(level) = permission {
            request = request.with_permission_level_override(level);
        }
        let agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(request)
            .expect("agent");
        let run = launch_test_provider(
            &mut app,
            &session_id,
            agent.id(),
            "dev-stub",
            "dev-stub",
            "model",
        );
        let token = run.runtime_mcp_auth_token().expect("token").to_string();
        agents.push((agent.id().to_string(), token));
    }
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let member = agents.pop().unwrap();
    let second = agents.pop().unwrap();
    let first = agents.pop().unwrap();
    Room {
        router,
        session_id,
        first,
        second,
        member,
        _worktree: worktree,
    }
}

async fn call(
    room: &Room,
    token: &str,
    tool: &str,
    arguments: serde_json::Value,
) -> RuntimeToolResult {
    room.router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(token, tool, arguments)
        .await
        .unwrap_or_else(|error| panic!("{tool}: {error}"))
}

async fn open(room: &Room, token: &str) -> String {
    let opened = call(
        room,
        token,
        "mcp__chariox__terminal_open",
        serde_json::json!({}),
    )
    .await;
    assert!(opened.ok, "{:?}", opened.payload);
    opened.payload["terminal"]["terminal_id"]
        .as_str()
        .expect("terminal id")
        .to_string()
}

#[test]
fn an_agent_runs_a_command_and_waits_for_the_shell_to_report_it_finished() {
    run("terminal-tools-run", || async {
        let room = room("terminal_tools_run", None);
        let (agent, token) = (&room.first.0, &room.first.1);
        let specs = room
            .router
            .runtime_state
            .runtime_tool_specs_for_auth_token(token);
        for tool in [
            "chariox.terminal_open",
            "chariox.terminal_list",
            "chariox.terminal_send",
            "chariox.terminal_read",
            "chariox.terminal_wait",
            "chariox.terminal_close",
        ] {
            assert!(specs.iter().any(|spec| spec.name == tool), "{tool}");
        }

        let id = open(&room, token).await;
        let sent = call(
            &room,
            token,
            "chariox.terminal_send",
            serde_json::json!({
                "terminal_id": id,
                "text": "echo tool-$((6*7)); (exit 3)",
                "enter": true,
            }),
        )
        .await;
        assert!(sent.ok, "{:?}", sent.payload);
        let waited = call(
            &room,
            token,
            "terminal_wait",
            serde_json::json!({
                "terminal_id": id,
                "until": ["finished"],
                "after_offset": sent.payload["after_offset"],
                "timeout_seconds": 20,
            }),
        )
        .await;
        assert_eq!(
            waited.payload["still_running"], false,
            "{:?}",
            waited.payload
        );
        assert_eq!(waited.payload["fired"]["kind"], "finished");
        assert_eq!(waited.payload["fired"]["exit_code"], 3);
        assert!(waited.payload["screen_text"]
            .as_str()
            .unwrap()
            .contains("tool-42"));

        let read = call(
            &room,
            token,
            "chariox.terminal_read",
            serde_json::json!({ "terminal_id": id }),
        )
        .await;
        assert!(read.payload["screen_text"]
            .as_str()
            .unwrap()
            .contains("tool-42"));
        assert!(read.payload["note"].as_str().unwrap().contains("untrusted"));

        // The agent that opened an untouched tab may close it.
        let closed = call(
            &room,
            token,
            "chariox.terminal_close",
            serde_json::json!({ "terminal_id": id }),
        )
        .await;
        assert_eq!(closed.payload["closed"], true);
        let listed = call(&room, token, "chariox.terminal_list", serde_json::json!({})).await;
        assert!(listed.payload["terminals"].as_array().unwrap().is_empty());
        let _ = agent;
    });
}

#[test]
fn a_program_one_agent_started_is_busy_for_the_other() {
    run("terminal-tools-busy", || async {
        let room = room("terminal_tools_busy", None);
        let (first_id, first) = (&room.first.0, &room.first.1);
        let second = &room.second.1;
        let id = open(&room, first).await;
        let started = call(
            &room,
            first,
            "chariox.terminal_send",
            serde_json::json!({ "terminal_id": id, "text": "cat", "enter": true }),
        )
        .await;
        assert!(started.ok, "{:?}", started.payload);
        // The other agent is refused once the program owns the terminal.
        let mut refusal = None;
        for _ in 0..100 {
            let attempt = call(
                &room,
                second,
                "chariox.terminal_send",
                serde_json::json!({ "terminal_id": id, "text": "echo intruder", "enter": true }),
            )
            .await;
            if !attempt.ok {
                refusal = attempt.payload["refusal"].as_str().map(str::to_string);
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(refusal, Some(format!("busy (driven by {first_id})")));
        // It can still read.
        let read = call(
            &room,
            second,
            "chariox.terminal_read",
            serde_json::json!({ "terminal_id": id }),
        )
        .await;
        assert!(read.ok);
        let ended = call(
            &room,
            first,
            "chariox.terminal_send",
            serde_json::json!({ "terminal_id": id, "text": "\u{4}" }),
        )
        .await;
        assert!(ended.ok);
        call(
            &room,
            first,
            "chariox.terminal_close",
            serde_json::json!({ "terminal_id": id }),
        )
        .await;
    });
}

#[test]
fn a_required_agent_asks_per_input_and_a_denial_types_nothing() {
    run("terminal-tools-permission", || async {
        let room = room(
            "terminal_tools_permission",
            Some(AgentPermissionLevel::Required),
        );
        let (agent_id, token) = (&room.first.0, &room.first.1);
        // Opening a shell types nothing: no question.
        let id = open(&room, token).await;

        let router = room.router.clone();
        let session_id = room.session_id.clone();
        let answerer = tokio::spawn(async move {
            let mut answered = Vec::new();
            for choice in ["deny", "allow"] {
                let interaction_id = loop {
                    let session = router
                        .runtime_state
                        .session_snapshot(&session_id)
                        .await
                        .unwrap();
                    if let Some(interaction) = session
                        .active_interactions()
                        .iter()
                        .find(|interaction| !answered.contains(&interaction.id().to_string()))
                    {
                        break interaction.id().to_string();
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                };
                router
                    .runtime_state
                    .resolve_runtime_interaction(&session_id, &interaction_id, choice, None)
                    .await
                    .expect("interaction resolves");
                answered.push(interaction_id);
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        });

        let denied = call(
            &room,
            token,
            "chariox.terminal_send",
            serde_json::json!({ "terminal_id": id, "text": "touch denied-marker", "enter": true }),
        )
        .await;
        assert!(!denied.ok);
        assert_eq!(denied.payload["reason"], "permission_denied");
        let allowed = call(
            &room,
            token,
            "chariox.terminal_send",
            serde_json::json!({ "terminal_id": id, "text": "echo approved-input", "enter": true }),
        )
        .await;
        assert!(allowed.ok, "{:?}", allowed.payload);
        answerer.await.unwrap();

        let waited = call(
            &room,
            token,
            "chariox.terminal_wait",
            serde_json::json!({
                "terminal_id": id,
                "until": [{"text": "approved-input"}],
                "after_offset": allowed.payload["after_offset"],
                "timeout_seconds": 20,
            }),
        )
        .await;
        assert_eq!(waited.payload["fired"]["kind"], "output_matched");
        let screen = waited.payload["screen_text"].as_str().unwrap();
        assert!(!screen.contains("denied-marker"), "{screen}");
        let _ = agent_id;
        call(
            &room,
            token,
            "chariox.terminal_close",
            serde_json::json!({ "terminal_id": id }),
        )
        .await;
    });
}

#[test]
fn an_agent_made_by_another_member_cannot_reach_the_owners_terminals() {
    run("terminal-tools-member-agent", || async {
        let room = room("terminal_tools_member_agent", None);
        let owner_token = &room.first.1;
        let member_token = &room.member.1;
        let id = open(&room, owner_token).await;

        // The member's agent holds a legitimate runtime token of this Room,
        // yet every terminal tool is refused for it.
        for (tool, arguments) in [
            ("chariox.terminal_list", serde_json::json!({})),
            (
                "chariox.terminal_read",
                serde_json::json!({ "terminal_id": id }),
            ),
            (
                "chariox.terminal_send",
                serde_json::json!({ "terminal_id": id, "text": "echo intruder", "enter": true }),
            ),
            (
                "chariox.terminal_close",
                serde_json::json!({ "terminal_id": id }),
            ),
            ("chariox.terminal_open", serde_json::json!({})),
        ] {
            let error = room
                .router
                .runtime_state
                .dispatch_authenticated_runtime_tool_call(member_token, tool, arguments)
                .await
                .expect_err(tool)
                .to_string();
            assert!(
                error.contains("does not act for the Room owner"),
                "{tool}: {error}"
            );
        }

        // The owner's own agent still works, and the terminal is untouched.
        let listed = call(
            &room,
            owner_token,
            "chariox.terminal_list",
            serde_json::json!({}),
        )
        .await;
        assert!(listed.ok);
        assert_eq!(
            listed.payload["terminals"].as_array().map(Vec::len),
            Some(1)
        );
        let screen = call(
            &room,
            owner_token,
            "chariox.terminal_read",
            serde_json::json!({ "terminal_id": id }),
        )
        .await;
        assert!(!screen.payload["scrollback_tail"]
            .as_str()
            .unwrap()
            .contains("intruder"));
        call(
            &room,
            owner_token,
            "chariox.terminal_close",
            serde_json::json!({ "terminal_id": id }),
        )
        .await;
    });
}
