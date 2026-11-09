//! The reach passthrough is retired (docs/design/sub-agents.md 7.2): an owner's Room agent
//! with a live run finds no second tool level next to `chariox`.
use super::*;

const OWNER_TOKEN: &str = "retired-reach-owner-token";

#[tokio::test]
async fn the_owners_room_agent_finds_no_reach_server() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let worktree = crate::test_support::TestWorktree::new("retired-reach-worktree");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    super::permission_prompt::insert_claude_run(
        &mut app,
        session.id(),
        agent.id(),
        "retired-reach-run",
        OWNER_TOKEN,
        crate::provider::ProviderClientInterface::Chariox,
    );
    let router = Arc::new(CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(app)),
        8,
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(super::super::run_mcp_http_server_on_listener(
        router, listener,
    ));
    let body = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}).to_string();
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("the runtime MCP server accepts connections");
    stream
        .write_all(
            format!(
                "POST /mcp/reach HTTP/1.1\r\nHost: kernel\r\nAuthorization: Bearer {OWNER_TOKEN}\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .expect("request is sent");
    let mut raw = String::new();
    stream
        .read_to_string(&mut raw)
        .await
        .expect("response is read");
    assert!(raw.starts_with("HTTP/1.1 404"), "{raw}");
    server.abort();
}
