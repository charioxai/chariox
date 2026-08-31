use super::persisted_daemon::load_persisted_daemon_config;
use super::*;

#[test]
fn for_tests_uses_fixed_runtime_identity() {
    let config = DaemonConfig::for_tests();
    assert_eq!(config.daemon_id, "daemon-test");
    assert_eq!(config.host_machine_id, "machine-test");
    assert_eq!(config.host_machine_alias, None);
    assert_eq!(config.daemon_alias, None);
}

#[test]
fn kernel_websocket_write_delay_coalesces_events_outside_test_configs() {
    let config = DaemonConfig::new("daemon", "machine", "tester");
    assert_eq!(
        config.kernel_websocket_write_delay_ms,
        DEFAULT_KERNEL_WEBSOCKET_WRITE_DELAY_MS
    );

    let test_config = DaemonConfig::for_tests();
    assert_eq!(test_config.kernel_websocket_write_delay_ms, 0);
}

#[test]
fn relay_heartbeat_defaults_to_human_scale_cadence() {
    let config = DaemonConfig::new("daemon", "machine", "tester");
    assert_eq!(config.relay_heartbeat_ms, DEFAULT_RELAY_HEARTBEAT_MS);
    assert_eq!(config.relay_heartbeat_ms, 5_000);
}

#[test]
fn generated_runtime_identity_has_expected_prefixes() {
    let relay_private_key = relay_crypto::generate_private_key_base64();
    let relay_public_key = relay_crypto::public_key_from_private_key_base64(&relay_private_key)
        .expect("relay public key should derive");
    let identity = RuntimeIdentity {
        daemon_id: format!("daemon-{}", generate_identity_suffix()),
        machine_id: format!("machine-{}", generate_identity_suffix()),
        machine_alias: None,
        daemon_alias: None,
        relay_public_key,
        relay_private_key,
    };
    assert!(identity.daemon_id.starts_with("daemon-"));
    assert!(identity.machine_id.starts_with("machine-"));
    assert!(identity.daemon_id.len() > "daemon-".len());
    assert!(identity.machine_id.len() > "machine-".len());
}

#[test]
fn runtime_identity_is_stable_per_host_port() {
    let _guard = crate::env_lock::lock();
    let temp_home = std::env::temp_dir().join(format!(
        "chariox-config-identity-test-{}",
        generate_identity_suffix()
    ));
    let old_home = env::var_os("HOME");
    let old_xdg_config_home = env::var_os("XDG_CONFIG_HOME");
    let old_xdg_state_home = env::var_os("XDG_STATE_HOME");
    let old_kernel_host = env::var_os("CHARIOX_KERNEL_HOST");
    let old_kernel_port = env::var_os("CHARIOX_KERNEL_PORT");
    unsafe {
        env::set_var("HOME", &temp_home);
        env::remove_var("XDG_CONFIG_HOME");
        env::remove_var("XDG_STATE_HOME");
        env::set_var("CHARIOX_KERNEL_HOST", "127.0.0.1");
        env::set_var("CHARIOX_KERNEL_PORT", "43118");
    }

    let default_identity = DaemonConfig::load_from_env();
    let restarted_default = DaemonConfig::load_from_env();
    unsafe {
        env::set_var("CHARIOX_KERNEL_PORT", "43119");
    }
    let other_port = DaemonConfig::load_from_env();

    unsafe {
        restore_env_var("HOME", old_home);
        restore_env_var("XDG_CONFIG_HOME", old_xdg_config_home);
        restore_env_var("XDG_STATE_HOME", old_xdg_state_home);
        restore_env_var("CHARIOX_KERNEL_HOST", old_kernel_host);
        restore_env_var("CHARIOX_KERNEL_PORT", old_kernel_port);
    }
    let _ = fs::remove_dir_all(temp_home);

    assert_eq!(default_identity.daemon_id, restarted_default.daemon_id);
    assert_eq!(
        default_identity.host_machine_id,
        restarted_default.host_machine_id
    );
    assert_eq!(default_identity.host_machine_id, other_port.host_machine_id);
    assert_ne!(default_identity.daemon_id, other_port.daemon_id);
}

