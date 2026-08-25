use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

mod support;
use support::runtime_integration::MockOpenCodeServer;

#[test]
fn mock_opencode_reports_the_runtime_mcp_as_connected() {
    let mock_server = MockOpenCodeServer::start(Duration::ZERO);
    let mut connect_stream = TcpStream::connect(("127.0.0.1", mock_server.port()))
        .expect("mock OpenCode server should accept MCP connect requests");
    connect_stream
        .write_all(
            b"POST /mcp/chariox/connect HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .expect("MCP connect request should write");
    let mut connect_response = String::new();
    connect_stream
        .read_to_string(&mut connect_response)
        .expect("MCP connect response should read");
    assert!(
        connect_response.starts_with("HTTP/1.1 200"),
        "{connect_response}"
    );

    let mut stream = TcpStream::connect(("127.0.0.1", mock_server.port()))
        .expect("mock OpenCode server should accept connections");
    stream
        .write_all(b"GET /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .expect("MCP status request should write");

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("MCP status response should read");

    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(
        response.contains(r#""chariox":{"status":"connected"}"#),
        "{response}"
    );
}
