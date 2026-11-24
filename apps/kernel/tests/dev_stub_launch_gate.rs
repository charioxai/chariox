//! Exercise the opt-in with the real kernel binary and its client protocol.
#![cfg(unix)]

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

#[path = "support/real_kernel.rs"]
mod real_kernel;
use real_kernel::{connect, create_session, request, Socket};

const OPT_IN: &str = "CHARIOX_PROVIDER_DEV_STUB";

fn scratch(cell: &str) -> PathBuf {
    std::env::temp_dir().join(format!("stub-{cell}-{}", std::process::id()))
}

async fn launch(socket: &mut Socket, session_id: &str) -> Result<Value, Value> {
    request(
        socket,
        json!({"LaunchProviderRun": {
            "session_id": session_id,
            "adapter_key": "dev-stub",
            "provider": "dev-stub",
            "account_profile": "default",
            "model": "terminal-echo-a"
        }}),
    )
    .await
}

async fn processes(socket: &mut Socket) -> Vec<Value> {
    let listed = request(
        socket,
        json!({"ListProviderProcesses": {"provider": "dev-stub"}}),
    )
    .await
    .unwrap();
    listed["ProviderProcessesListed"]["processes"]
        .as_array()
        .unwrap()
        .clone()
}

#[tokio::test]
async fn refuses_dev_stub_without_exact_opt_in() {
    for (cell, env) in [("unset", vec![]), ("true", vec![(OPT_IN, "true".into())])] {
        let scratch = scratch(cell);
        let kernel = real_kernel::start(&scratch, &env, None);
        let mut socket = connect(&kernel).await;
        let session = create_session(&mut socket, &scratch).await;
        let error = launch(&mut socket, &session).await.unwrap_err();
        assert_eq!(error["code"], "provider_dev_stub_disabled");
        assert_eq!(error["retryable"], false);
        assert!(error["message"]
            .as_str()
            .unwrap()
            .contains("CHARIOX_PROVIDER_DEV_STUB=1"));
        assert!(processes(&mut socket).await.is_empty());
    }
}

#[tokio::test]
async fn starts_dev_stub_pty_on_an_opted_in_private_kernel() {
    let scratch = scratch("private");
    let kernel = real_kernel::start(&scratch, &[(OPT_IN, "1".into())], None);
    let mut socket = connect(&kernel).await;
    let session = create_session(&mut socket, &scratch).await;
    let launched = launch(&mut socket, &session).await.unwrap();
    assert!(launched["ProviderRunLaunchAccepted"]["provider_run"]["id"].is_string());

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if processes(&mut socket)
            .await
            .iter()
            .any(|process| process["pid"].as_u64().is_some())
        {
            break;
        }
        assert!(Instant::now() < deadline, "dev-stub process did not start");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[test]
fn rejects_dev_stub_opt_in_at_hosted_or_managed_startup() {
    let cloud_profile = json!({
        "api_url": "https://cloud.example.test",
        "relay_url": "wss://relay.example.test",
        "email": "test@example.test",
        "account_id": "account",
        "user_id": "user",
        "account_slug": "account",
        "realm_id": "realm",
        "issuer_id": "issuer"
    });
    let cases: [(&str, Vec<(&str, OsString)>); 3] = [
        (
            "managed",
            vec![("CHARIOX_MANAGED_PROVIDER_ISOLATION", "1".into())],
        ),
        (
            "relay",
            vec![
                ("CHARIOX_RELAY_URL", "wss://relay.example.test".into()),
                ("CHARIOX_RELAY_TOKEN", "test-fixture".into()),
            ],
        ),
        (
            "cloud",
            vec![(
                "CHARIOX_CLOUD_RELAY_CONFIG_JSON",
                cloud_profile.to_string().into(),
            )],
        ),
    ];
    for (cell, mut env) in cases {
        env.push((OPT_IN, "1".into()));
        let scratch = scratch(cell);
        let mut kernel = real_kernel::start(&scratch, &env, None);
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = kernel.try_wait() {
                break status;
            }
            assert!(Instant::now() < deadline, "{cell} kernel did not exit");
            std::thread::sleep(Duration::from_millis(50));
        };
        assert!(!status.success(), "{cell} kernel exited successfully");
        let log = std::fs::read_to_string(scratch.join("kernel.log")).unwrap();
        assert!(
            log.contains("Unset CHARIOX_PROVIDER_DEV_STUB")
                && log.contains("hosted or managed kernel"),
            "{cell} startup log did not explain how to fix the configuration"
        );
    }
}
