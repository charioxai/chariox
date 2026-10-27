//! MP-08 / MP-10 / MP-11: native utility lifetime regression; synthetic RPCs only.
use super::*;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

const CHILD: &str =
    "local::provider_requests::catalog::codex_refresh_tests::account_endpoint_child";

#[test]
#[ignore = "explicitly executed by automatic_refresh_keeps_pending_login_server"]
fn account_endpoint_child() {
    let endpoint = std::env::var("CTXFIX_ACCOUNT_LISTEN").unwrap();
    let listener = std::net::TcpListener::bind(endpoint.trim_start_matches("ws://")).unwrap();
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(std::env::var("CTXFIX_ACCOUNT_LOG").unwrap())
        .unwrap();
    writeln!(log, "{}", std::process::id()).unwrap();
    for stream in listener.incoming() {
        let mut stream = stream.unwrap();
        std::thread::spawn(move || {
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut header = [0; 64];
            if stream
                .peek(&mut header)
                .is_ok_and(|n| header[..n].starts_with(b"GET /readyz "))
            {
                let _ = stream.read(&mut header);
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
                return;
            }
            let Ok(mut socket) = tokio_tungstenite::tungstenite::accept(stream) else {
                return;
            };
            while let Ok(message) = socket.read() {
                let Ok(text) = message.to_text() else {
                    continue;
                };
                let Ok(request) = serde_json::from_str::<serde_json::Value>(text) else {
                    continue;
                };
                let Some(id) = request.get("id") else {
                    continue;
                };
                let result = match request["method"].as_str() {
                    Some("account/read") => {
                        serde_json::json!({"account":null,"requiresOpenaiAuth":true})
                    }
                    Some("account/login/start") => {
                        serde_json::json!({"type":"chatgptDeviceCode","loginId":"synthetic-pending","verificationUrl":"https://example.invalid/device","userCode":"SYNTHETIC"})
                    }
                    _ => serde_json::json!({}),
                };
                if socket
                    .send(tokio_tungstenite::tungstenite::Message::Text(
                        serde_json::json!({"id":id,"result":result})
                            .to_string()
                            .into(),
                    ))
                    .is_err()
                {
                    break;
                }
            }
        });
    }
}

#[test]
fn automatic_refresh_keeps_pending_login_server() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!(
        "chariox-account-refresh-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::env::set_var("CHARIOX_HOME", root.join("state"));
    std::env::set_var("HOME", root.join("home"));
    let registry =
        crate::account_profile::ProviderAccountProfileRegistry::open(root.join("profiles.json"))
            .unwrap();
    registry
        .migrate_effective_defaults("local", &root.join("home"))
        .unwrap();
    let profile = registry
        .create_managed("local", "codex", "pending")
        .unwrap();
    struct Cleanup(PathBuf, String);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            crate::provider::invalidate_codex_account_endpoint("local", &self.1);
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone(), profile.profile_id.clone());
    let log = root.join("launches");
    let executable = root.join("codex");
    let quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    std::fs::write(&executable, format!(
        "#!/bin/sh\nif [ \"$1\" = --version ]; then echo fixture; exit; fi\nwhile [ \"$#\" -gt 0 ]; do if [ \"$1\" = --listen ]; then shift; export CTXFIX_ACCOUNT_LISTEN=\"$1\"; break; fi; shift; done\nexport CTXFIX_ACCOUNT_LOG={}\nexec {} --exact {} --ignored --nocapture\n",
        quote(log.to_str().unwrap()), quote(std::env::current_exe().unwrap().to_str().unwrap()), quote(CHILD)
    )).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::env::set_var("CHARIOX_CODEX_BIN", executable);
    let login = start_provider_login_response(
        &registry,
        "local",
        StartProviderLoginRequest {
            provider: "codex".into(),
            account_profile: profile.profile_id.clone(),
            method: Some("device_code".into()),
        },
    )
    .unwrap();
    assert!(matches!(
        login,
        LocalDaemonResponse::ProviderLoginStarted { .. }
    ));
    let original = std::fs::read_to_string(&log).unwrap();
    let config = DaemonConfig::for_tests();
    for _ in 0..2 {
        let observed = refresh_provider_account_profile_with_config(
            &registry,
            "local",
            "codex",
            &profile.profile_id,
            &config,
        )
        .unwrap();
        assert_eq!(
            observed.auth_state,
            crate::account_profile::ProviderAccountAuthState::NotConfigured
        );
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            original,
            "automatic login observation must preserve the server holding the device challenge"
        );
    }
    refresh_provider_account_profile_explicitly(
        &registry,
        "local",
        "codex",
        &profile.profile_id,
        &config,
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(&log).unwrap().lines().count(),
        2,
        "explicit refresh must still reload the scoped native utility after external login/import"
    );
}