#[test]
fn chariox_home_owns_config_identity_state_and_runtime_paths() {
    let _guard = crate::env_lock::lock();
    let temp_home = std::env::temp_dir().join(format!(
        "chariox-explicit-home-test-{}",
        generate_identity_suffix()
    ));
    let old_chariox_home = env::var_os("CHARIOX_HOME");
    let old_home = env::var_os("HOME");
    let old_xdg_config_home = env::var_os("XDG_CONFIG_HOME");
    let old_xdg_state_home = env::var_os("XDG_STATE_HOME");
    unsafe {
        env::set_var("CHARIOX_HOME", &temp_home);
        env::set_var("HOME", temp_home.join("unrelated-home"));
        env::set_var("XDG_CONFIG_HOME", temp_home.join("unrelated-config"));
        env::set_var("XDG_STATE_HOME", temp_home.join("unrelated-state"));
    }

    let config = DaemonConfig::load_from_env();
    let durable_state_path = config.durable_state_path();

    unsafe {
        restore_env_var("CHARIOX_HOME", old_chariox_home);
        restore_env_var("HOME", old_home);
        restore_env_var("XDG_CONFIG_HOME", old_xdg_config_home);
        restore_env_var("XDG_STATE_HOME", old_xdg_state_home);
    }
    let _ = fs::remove_dir_all(&temp_home);

    assert_eq!(config.user_config_path, temp_home.join("config.toml"));
    assert_eq!(
        durable_state_path,
        temp_home.join("state").join("kernel.db")
    );
    assert_eq!(config.session_history_root(), temp_home.join("sessions"));
    assert!(config.local_socket_path.starts_with(temp_home.join("run")));
}

#[test]
fn renamed_vault_backend_deserializes_to_the_only_supported_encrypted_backend() {
    let config = toml::from_str::<CharioxUserConfig>(
        r#"
            [credential_vault]
            backend = "arroba_encrypted"
        "#,
    )
    .expect("renamed config should migrate while loading");

    assert_eq!(
        config.credential_vault.backend,
        CredentialVaultBackend::CharioxEncrypted
    );
    assert_eq!(
        toml::to_string(&config)
            .expect("config should serialize")
            .matches("arroba_encrypted")
            .count(),
        0,
        "the removed backend name must never be written again"
    );
}

#[test]
fn env_relay_config_takes_precedence_over_persisted_cloud_relay_profile() {
    let _guard = crate::env_lock::lock();
    let temp_home = std::env::temp_dir().join(format!(
        "chariox-config-relay-env-test-{}",
        generate_identity_suffix()
    ));
    let old_home = env::var_os("HOME");
    let old_xdg_config_home = env::var_os("XDG_CONFIG_HOME");
    let old_xdg_state_home = env::var_os("XDG_STATE_HOME");
    let old_relay_url = env::var_os("CHARIOX_RELAY_URL");
    let old_relay_token = env::var_os("CHARIOX_RELAY_TOKEN");
    let old_cloud_relay_config = env::var_os("CHARIOX_CLOUD_RELAY_CONFIG_JSON");
    unsafe {
        env::set_var("HOME", &temp_home);
        env::remove_var("XDG_CONFIG_HOME");
        env::remove_var("XDG_STATE_HOME");
        env::set_var("CHARIOX_RELAY_URL", "ws://127.0.0.1:47000");
        env::set_var("CHARIOX_RELAY_TOKEN", "local-drill-token");
        env::remove_var("CHARIOX_CLOUD_RELAY_CONFIG_JSON");
    }
    let daemon_config_path = DaemonConfig::default_daemon_config_path();
    if let Some(parent) = daemon_config_path.parent() {
        fs::create_dir_all(parent).expect("daemon config parent should be created");
    }
    fs::write(
        &daemon_config_path,
        r#"{
              "relay_url": "wss://cloud-relay.example",
              "relay_token": "cloud-token",
              "cloud_relay": {
                "api_url": "https://cloud.example",
                "email": "test@example.com",
                "account_id": "account-1",
                "user_id": "user-1",
                "account_slug": "account",
                "realm_id": "realm-1",
                "relay_url": "wss://cloud-relay.example",
                "issuer_id": "issuer-1",
                "machine_credential": "machine-credential",
                "token_expires_at_ms": 1
              }
            }"#,
    )
    .expect("daemon config should write");

    let mut config = DaemonConfig::load_from_env();
    config
        .persist_relay_config()
        .expect("local relay config should persist");
    let persisted_after_local_override = load_persisted_daemon_config();
    assert!(persisted_after_local_override.cloud_relay.is_some());
    config
        .persist_cloud_relay_profile(None)
        .expect("explicit Cloud sign-out should persist");
    assert!(load_persisted_daemon_config().cloud_relay.is_none());

    unsafe {
        restore_env_var("HOME", old_home);
        restore_env_var("XDG_CONFIG_HOME", old_xdg_config_home);
        restore_env_var("XDG_STATE_HOME", old_xdg_state_home);
        restore_env_var("CHARIOX_RELAY_URL", old_relay_url);
        restore_env_var("CHARIOX_RELAY_TOKEN", old_relay_token);
        restore_env_var("CHARIOX_CLOUD_RELAY_CONFIG_JSON", old_cloud_relay_config);
    }
    let _ = fs::remove_dir_all(temp_home);

    assert_eq!(config.relay_url.as_deref(), Some("ws://127.0.0.1:47000"));
    assert_eq!(config.relay_token.as_deref(), Some("local-drill-token"));
    assert_eq!(config.cloud_relay, None);
}

