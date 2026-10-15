//! MP-08 / MP-10 / MP-11: review regressions supplement hosted user-flow drills.
use super::*;
fn context(root: &std::path::Path) -> EnvironmentCheckContext {
    EnvironmentCheckContext {
        config: crate::config::DaemonConfig::for_tests(),
        owner: "owner".into(),
        profiles: crate::account_profile::ProviderAccountProfileRegistry::open(
            root.join("profiles"),
        )
        .unwrap(),
        apps: crate::durable_state::DurableKernelStateStore::open(root.join("apps.db")).unwrap(),
    }
}
fn operation(
    env: &ProjectEnvironment,
    target: &EnvironmentTargetBinding,
    selected: Vec<&str>,
) -> EnvironmentOperation {
    EnvironmentOperation {
        operation_id: "review0212-check".into(),
        attempt: 1,
        local_project_id: env.local_project_id.clone(),
        revision_digest: env.content_digest.clone(),
        target: target.clone(),
        kind: EnvironmentOperationKind::Check,
        phase: EnvironmentOperationPhase::Requested,
        selected_items: selected.into_iter().map(str::to_owned).collect(),
        per_item_opt_ins: vec![],
        per_item_results: vec![],
        created_at_ms: crate::session::unix_epoch_ms(),
        updated_at_ms: crate::session::unix_epoch_ms(),
        cancellation: None,
        receipts: vec![],
        recovery_state: EnvironmentRecoveryState::Pending,
    }
}
fn project(root: &std::path::Path, id: &str) -> crate::session::RuntimeProject {
    crate::session::RuntimeProject::new(
        id,
        "owner",
        root.to_str().unwrap(),
        "Review",
        crate::session::RuntimeProjectKind::Named,
    )
}

fn check_requirement(
    root: &std::path::Path,
    ctx: EnvironmentCheckContext,
    requirement: Requirement,
    budget: std::time::Duration,
) -> MachineObservation {
    let store = ProjectEnvironmentStore::new(root);
    let project = project(root, "review1118");
    let mut env = store.snapshot(&project).unwrap();
    let mut draft = environment_draft(&env);
    draft.project_requirements = vec![requirement];
    store
        .save_revision_locked(
            &env,
            &crate::local::SaveProjectEnvironmentRevisionRequest {
                project_id: project.id().into(),
                expected_revision: env.revision,
                expected_content_digest: env.content_digest.clone(),
                draft,
                accepted_proposal_ids: vec![],
                excluded_proposal_ids: vec![],
            },
            "owner",
        )
        .unwrap();
    env = store.snapshot(&project).unwrap();
    let target = EnvironmentTargetBinding {
        machine_id: "machine".into(),
        target_instance_generation: "boot".into(),
        slice_ref: None,
    };
    let op = operation(&env, &target, vec![]);
    let mut checks = store.load_checks(project.id()).unwrap();
    checks.operations.push(op.clone());
    store.save_checks(project.id(), &checks).unwrap();
    store
        .run_check_until(
            ctx,
            env,
            op,
            || Ok(project.clone()),
            std::time::Instant::now() + budget,
        )
        .unwrap();
    let checks = store.load_checks(project.id()).unwrap();
    assert_eq!(
        checks.operations.last().unwrap().phase,
        EnvironmentOperationPhase::Ready
    );
    checks.observations[0].clone()
}
fn mcp_requirement(url: &str) -> Requirement {
    let config:crate::mcp::CharioxMcpServerConfig=serde_json::from_value(serde_json::json!({"name":"review1118","transport":{"type":"streamable_http","url":url},"enabled":true,"required":false})).unwrap();
    crate::mcp::CharioxMcpRegistry::new(
        crate::runtime::capability_registry::mcp_registry_roots(None).unwrap(),
    )
    .install(&config)
    .unwrap();
    let mut r = requirement("mcp");
    r.spec=serde_json::from_value(serde_json::json!({"kind":"agent_tools","tool_kind":"mcp","registry_ref":"review1118","package_digest":null,"scope":{"kind":"project"},"runtime_requirements":[],"vault_refs":[],"compatible_providers":[]})).unwrap();
    r
}
#[test]
fn review1118_mcp_notification_202_reaches_tools_list_in_check() {
    use std::io::{Read, Write};
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let root = crate::test_support::TestWorktree::new("review1118-mcp");
    std::env::set_var("CHARIOX_HOME", root.path());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let server_finished = finished.clone();
    let server = std::thread::spawn(move || {
        let mut methods = vec![];
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while methods.len() < 3
            && !server_finished.load(std::sync::atomic::Ordering::SeqCst)
            && std::time::Instant::now() < deadline
        {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(std::time::Duration::from_millis(5));
                continue;
            };
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                .unwrap();
            let mut bytes = vec![];
            let mut byte = [0];
            while !bytes.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                bytes.push(byte[0]);
            }
            let headers = String::from_utf8(bytes).unwrap();
            let length: usize = headers
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse().unwrap())
                })
                .unwrap();
            let mut body = vec![0; length];
            stream.read_exact(&mut body).unwrap();
            let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let method = payload["method"].as_str().unwrap();
            methods.push(method.to_string());
            let response = match method {
                "initialize" => Some(
                    serde_json::json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"test","version":"1"}}}),
                ),
                "tools/list" => {
                    Some(serde_json::json!({"jsonrpc":"2.0","id":2,"result":{"tools":[]}}))
                }
                _ => None,
            };
            if let Some(body) = response {
                let body = body.to_string();
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            } else {
                stream
                    .write_all(
                        b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .unwrap();
            }
        }
        methods
    });
    let ctx = context(root.path());
    let r = mcp_requirement(&url);
    let observation = check_requirement(root.path(), ctx, r, std::time::Duration::from_secs(10));
    finished.store(true, std::sync::atomic::Ordering::SeqCst);
    let methods = server.join().unwrap();
    assert_eq!(
        methods,
        vec!["initialize", "notifications/initialized", "tools/list"]
    );
    assert_eq!(observation.status, EnvironmentObservationStatus::Ready);
}

