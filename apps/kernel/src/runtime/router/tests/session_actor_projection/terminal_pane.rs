//! Claude Code's own terminal as an agent pane, through the kernel's MCP HTTP endpoint the
//! terminal uses: its status line, cards as elicitation dialogs, and settings it reports.
use super::*;
use crate::local::{GetPaneFeedRequest, PaneFeed, RespondToInteractionRequest};
use crate::provider::{
    ProviderClientInterface, RuntimeMcpBinding, CLAUDE_PANE_STATUS_FILE_ENV,
    CLAUDE_TERMINAL_MODEL_FILE_ENV, CLAUDE_TERMINAL_MODE_FILE_ENV,
};
use crate::session::NativeInteractionOrigin;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::Method;
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use std::time::Duration;

const TOKEN: &str = "terminal-pane-token";

struct Terminal {
    router: Arc<CommandRouter>,
    session_id: String,
    agent_id: String,
    run_id: String,
    address: std::net::SocketAddr,
    files: crate::test_temp_dir::TestTempDir,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn terminal() -> Terminal {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let files = crate::test_temp_dir::TestTempDir::new("terminal-pane");
    let path = files.path().display().to_string();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(&path, &path))
        .expect("session should be created");
    let (session_id, agent_id) = (session.id().to_string(), agent.id().to_string());
    let run = app
        .launch_provider(
            LaunchProviderRequest::new(&session_id, "dev-stub", "claude-code", "default", "sonnet")
                .with_agent_id(&agent_id)
                .with_client_interface(ProviderClientInterface::NativeTui)
                .with_runtime_mcp_binding(RuntimeMcpBinding::new("http://127.0.0.1/mcp", TOKEN)),
        )
        .expect("provider run should launch");
    let env = [
        (CLAUDE_PANE_STATUS_FILE_ENV, "pane-status.txt"),
        (CLAUDE_TERMINAL_MODEL_FILE_ENV, "terminal-model.json"),
        (CLAUDE_TERMINAL_MODE_FILE_ENV, "terminal-mode"),
    ]
    .into_iter()
    .map(|(name, file)| {
        (
            name.to_string(),
            files.path().join(file).display().to_string(),
        )
    })
    .collect();
    let run = run.with_pty_env(env);
    let run_id = run.id().to_string();
    app.update_provider_run_projection(run);
    let router = Arc::new(CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(app)),
        2,
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener binds");
    let address = listener.local_addr().expect("listener address");
    let server_router = router.clone();
    let server = tokio::spawn(async move {
        let _ =
            crate::transport::mcp_server::run_mcp_http_server_on_listener(server_router, listener)
                .await;
    });
    Terminal {
        router,
        session_id,
        agent_id,
        run_id,
        address,
        files,
        server,
    }
}

fn question(id: &str, agent_id: &str) -> RuntimeInteraction {
    RuntimeInteraction::new(
        id,
        agent_id,
        RuntimeInteractionKind::Choice,
        RuntimeInteractionLevel::Info,
        None,
        format!("Question {id}"),
        vec![
            RuntimeInteractionChoice::new("allow", "Allow", "allow", None),
            RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
        ],
        None,
        None,
        None,
    )
}

impl Terminal {
    async fn http(&self, method: Method, body: String) -> hyper::Response<Incoming> {
        let io = TokioIo::new(
            tokio::net::TcpStream::connect(self.address)
                .await
                .expect("the MCP endpoint answers"),
        );
        let (mut sender, connection) = hyper::client::conn::http1::handshake(io)
            .await
            .expect("http handshake");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        sender
            .send_request(
                hyper::Request::builder()
                    .method(method)
                    .uri("/mcp")
                    .header("Host", "127.0.0.1")
                    .header("Authorization", format!("Bearer {TOKEN}"))
                    .header("Accept", "application/json, text/event-stream")
                    .header("Content-Type", "application/json")
                    .body(Full::new(Bytes::from(body)))
                    .expect("request builds"),
            )
            .await
            .expect("request is sent")
    }

    /// Claude's notification stream; the first frame is the catalog's opening one.
    async fn stream(&self) -> Incoming {
        let response = self.http(Method::GET, String::new()).await;
        assert_eq!(response.status(), hyper::StatusCode::OK);
        response.into_body()
    }

    async fn raise(
        &self,
        interaction: RuntimeInteraction,
    ) -> tokio::sync::oneshot::Receiver<crate::runtime::state::PendingInteractionResolution> {
        self.router
            .runtime_state
            .create_runtime_interaction(&self.session_id, interaction)
            .await
            .expect("the card is raised")
    }

