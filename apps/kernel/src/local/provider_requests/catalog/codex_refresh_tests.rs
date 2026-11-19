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
    let authenticated = std::env::var_os("CODEX_HOME")
        .and_then(|home| std::fs::read_to_string(PathBuf::from(home).join("auth.json")).ok())
        .is_some_and(|auth| auth.contains("synthetic-accepted-login"));
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
                        if authenticated {
                            serde_json::json!({"account":{"email":"accepted@example.invalid"},"requiresOpenaiAuth":true})
                        } else {
                            serde_json::json!({"account":null,"requiresOpenaiAuth":true})
                        }
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

    // A linked home can retain credential bytes after the provider stops
    // accepting its login. Status must explain recovery instead of erasing it.
    let linked_home = root.join("linked-codex");
    std::fs::create_dir_all(&linked_home).unwrap();
    std::fs::set_permissions(&linked_home, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(
        linked_home.join("auth.json"),
        r#"{"tokens":{"access_token":"synthetic-rejected-login"}}"#,
    )
    .unwrap();
    let linked = registry
        .link_existing("local", "codex", "linked", &linked_home)
        .unwrap();
    let _linked_cleanup = Cleanup(root.join("unused"), linked.profile_id.clone());
    for restart in [false, true] {
        if restart {
            crate::provider::invalidate_codex_account_endpoint("local", &linked.profile_id);
        }
        let reopened = crate::account_profile::ProviderAccountProfileRegistry::open(
            root.join("profiles.json"),
        )
        .unwrap();
        let response = provider_auth_status_response(
            &reopened,
            "local",
            GetProviderAuthStatusRequest {
                provider: "codex".into(),
                account_profile: linked.profile_id.clone(),
            },
            &config,
        )
        .unwrap();
        let LocalDaemonResponse::ProviderAuthStatus { status } = response else {
            panic!("expected auth status")
        };
        assert_eq!(
            status.auth_state, "error",
            "a retained login needs an actionable failure"
        );
        assert!(status.login_hint.unwrap().contains("sign in"));
        let saved =
            serde_json::to_value(reopened.get("local", "codex", &linked.profile_id).unwrap())
                .unwrap();
        assert_eq!(saved["auth_state"], "error");
        assert!(saved["auth_error"].as_str().unwrap().contains("sign in"));
    }
    std::fs::write(
        linked_home.join("auth.json"),
        r#"{"tokens":{"access_token":"synthetic-accepted-login"}}"#,
    )
    .unwrap();
    let response = provider_auth_status_response(
        &registry,
        "local",
        GetProviderAuthStatusRequest {
            provider: "codex".into(),
            account_profile: linked.profile_id.clone(),
        },
        &config,
    )
    .unwrap();
    let LocalDaemonResponse::ProviderAuthStatus { status } = response else {
        panic!("expected auth status")
    };
    assert_eq!(
        status.auth_state, "authenticated",
        "external credential changes must retire the stale cache"
    );
    let saved =
        serde_json::to_value(registry.get("local", "codex", &linked.profile_id).unwrap()).unwrap();
    assert!(
        saved.get("auth_error").is_none(),
        "successful validation clears the failure"
    );
    let binding = crate::provider_account_copy::ProviderAccountCopyBinding {
        operation_id: "cache-alias".into(),
        provider: "codex".into(),
        source_profile_id: linked.profile_id.clone(),
        source_kernel_id: "local-kernel".into(),
        target_kernel_id: "local-kernel".into(),
        source_key_thumbprint: "a".repeat(64),
        target_key_thumbprint: "a".repeat(64),
        replace: false,
    };
    let receipt = registry
        .install_account_copy(
            &config,
            "local",
            &binding,
            &crate::provider_account_copy::ProviderAccountCopyCredential::new(String::new()),
        )
        .unwrap();
    let _alias_cleanup = Cleanup(root.join("unused-alias"), receipt.account_profile.clone());
    let launches = std::fs::read_to_string(&log).unwrap().lines().count();
    for account_profile in [&receipt.account_profile, &linked.profile_id] {
        provider_auth_status_response(
            &registry,
            "local",
            GetProviderAuthStatusRequest {
                provider: "codex".into(),
                account_profile: account_profile.clone(),
            },
            &config,
        )
        .unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(&log).unwrap().lines().count(),
        launches,
        "two linked profiles reuse one app-server for the same native credential"
    );
    std::fs::write(
        linked_home.join("auth.json"),
        r#"{"tokens":{"access_token":"synthetic-accepted-login-rotated"}}"#,
    )
    .unwrap();
    for account_profile in [&linked.profile_id, &receipt.account_profile] {
        let response = provider_auth_status_response(
            &registry,
            "local",
            GetProviderAuthStatusRequest {
                provider: "codex".into(),
                account_profile: account_profile.clone(),
            },
            &config,
        )
        .unwrap();
        assert!(
            matches!(response, LocalDaemonResponse::ProviderAuthStatus { status } if status.auth_state == "authenticated")
        );
    }
    assert_eq!(
        std::fs::read_to_string(&log).unwrap().lines().count(),
        launches + 1,
        "rotation retires the shared cached helper once, not once per linked profile"
    );
    registry
        .mark_logged_out("local", "codex", &receipt.account_profile)
        .unwrap();
    for account_profile in [&linked.profile_id, &receipt.account_profile] {
        let profile = registry.get("local", "codex", account_profile).unwrap();
        assert_eq!(
            profile.auth_state,
            crate::account_profile::ProviderAccountAuthState::NotConfigured
        );
        assert!(serde_json::to_value(profile)
            .unwrap()
            .get("auth_error")
            .is_none());
    }
}