#[test]
fn relay_url_uses_cloud_profile_tolerates_spacing_and_trailing_slashes() {
    let mut config = DaemonConfig::for_tests();
    config.cloud_relay = Some(PersistedCloudRelayProfile {
        api_url: "https://cloud.example.test".to_string(),
        email: "user@example.test".to_string(),
        account_id: "account-1".to_string(),
        user_id: "user-1".to_string(),
        account_slug: "account".to_string(),
        realm_id: "realm-1".to_string(),
        relay_url: "wss://relay.example.test/".to_string(),
        issuer_id: "issuer-1".to_string(),
        client_id: None,
        client_alias: None,
        machine_id: Some("machine-1".to_string()),
        machine_alias: None,
        machine_credential: Some("machine-secret".to_string()),
        cloud_session_token: None,
        cloud_session_expires_at_ms: None,
        token_expires_at_ms: Some(42),
    });

    assert!(config.relay_url_uses_cloud_profile(" wss://relay.example.test "));
    assert!(config.relay_url_uses_cloud_profile("wss://relay.example.test//"));
    assert!(!config.relay_url_uses_cloud_profile("wss://other-relay.example.test"));
}

#[test]
fn env_cloud_profile_can_accompany_env_relay_config_for_worker_refresh() {
    let _guard = crate::env_lock::lock();
    let temp_home = std::env::temp_dir().join(format!(
        "chariox-config-env-cloud-relay-test-{}",
        generate_identity_suffix()
    ));
    let old_home = env::var_os("HOME");
    let old_xdg_config_home = env::var_os("XDG_CONFIG_HOME");
    let old_xdg_state_home = env::var_os("XDG_STATE_HOME");
    let old_relay_url = env::var_os("CHARIOX_RELAY_URL");
    let old_relay_token = env::var_os("CHARIOX_RELAY_TOKEN");
    let old_cloud_relay_config = env::var_os("CHARIOX_CLOUD_RELAY_CONFIG_JSON");
    unsafe {
        env::set_var("HOME", &temp_home);
        env::remove_var("XDG_CONFIG_HOME");
        env::remove_var("XDG_STATE_HOME");
        env::set_var("CHARIOX_RELAY_URL", "wss://195.201.123.115.sslip.io");
        env::set_var("CHARIOX_RELAY_TOKEN", "runtime-token");
        env::set_var(
            "CHARIOX_CLOUD_RELAY_CONFIG_JSON",
            r#"{
                  "cloud_relay": {
                    "api_url": "https://staging.chariox.com",
                    "email": "worker@example.com",
                    "account_id": "account-1",
                    "user_id": "user-1",
                    "account_slug": "account",
                    "realm_id": "realm-1",
                    "relay_url": "ws://195.201.123.115:43130",
                    "issuer_id": "chariox-cloud-staging",
                    "machine_id": "machine-1",
                    "machine_credential": "machine-credential",
                    "token_expires_at_ms": 1
                  }
                }"#,
        );
    }

    let config = DaemonConfig::load_from_env();

    unsafe {
        restore_env_var("HOME", old_home);
        restore_env_var("XDG_CONFIG_HOME", old_xdg_config_home);
        restore_env_var("XDG_STATE_HOME", old_xdg_state_home);
        restore_env_var("CHARIOX_RELAY_URL", old_relay_url);
        restore_env_var("CHARIOX_RELAY_TOKEN", old_relay_token);
        restore_env_var("CHARIOX_CLOUD_RELAY_CONFIG_JSON", old_cloud_relay_config);
    }
    let _ = fs::remove_dir_all(temp_home);

    assert_eq!(
        config.relay_url.as_deref(),
        Some("wss://195.201.123.115.sslip.io")
    );
    assert_eq!(config.relay_token.as_deref(), Some("runtime-token"));
    let profile = config
        .cloud_relay
        .expect("env cloud profile should be loaded with env relay config");
    assert_eq!(profile.account_id, "account-1");
    assert_eq!(profile.machine_id.as_deref(), Some("machine-1"));
    assert_eq!(profile.relay_url, HOSTED_STAGING_RELAY_URL);
}