    async fn feed(&self) -> PaneFeed {
        let request = LocalDaemonRequest::GetPaneFeed(GetPaneFeedRequest {
            session_id: self.session_id.clone(),
            agent_id: self.agent_id.clone(),
        });
        let command = KernelCommand::from_local_request("pane", None, None, &request);
        match self.router.dispatch(command, request).await {
            Ok(LocalDaemonResponse::PaneFeed { feed }) => feed,
            other => panic!("expected the pane feed, got {other:?}"),
        }
    }

    fn file(&self, name: &str) -> std::path::PathBuf {
        self.files.path().join(name)
    }

    async fn status_line_shows(&self, text: &str) -> String {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            let shown = std::fs::read_to_string(self.file("pane-status.txt")).unwrap_or_default();
            if shown.contains(text) {
                return shown;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the status line never showed {text:?}: {shown:?}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// The next JSON-RPC message on the stream that has `method`, within `within`.
async fn next_message(stream: &mut Incoming, method: &str, within: Duration) -> Option<Value> {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let frame = tokio::time::timeout_at(deadline, stream.frame())
            .await
            .ok()??
            .ok()?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        for line in String::from_utf8_lossy(&data).lines() {
            let Some(message) = line.strip_prefix("data: ") else {
                continue;
            };
            let message: Value = serde_json::from_str(message).expect("frames are JSON-RPC");
            if message["method"] == method {
                return Some(message);
            }
        }
    }
}

#[tokio::test]
async fn a_card_opens_as_claudes_own_dialog_and_one_answer_there_closes_it_everywhere() {
    let terminal = terminal().await;
    let mut stream = terminal.stream().await;
    let resolved = terminal.raise(question("ask", &terminal.agent_id)).await;

    let dialog = next_message(&mut stream, "elicitation/create", Duration::from_secs(3))
        .await
        .expect("the card opens as an elicitation dialog in the terminal");
    assert_eq!(dialog["params"]["mode"], "form");
    assert_eq!(dialog["params"]["message"], "Question ask");
    assert_eq!(
        dialog["params"]["requestedSchema"]["properties"]["choice"]["oneOf"],
        json!([{ "const": "allow", "title": "Allow" }, { "const": "deny", "title": "Deny" }])
    );
    let shown = terminal.status_line_shows("1 card waiting").await;
    assert!(
        shown.starts_with("Room") || shown.contains(" · "),
        "{shown}"
    );

    let answer = json!({
        "jsonrpc": "2.0",
        "id": dialog["id"],
        "result": { "action": "accept", "content": { "choice": "deny" } }
    });
    let response = terminal.http(Method::POST, answer.to_string()).await;
    assert_eq!(response.status(), hyper::StatusCode::ACCEPTED);
    let resolution = tokio::time::timeout(Duration::from_secs(2), resolved)
        .await
        .expect("the terminal's answer settles the card")
        .expect("the card resolves");
    assert_eq!(resolution.choice_id.as_deref(), Some("deny"));
    assert!(
        terminal.feed().await.cards.is_empty(),
        "the card left every pane"
    );
}

#[tokio::test]
async fn a_card_answered_in_another_client_closes_the_terminal_dialog_within_two_seconds() {
    let terminal = terminal().await;
    let mut stream = terminal.stream().await;
    let _resolved = terminal.raise(question("ask", &terminal.agent_id)).await;
    let dialog = next_message(&mut stream, "elicitation/create", Duration::from_secs(3))
        .await
        .expect("the card opens in the terminal");

    let request = LocalDaemonRequest::RespondToInteraction(RespondToInteractionRequest {
        session_id: terminal.session_id.clone(),
        interaction_id: "ask".into(),
        choice_id: "allow".into(),
        custom_reply: None,
        passkey: None,
        passkey_remember_minutes: None,
    });
    let command = KernelCommand::from_local_request("web", None, None, &request);
    terminal
        .router
        .dispatch(command, request)
        .await
        .expect("the web pane answers");

    let cancelled = next_message(
        &mut stream,
        "notifications/cancelled",
        Duration::from_secs(2),
    )
    .await
    .expect("the terminal's dialog closes");
    assert_eq!(cancelled["params"]["requestId"], dialog["id"]);
}

#[tokio::test]
async fn a_dismissed_dialog_leaves_the_card_open_and_does_not_reopen() {
    let terminal = terminal().await;
    let mut stream = terminal.stream().await;
    let _resolved = terminal.raise(question("ask", &terminal.agent_id)).await;
    let dialog = next_message(&mut stream, "elicitation/create", Duration::from_secs(3))
        .await
        .expect("the card opens in the terminal");
    let dismiss = json!({ "jsonrpc": "2.0", "id": dialog["id"], "result": { "action": "cancel" } });
    terminal.http(Method::POST, dismiss.to_string()).await;

    assert!(
        next_message(
            &mut stream,
            "elicitation/create",
            Duration::from_millis(2500)
        )
        .await
        .is_none(),
        "a dismissed card does not open again over the terminal"
    );
    assert_eq!(
        terminal.feed().await.cards.len(),
        1,
        "not now: the card stays open"
    );
    terminal.status_line_shows("1 card waiting").await;
    drop(stream);
    let mut reconnected = terminal.stream().await;
    assert!(
        next_message(
            &mut reconnected,
            "elicitation/create",
            Duration::from_millis(2500)
        )
        .await
        .is_none(),
        "the same run keeps its dismissal after the notification stream reconnects"
    );
    assert_eq!(
        terminal.feed().await.cards.len(),
        1,
        "reconnecting never answers the card"
    );
}

#[tokio::test]
async fn claudes_own_permission_card_opens_no_second_dialog() {
    let terminal = terminal().await;
    let mut stream = terminal.stream().await;
    // The agent's own question raised in a native turn carries the run's origin; Claude's
    // permission prompt is the card with Claude's own dialog.
    let own = RuntimeInteraction::new(
        crate::provider::claude_native_permission_interaction_id(&terminal.run_id, "request-1"),
        &terminal.agent_id,
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Warning,
        None,
        "Run ls?".to_string(),
        vec![RuntimeInteractionChoice::new(
            "allow_once",
            "Allow once",
            "allow",
            None,
        )],
        None,
        None,
        None,
    )
    .with_native_origin(Some(NativeInteractionOrigin::ProviderStartup {
        provider_run_id: terminal.run_id.clone(),
    }));
    let _own = terminal.raise(own).await;
    terminal.status_line_shows("1 card waiting").await;
    assert!(
        next_message(
            &mut stream,
            "elicitation/create",
            Duration::from_millis(2500)
        )
        .await
        .is_none(),
        "Claude's own dialog is the terminal face of its permission card"
    );
}

#[tokio::test]
async fn a_model_and_mode_switched_in_the_terminal_show_in_every_client() {
    let terminal = terminal().await;
    let write = |name: &str, contents: &str| {
        std::fs::write(terminal.file(name), contents).expect("the terminal writes its settings")
    };
    // Claude's first reading is the launch choice under its full name, not a switch.
    write(
        "terminal-model.json",
        r#"{"model":"claude-sonnet-5-5","effort":null}"#,
    );
    let _stream = terminal.stream().await;
    terminal.status_line_shows("· sonnet ·").await;
    write(
        "terminal-model.json",
        r#"{"model":"claude-haiku-5-5","effort":"low"}"#,
    );
    write("terminal-mode", "plan");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let settings = terminal.feed().await.settings;
        if settings.model.as_deref() == Some("claude-haiku-5-5")
            && settings.effort.as_deref() == Some("low")
            && settings.execution_mode == crate::provider::AgentExecutionMode::Plan
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the switch never reached the kernel: {settings:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    terminal
        .status_line_shows("claude-haiku-5-5 · low · plan")
        .await;
}

#[tokio::test]
async fn clearing_effort_in_the_terminal_clears_the_kernel_setting() {
    let terminal = terminal().await;
    std::fs::write(
        terminal.file("terminal-model.json"),
        r#"{"model":"claude-sonnet-5-5","effort":null}"#,
    )
    .unwrap();
    let _stream = terminal.stream().await;
    terminal.status_line_shows("· sonnet ·").await;
    std::fs::write(
        terminal.file("terminal-model.json"),
        r#"{"model":"claude-sonnet-5-5","effort":"high"}"#,
    )
    .unwrap();
    terminal.status_line_shows("· high ·").await;
    std::fs::write(
        terminal.file("terminal-model.json"),
        r#"{"model":"claude-sonnet-5-5","effort":null}"#,
    )
    .unwrap();
    terminal.status_line_shows("· default ·").await;
    assert_eq!(terminal.feed().await.settings.effort, None);
}
