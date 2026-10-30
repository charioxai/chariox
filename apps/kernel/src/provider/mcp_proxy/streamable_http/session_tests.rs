//! MP-08 / MP-10 / MP-11: owner isolation, header authority and lease release.
use super::*;
use std::io::{Read, Write};
#[test]
fn review1301_http_sessions_keep_owner_headers_and_release_only_owned_leases() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut backing = CharioxMcpServerConfig::streamable_http(
        "review1301-session",
        format!("http://{}/mcp", listener.local_addr().unwrap()),
    );
    if let CharioxMcpTransportConfig::StreamableHttp { http_headers, .. } = &mut backing.transport {
        http_headers.insert("Mcp-Session-Id".into(), "spoofed".into());
    }
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        let mut seen = Vec::new();
        while seen.len() < 7 && std::time::Instant::now() < deadline {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut headers = Vec::new();
            let mut byte = [0];
            while !headers.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                headers.push(byte[0]);
            }
            let headers = String::from_utf8(headers).unwrap();
            let values: BTreeMap<_, _> = headers
                .lines()
                .filter_map(|line| {
                    line.split_once(':')
                        .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_owned()))
                })
                .collect();
            let length = values
                .get("content-length")
                .map(|v| v.parse::<usize>().unwrap())
                .unwrap_or(0);
            let mut body = vec![0; length];
            stream.read_exact(&mut body).unwrap();
            if headers.starts_with("DELETE ") {
                seen.push(format!("delete:{}", values.get("mcp-session-id").unwrap()));
                stream
                    .write_all(
                        b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .unwrap();
                continue;
            }
            let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let method = payload["method"].as_str().unwrap();
            let owner = payload["params"]["owner"].as_str().unwrap();
            let session = format!("synthetic-{owner}");
            let assigned = if method == "initialize" {
                assert!(!values.contains_key("mcp-session-id"));
                format!("Mcp-Session-Id: {session}\r\n")
            } else {
                assert_eq!(values.get("mcp-session-id"), Some(&session));
                assert_eq!(
                    values.get("mcp-protocol-version").map(String::as_str),
                    Some("2025-03-26")
                );
                String::new()
            };
            seen.push(format!("{method}:{owner}"));
            let body=serde_json::json!({"jsonrpc":"2.0","id":payload["id"],"result":{"protocolVersion":"2025-03-26","tools":[]}}).to_string();
            write!(stream,"HTTP/1.1 200 OK\r\n{assigned}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        }
        seen
    });
    for (run, owner, method) in [
        ("review1301-run-one", "one", "initialize"),
        ("review1301-run-two", "two", "initialize"),
        ("review1301-run-one", "one", "tools/list"),
        ("review1301-run-two", "two", "tools/list"),
    ] {
        dispatch(
            run,
            &format!("review1301-owner-{owner}"),
            &backing,
            serde_json::json!({"jsonrpc":"2.0","id":1,"method":method,"params":{"owner":owner}}),
        )
        .unwrap();
    }
    release_run("review1301-run-one");
    assert!(!sessions()
        .lock()
        .unwrap()
        .keys()
        .any(|key| key.0 == "review1301-run-one"));
    assert!(sessions()
        .lock()
        .unwrap()
        .keys()
        .any(|key| key.0 == "review1301-run-two"));
    dispatch(
        "review1301-run-two",
        "review1301-owner-two",
        &backing,
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{"owner":"two"}}),
    )
    .unwrap();
    release_session("review1301-owner-two");
    assert!(!sessions()
        .lock()
        .unwrap()
        .keys()
        .any(|key| key.1 == "review1301-owner-two"));
    assert_eq!(
        server.join().unwrap(),
        vec![
            "initialize:one",
            "initialize:two",
            "tools/list:one",
            "tools/list:two",
            "delete:synthetic-one",
            "tools/list:two",
            "delete:synthetic-two"
        ]
    );
}