#[test]
fn managed_slice_owner_public_key_loads_from_runtime_environment() {
    let _guard = crate::env_lock::lock();
    let temp_home = std::env::temp_dir().join(format!(
        "chariox-config-slice-owner-key-test-{}",
        generate_identity_suffix()
    ));
    let old_home = env::var_os("HOME");
    let old_xdg_config_home = env::var_os("XDG_CONFIG_HOME");
    let old_xdg_state_home = env::var_os("XDG_STATE_HOME");
    let old_owner_public_key = env::var_os("CHARIOX_MANAGED_SLICE_RELAY_OWNER_PUBLIC_KEY");
    unsafe {
        env::set_var("HOME", &temp_home);
        env::remove_var("XDG_CONFIG_HOME");
        env::remove_var("XDG_STATE_HOME");
        env::set_var(
            "CHARIOX_MANAGED_SLICE_RELAY_OWNER_PUBLIC_KEY",
            "  slice-owner-public-key  ",
        );
    }

    let config = DaemonConfig::load_from_env();

    unsafe {
        restore_env_var("HOME", old_home);
        restore_env_var("XDG_CONFIG_HOME", old_xdg_config_home);
        restore_env_var("XDG_STATE_HOME", old_xdg_state_home);
        restore_env_var(
            "CHARIOX_MANAGED_SLICE_RELAY_OWNER_PUBLIC_KEY",
            old_owner_public_key,
        );
    }
    let _ = fs::remove_dir_all(temp_home);

    assert_eq!(
        config.managed_slice_relay_owner_public_key.as_deref(),
        Some("slice-owner-public-key")
    );
}

#[test]
fn load_from_env_imports_cli_cloud_profile_for_kernel_startup() {
    let _guard = crate::env_lock::lock();
    let temp_home = std::env::temp_dir().join(format!(
        "chariox-config-cli-cloud-import-test-{}",
        generate_identity_suffix()
    ));
    let old_home = env::var_os("HOME");
    let old_xdg_config_home = env::var_os("XDG_CONFIG_HOME");
    let old_xdg_state_home = env::var_os("XDG_STATE_HOME");
    let old_relay_url = env::var_os("CHARIOX_RELAY_URL");
    let old_relay_token = env::var_os("CHARIOX_RELAY_TOKEN");
    let old_cloud_relay_config = env::var_os("CHARIOX_CLOUD_RELAY_CONFIG_JSON");
    unsafe {
        env::set_var("HOME", &temp_home);
        env::remove_var("XDG_CONFIG_HOME");
        env::remove_var("XDG_STATE_HOME");
        env::remove_var("CHARIOX_RELAY_URL");
        env::remove_var("CHARIOX_RELAY_TOKEN");
        env::remove_var("CHARIOX_CLOUD_RELAY_CONFIG_JSON");
    }
    let preferences_path = temp_home.join(".chariox").join("config.json");
    fs::create_dir_all(preferences_path.parent().expect("preferences parent"))
        .expect("preferences parent should be created");
    fs::write(
        &preferences_path,
        r#"{
              "relay": {
                "cloud": {
                  "apiUrl": "https://staging.chariox.com",
                  "email": "test@example.com",
                  "accountId": "account-1",
                  "userId": "user-1",
                  "accountSlug": "account",
                  "realmId": "realm-1",
                  "relayUrl": "ws://195.201.123.115:43130",
                  "issuerId": "chariox-cloud-staging",
                  "machineId": "machine-1",
                  "machineCredential": "machine-credential",
                  "cloudSessionToken": "session-token",
                  "cloudSessionExpiresAtMs": 12345
                }
              }
            }"#,
    )
    .expect("CLI preferences should write");

    let config = DaemonConfig::load_from_env();

    unsafe {
        restore_env_var("HOME", old_home);
        restore_env_var("XDG_CONFIG_HOME", old_xdg_config_home);
        restore_env_var("XDG_STATE_HOME", old_xdg_state_home);
        restore_env_var("CHARIOX_RELAY_URL", old_relay_url);
        restore_env_var("CHARIOX_RELAY_TOKEN", old_relay_token);
        restore_env_var("CHARIOX_CLOUD_RELAY_CONFIG_JSON", old_cloud_relay_config);
    }
    let _ = fs::remove_dir_all(temp_home);

    let profile = config
        .cloud_relay
        .expect("CLI cloud profile should seed kernel cloud relay");
    assert_eq!(profile.account_id, "account-1");
    assert_eq!(profile.machine_id.as_deref(), Some("machine-1"));
    assert_eq!(
        profile.machine_credential.as_deref(),
        Some("machine-credential")
    );
    assert_eq!(profile.relay_url, HOSTED_STAGING_RELAY_URL);
    assert_eq!(config.relay_url, None);
    assert_eq!(config.relay_token, None);
}