#[test]
fn review1301_mcp_notification_202_reaches_tools_list_in_check() {
    use std::io::{Read, Write};
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let root = crate::test_support::TestWorktree::new("review1118-mcp");
    std::env::set_var("CHARIOX_HOME", root.path());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let server_finished = finished.clone();
    let server = std::thread::spawn(move || {
        let mut methods = vec![];
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while methods.len() < 3
            && !server_finished.load(std::sync::atomic::Ordering::SeqCst)
            && std::time::Instant::now() < deadline
        {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(std::time::Duration::from_millis(5));
                continue;
            };
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                .unwrap();
            let mut bytes = vec![];
            let mut byte = [0];
            while !bytes.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                bytes.push(byte[0]);
            }
            let headers = String::from_utf8(bytes).unwrap();
            let length: usize = headers
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse().unwrap())
                })
                .unwrap();
            let mut body = vec![0; length];
            stream.read_exact(&mut body).unwrap();
            let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let method = payload["method"].as_str().unwrap();
            methods.push(method.to_string());
            let session_present = headers
                .lines()
                .any(|line| line.eq_ignore_ascii_case("Mcp-Session-Id: synthetic-session"));
            if method != "initialize" && !session_present {
                stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                break;
            }
            let response = match method {
                "initialize" => Some(
                    serde_json::json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"test","version":"1"}}}),
                ),
                "tools/list" => {
                    Some(serde_json::json!({"jsonrpc":"2.0","id":2,"result":{"tools":[]}}))
                }
                _ => None,
            };
            if let Some(body) = response {
                let body = body.to_string();
                write!(stream,"HTTP/1.1 200 OK\r\nMcp-Session-Id: synthetic-session\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            } else {
                stream
                    .write_all(
                        b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .unwrap();
            }
        }
        methods
    });
    let ctx = context(root.path());
    let r = mcp_requirement(&url);
    let observation = check_requirement(root.path(), ctx, r, std::time::Duration::from_secs(10));
    finished.store(true, std::sync::atomic::Ordering::SeqCst);
    let methods = server.join().unwrap();
    assert_eq!(
        methods,
        vec!["initialize", "notifications/initialized", "tools/list"]
    );
    assert_eq!(observation.status, EnvironmentObservationStatus::Ready);
}

#[path = "review1118_account_tests.rs"]
mod account;
#[path = "review1118_mcp_dns_tests.rs"]
mod dns;
