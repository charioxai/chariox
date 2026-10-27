use super::{dispatch_provider_mcp_proxy_request, shutdown_provider_mcp_proxy_session};
use crate::mcp::CharioxMcpServerConfig;
use serde_json::json;
use std::time::Duration;

#[test]
#[ignore = "requires Bubblewrap, Python3 and Linux user namespaces; run explicitly on managed host"]
fn sandboxed_stdio_mcp_survives_launching_thread_exit_without_restart() {
    const RUN: &str = "sandbox-lifetime-mcp-run";
    const SESSION: &str = "sandbox-lifetime-mcp-session";
    let mut config = CharioxMcpServerConfig::stdio(
        "sandbox-lifetime-mcp",
        "/usr/bin/bwrap",
        ["--die-with-parent", "--new-session", "--unshare-user", "--unshare-pid",
            "--uid", "0", "--gid", "0", "--ro-bind", "/", "/", "--",
            "/usr/bin/python3", "-u", "-c",
            "import json,sys\ncount=0\nfor line in sys.stdin:\n message=json.loads(line)\n count+=1\n print(json.dumps({'jsonrpc':'2.0','id':message['id'],'result':{'count':count}}),flush=True)"]
            .into_iter().map(str::to_string).collect(),
    );
    config.tool_timeout_sec = Some(5);
    let first_config = config.clone();
    let first = std::thread::spawn(move || {
        dispatch_provider_mcp_proxy_request(
            RUN,
            SESSION,
            &first_config,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"counter"}}),
        )
    })
    .join()
    .expect("initial caller should return");
    // A second request must reach the existing process, not silently recreate it.
    std::thread::sleep(Duration::from_secs(1));
    let second = dispatch_provider_mcp_proxy_request(
        RUN,
        SESSION,
        &config,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"counter"}}),
    );
    shutdown_provider_mcp_proxy_session(SESSION);
    assert_eq!(first.unwrap().pointer("/result/count"), Some(&json!(1)));
    assert_eq!(
        second.unwrap().pointer("/result/count"),
        Some(&json!(2)),
        "MCP process must retain its state after the launching caller exits"
    );
}
