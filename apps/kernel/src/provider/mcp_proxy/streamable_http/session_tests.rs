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
        {
            let mut seen = server.join().unwrap();
            seen.sort();
            seen
        },
        {
            let mut expected = vec![
                "initialize:one",
                "initialize:two",
                "tools/list:one",
                "tools/list:two",
                "delete:synthetic-one",
                "tools/list:two",
                "delete:synthetic-two",
            ];
            expected.sort();
            expected
        }
    );
}

// MP-08 / MP-10 / MP-11: cancellation must overtake a slow POST for both transport modes.
#[test]
fn review0253_http_session_allows_concurrent_cancellation() {
    for stateful in [false, true] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let backing = CharioxMcpServerConfig::streamable_http(
            "review0253",
            format!("http://{}/mcp", listener.local_addr().unwrap()),
        );
        let (slow_seen_tx, slow_seen_rx) = std::sync::mpsc::channel();
        let (gate_tx, gate_rx) = std::sync::mpsc::channel();
        let gate = Arc::new(Mutex::new(gate_rx));
        let server = std::thread::spawn(move || {
            let mut workers = vec![];
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                let gate = gate.clone();
                let slow_seen = slow_seen_tx.clone();
                workers.push(std::thread::spawn(move||{
                    stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                    let mut reader=BufReader::new(&mut stream);let mut headers=String::new();
                    loop{let mut line=String::new();reader.read_line(&mut line).unwrap();headers.push_str(&line);if line=="\r\n"{break}}
                    let length=headers.lines().find_map(|l|l.to_ascii_lowercase().strip_prefix("content-length:").map(|n|n.trim().parse::<usize>().unwrap())).unwrap();
                    let mut body=vec![0;length];reader.read_exact(&mut body).unwrap();drop(reader);
                    let request:serde_json::Value=serde_json::from_slice(&body).unwrap();
                    let method=request["method"].as_str().unwrap();
                    if method!="initialize" && stateful {assert!(headers.to_ascii_lowercase().contains("mcp-session-id: synthetic-review0253"));}
                    if method=="tools/call" {slow_seen.send(()).unwrap();let _=gate.lock().unwrap().recv_timeout(Duration::from_secs(3));}
                    if method=="notifications/cancelled" {stream.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();return}
                    let session=if method=="initialize" && stateful {"Mcp-Session-Id: synthetic-review0253\r\n"}else{""};
                    let body=serde_json::json!({"jsonrpc":"2.0","id":request["id"],"result":{"protocolVersion":"2025-03-26"}}).to_string();
                    write!(stream,"HTTP/1.1 200 OK\r\n{session}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                }));
            }
            for worker in workers {
                worker.join().unwrap()
            }
        });
        let run = if stateful {
            "review0253-stateful"
        } else {
            "review0253-stateless"
        };
        dispatch(
            run,
            run,
            &backing,
            serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
        )
        .unwrap();
        let slow_backing = backing.clone();
        let slow = std::thread::spawn(move || {
            dispatch(
                run,
                run,
                &slow_backing,
                serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call"}),
            )
        });
        slow_seen_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let (cancel_tx, cancel_rx) = std::sync::mpsc::channel();
        let cancel = std::thread::spawn(move || {
            let result = dispatch(
                run,
                run,
                &backing,
                serde_json::json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":2}}),
            );
            cancel_tx.send(result.is_ok()).unwrap();
            result
        });
        let overtook = cancel_rx
            .recv_timeout(Duration::from_millis(300))
            .unwrap_or(false);
        gate_tx.send(()).unwrap();
        slow.join().unwrap().unwrap();
        cancel.join().unwrap().unwrap();
        server.join().unwrap();
        release_run(run);
        assert!(
            overtook,
            "cancellation was serialized behind the in-flight POST (stateful={stateful})"
        );
    }
}

// MP-08 / MP-10 / MP-11: retiring an owner must not await an unreachable upstream DELETE.
#[test]
fn review0253_http_release_does_not_wait_for_remote_delete() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let backing = CharioxMcpServerConfig::streamable_http(
        "review0253-release",
        format!("http://{}/mcp", listener.local_addr().unwrap()),
    );
    let server = std::thread::spawn(move || {
        let (mut first, _) = listener.accept().unwrap();
        first
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut reader = BufReader::new(&mut first);
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if let Some(n) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = n.trim().parse::<usize>().unwrap()
            }
            if line == "\r\n" {
                break;
            }
        }
        let mut bytes = vec![0; length];
        reader.read_exact(&mut bytes).unwrap();
        drop(reader);
        let body = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-03-26"}}"#;
        write!(first,"HTTP/1.1 200 OK\r\nMcp-Session-Id: synthetic-release\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        drop(first);
        let (_delete, _) = listener.accept().unwrap();
        std::thread::sleep(Duration::from_millis(1200));
    });
    dispatch(
        "review0253-release",
        "review0253-release",
        &backing,
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
    )
    .unwrap();
    let start = std::time::Instant::now();
    release_run("review0253-release");
    let elapsed = start.elapsed();
    server.join().unwrap();
    assert!(
        elapsed < Duration::from_millis(100),
        "remote lease cleanup blocked its caller for {elapsed:?}"
    );
}