#[test]
fn persisted_daemon_cloud_profile_takes_precedence_over_cli_profile() {
    let _guard = crate::env_lock::lock();
    let temp_home = std::env::temp_dir().join(format!(
        "chariox-config-daemon-cloud-precedence-test-{}",
        generate_identity_suffix()
    ));
    let old_home = env::var_os("HOME");
    let old_xdg_config_home = env::var_os("XDG_CONFIG_HOME");
    let old_xdg_state_home = env::var_os("XDG_STATE_HOME");
    let old_relay_url = env::var_os("CHARIOX_RELAY_URL");
    let old_relay_token = env::var_os("CHARIOX_RELAY_TOKEN");
    let old_cloud_relay_config = env::var_os("CHARIOX_CLOUD_RELAY_CONFIG_JSON");
    unsafe {
        env::set_var("HOME", &temp_home);
        env::remove_var("XDG_CONFIG_HOME");
        env::remove_var("XDG_STATE_HOME");
        env::remove_var("CHARIOX_RELAY_URL");
        env::remove_var("CHARIOX_RELAY_TOKEN");
        env::remove_var("CHARIOX_CLOUD_RELAY_CONFIG_JSON");
    }
    let daemon_config_path = DaemonConfig::default_daemon_config_path();
    fs::create_dir_all(daemon_config_path.parent().expect("daemon config parent"))
        .expect("daemon config parent should be created");
    fs::write(
        &daemon_config_path,
        r#"{
              "cloud_relay": {
                "api_url": "https://daemon-cloud.example",
                "email": "daemon@example.com",
                "account_id": "daemon-account",
                "user_id": "daemon-user",
                "account_slug": "daemon",
                "realm_id": "daemon-realm",
                "relay_url": "wss://daemon-relay.example",
                "issuer_id": "daemon-issuer",
                "machine_credential": "daemon-machine-credential"
              }
            }"#,
    )
    .expect("daemon config should write");
    let preferences_path = temp_home.join(".chariox").join("config.json");
    fs::write(
        &preferences_path,
        r#"{
              "relay": {
                "cloud": {
                  "apiUrl": "https://cli-cloud.example",
                  "email": "cli@example.com",
                  "accountId": "cli-account",
                  "userId": "cli-user",
                  "accountSlug": "cli",
                  "realmId": "cli-realm",
                  "relayUrl": "wss://cli-relay.example",
                  "issuerId": "cli-issuer",
                  "machineCredential": "cli-machine-credential"
                }
              }
            }"#,
    )
    .expect("CLI preferences should write");

    let config = DaemonConfig::load_from_env();

    unsafe {
        restore_env_var("HOME", old_home);
        restore_env_var("XDG_CONFIG_HOME", old_xdg_config_home);
        restore_env_var("XDG_STATE_HOME", old_xdg_state_home);
        restore_env_var("CHARIOX_RELAY_URL", old_relay_url);
        restore_env_var("CHARIOX_RELAY_TOKEN", old_relay_token);
        restore_env_var("CHARIOX_CLOUD_RELAY_CONFIG_JSON", old_cloud_relay_config);
    }
    let _ = fs::remove_dir_all(temp_home);

    let profile = config
        .cloud_relay
        .expect("daemon cloud profile should be loaded");
    assert_eq!(profile.account_id, "daemon-account");
    assert_eq!(profile.relay_url, "wss://daemon-relay.example");
}
