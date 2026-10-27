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
    crate::test_support::isolated_env_test!();
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
fn protected_room_restart_preserves_retained_identity_and_validates_slice_binding() {
    let _guard = crate::env_lock::lock();
    let directory = env::temp_dir().join(format!(
        "chariox-protected-identity-config-{}",
        generate_identity_suffix()
    ));
    fs::create_dir(&directory).expect("create isolated synthetic identity fixture");
    struct Restore(
        Vec<(&'static str, Option<std::ffi::OsString>)>,
        std::path::PathBuf,
    );
    impl Drop for Restore {
        fn drop(&mut self) {
            for (name, value) in self.0.drain(..) {
                unsafe { restore_env_var(name, value) };
            }
            let _ = fs::remove_dir_all(&self.1);
        }
    }
    let names = [
        "HOME",
        "CHARIOX_HOME",
        "CHARIOX_SLICE_PRIVATE_ROOT",
        "CHARIOX_KERNEL_HOST",
        "CHARIOX_KERNEL_PORT",
        "CHARIOX_DAEMON_ID",
        "CHARIOX_MACHINE_ID",
        "CHARIOX_DAEMON_ALIAS",
        "CHARIOX_MACHINE_ALIAS",
        "CHARIOX_SLICE_ID",
        "CHARIOX_ROOM_ENVIRONMENT_HOME_KERNEL_ID",
        "CHARIOX_ROOM_ENVIRONMENT_HOME_PUBLIC_KEY",
        "CHARIOX_ROOM_ENVIRONMENT_SESSION_ID",
        "CHARIOX_ROOM_ENVIRONMENT_SLICE_ID",
        "CHARIOX_RELAY_URL",
        "CHARIOX_RELAY_TOKEN",
        "CHARIOX_CLOUD_RELAY_CONFIG_JSON",
    ];
    let _restore = Restore(
        names
            .into_iter()
            .map(|name| (name, env::var_os(name)))
            .collect(),
        directory.clone(),
    );
    let public = relay_crypto::public_key_from_private_key_base64(
        &relay_crypto::generate_private_key_base64(),
    )
    .unwrap();
    let foreign_public = relay_crypto::public_key_from_private_key_base64(
        &relay_crypto::generate_private_key_base64(),
    )
    .unwrap();
    unsafe {
        env::remove_var("CHARIOX_RELAY_URL");
        env::remove_var("CHARIOX_RELAY_TOKEN");
        env::remove_var("CHARIOX_CLOUD_RELAY_CONFIG_JSON");
        env::set_var("HOME", &directory);
        env::set_var("CHARIOX_HOME", &directory);
        env::set_var(
            "CHARIOX_SLICE_PRIVATE_ROOT",
            "/var/lib/chariox/slice-private",
        );
        env::set_var("CHARIOX_KERNEL_HOST", "127.0.0.1");
        env::set_var("CHARIOX_KERNEL_PORT", "43119");
        env::set_var("CHARIOX_DAEMON_ID", "foreign-kernel");
        env::set_var("CHARIOX_MACHINE_ID", "slice:synthetic-slice");
        env::set_var("CHARIOX_SLICE_ID", "synthetic-slice");
        env::set_var("CHARIOX_ROOM_ENVIRONMENT_HOME_KERNEL_ID", "synthetic-home");
        env::set_var("CHARIOX_ROOM_ENVIRONMENT_HOME_PUBLIC_KEY", public.clone());
        env::set_var("CHARIOX_ROOM_ENVIRONMENT_SESSION_ID", "synthetic-room");
        env::set_var("CHARIOX_ROOM_ENVIRONMENT_SLICE_ID", "synthetic-slice");
        env::set_var("CHARIOX_DAEMON_ALIAS", "display-only-kernel");
        env::set_var("CHARIOX_MACHINE_ALIAS", "display-only-machine");
    }
    let registry = DaemonConfig::default_kernel_registry_path();
    fs::create_dir_all(registry.parent().unwrap()).unwrap();
    let sentinel = serde_json::to_vec(&serde_json::json!({"version": 1, "machine_id": "retained-machine",
        "kernels": {"127.0.0.1:43119": {"kernel_id": "retained-kernel", "host": "127.0.0.1", "port": 43119,
            "relay_public_key": "synthetic-public", "relay_private_key": "synthetic-private-sentinel"}}})).unwrap();
    fs::write(&registry, &sentinel).unwrap();
    // Exercise the same environment composition used by load_from_env twice.
    // Only private identity loading is substituted; no runtime keys are created.
    let load = || {
        DaemonConfig::load_from_env_with_identity_loader(|host, port| {
            assert_eq!((host, port), ("127.0.0.1", 43119));
            assert!(fs::read(&registry).unwrap() == sentinel);
            RuntimeIdentity {
                daemon_id: "retained-kernel".to_string(),
                machine_id: "retained-machine".to_string(),
                machine_alias: None,
                daemon_alias: None,
                relay_public_key: "synthetic-public".to_string(),
                relay_private_key: "synthetic-private-sentinel".to_string(),
            }
        })
    };
    let first = load();
    let restarted = load();
    assert_eq!(first.daemon_id, "retained-kernel");
    assert_eq!(restarted.daemon_id, "retained-kernel");
    assert_eq!(first.host_machine_id, "retained-machine");
    assert_eq!(restarted.host_machine_id, "retained-machine");
    assert!(fs::read(&registry).unwrap() == sentinel);
    for config in [&first, &restarted] {
        let binding = config.room_environment_worker_binding.as_ref().unwrap();
        config
            .validate()
            .expect("retained Room worker must pass the boot config gate");
        let key = public.clone();
        assert!(binding.permits("synthetic-home", &key, "synthetic-room", "synthetic-slice"));
        assert!(!binding.permits("foreign-home", &key, "synthetic-room", "synthetic-slice"));
        assert!(!binding.permits(
            "synthetic-home",
            &foreign_public.clone(),
            "synthetic-room",
            "synthetic-slice"
        ));
        assert!(!binding.permits("synthetic-home", &key, "foreign-room", "synthetic-slice"));
        assert!(!binding.permits("synthetic-home", &key, "synthetic-room", "foreign-slice"));
    }
    for slice in [None, Some("foreign-slice"), Some(" synthetic-slice ")] {
        unsafe {
            restore_env_var("CHARIOX_SLICE_ID", slice.map(Into::into));
        }
        // Validation and cloning use the captured boot scope, not ambient env.
        restarted
            .clone()
            .validate()
            .expect("loaded binding must remain stable");
        assert!(matches!(
            load().validate(),
            Err(crate::error::DaemonError::InvalidConfig {
                field: "room_environment_worker_binding",
                ..
            })
        ));
    }
    unsafe {
        env::remove_var("CHARIOX_SLICE_PRIVATE_ROOT");
    }
    restarted
        .validate()
        .expect("protected mode was captured at boot");
    let legacy = load();
    legacy
        .validate()
        .expect("legacy provisioned machine alias must still boot");
    let binding = legacy.room_environment_worker_binding.as_ref().unwrap();
    assert!(binding
        .validate(&legacy.daemon_id, &restarted.host_machine_id)
        .is_err());
    assert!(binding
        .validate(&legacy.daemon_id, "slice:synthetic-slice")
        .is_ok());
    assert!(binding
        .validate(&legacy.daemon_id, "slice:foreign-slice")
        .is_err());
}

#[test]
fn protected_slice_announces_its_canonical_worker_ref_with_retained_keys() {
    use sha2::{Digest, Sha256};
    let _guard = crate::env_lock::lock();
    let directory = env::temp_dir().join(format!(
        "chariox-protected-worker-ref-{}",
        generate_identity_suffix()
    ));
    let fixture_root = directory;
    let directory = fixture_root.join("deep/".repeat(40)).join("kernel");
    fs::create_dir_all(&directory).expect("create isolated synthetic identity fixture");
    struct Restore(
        Vec<(&'static str, Option<std::ffi::OsString>)>,
        std::path::PathBuf,
    );
    impl Drop for Restore {
        fn drop(&mut self) {
            for (name, value) in self.0.drain(..) {
                unsafe { restore_env_var(name, value) };
            }
            let _ = fs::remove_dir_all(&self.1);
        }
    }
    let names = [
        "HOME",
        "CHARIOX_HOME",
        "CHARIOX_SLICE_PRIVATE_ROOT",
        "CHARIOX_KERNEL_HOST",
        "CHARIOX_KERNEL_PORT",
        "CHARIOX_DAEMON_ID",
        "CHARIOX_DAEMON_SOCKET",
        "CHARIOX_MACHINE_ID",
        "CHARIOX_SLICE_OWNER_MACHINE_ID",
        "CHARIOX_SLICE_ID",
        "CHARIOX_ROOM_ENVIRONMENT_HOME_KERNEL_ID",
        "CHARIOX_ROOM_ENVIRONMENT_HOME_PUBLIC_KEY",
        "CHARIOX_ROOM_ENVIRONMENT_SESSION_ID",
        "CHARIOX_ROOM_ENVIRONMENT_SLICE_ID",
        "CHARIOX_RELAY_URL",
        "CHARIOX_RELAY_TOKEN",
        "CHARIOX_CLOUD_RELAY_CONFIG_JSON",
    ];
    let _restore = Restore(
        names
            .into_iter()
            .map(|name| (name, env::var_os(name)))
            .collect(),
        fixture_root.clone(),
    );
    let worker_ref_for = |machine: &str| {
        format!(
            "slice:{:x}:{}",
            Sha256::digest(machine.as_bytes()),
            "0".repeat(64)
        )
    };
    let worker_ref = worker_ref_for("synthetic-owner-machine");
    unsafe {
        for name in [
            "CHARIOX_DAEMON_SOCKET",
            "CHARIOX_RELAY_URL",
            "CHARIOX_RELAY_TOKEN",
            "CHARIOX_CLOUD_RELAY_CONFIG_JSON",
            "CHARIOX_SLICE_ID",
            "CHARIOX_ROOM_ENVIRONMENT_HOME_KERNEL_ID",
            "CHARIOX_ROOM_ENVIRONMENT_HOME_PUBLIC_KEY",
            "CHARIOX_ROOM_ENVIRONMENT_SESSION_ID",
            "CHARIOX_ROOM_ENVIRONMENT_SLICE_ID",
        ] {
            env::remove_var(name);
        }
        env::set_var("HOME", &directory);
        env::set_var("CHARIOX_HOME", &directory);
        env::set_var(
            "CHARIOX_SLICE_PRIVATE_ROOT",
            "/var/lib/chariox/slice-private",
        );
        env::set_var("CHARIOX_KERNEL_HOST", "127.0.0.1");
        env::set_var("CHARIOX_KERNEL_PORT", "43119");
        env::set_var("CHARIOX_MACHINE_ID", "slice:synthetic-slice");
        env::set_var("CHARIOX_SLICE_OWNER_MACHINE_ID", "synthetic-owner-machine");
        env::set_var("CHARIOX_DAEMON_ID", &worker_ref);
    }
    let load = || {
        DaemonConfig::load_from_env_with_identity_loader(|_, _| RuntimeIdentity {
            daemon_id: "retained-kernel".to_string(),
            machine_id: "retained-machine".to_string(),
            machine_alias: None,
            daemon_alias: None,
            relay_public_key: "synthetic-public".to_string(),
            relay_private_key: "synthetic-private-sentinel".to_string(),
        })
    };
    // The home discovers the worker by its canonical per-creation ref; the
    // retained keys and machine identity stay authoritative.
    let config = load();
    assert_eq!(config.daemon_id, worker_ref);
    assert_eq!(
        config.local_socket_path,
        DaemonConfig::default_local_socket_path("retained-kernel")
    );
    assert_ne!(
        config.local_socket_path,
        DaemonConfig::default_local_socket_path(&worker_ref)
    );
    assert!(config.local_socket_path.as_os_str().len() < 104);
    assert_eq!(load().local_socket_path, config.local_socket_path);
    assert_eq!(config.host_machine_id, "retained-machine");
    assert_eq!(config.relay_public_key, "synthetic-public");
    // No other ambient value can rename the retained kernel.
    for foreign in [
        "foreign-kernel".to_string(),
        "slice:synthetic-slice".to_string(),
        worker_ref_for("foreign-owner-machine"),
    ] {
        unsafe { env::set_var("CHARIOX_DAEMON_ID", &foreign) };
        assert_eq!(load().daemon_id, "retained-kernel", "{foreign}");
    }
    unsafe {
        env::set_var("CHARIOX_DAEMON_ID", &worker_ref);
        env::remove_var("CHARIOX_SLICE_OWNER_MACHINE_ID");
    }
    assert_eq!(load().daemon_id, "retained-kernel");
    // Unprotected workers keep using the provisioned id as before.
    unsafe { env::remove_var("CHARIOX_SLICE_PRIVATE_ROOT") };
    assert_eq!(load().daemon_id, worker_ref);
}

#[test]
fn chariox_home_owns_config_identity_state_and_runtime_paths() {
    crate::test_support::isolated_env_test!();
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
    let expected_socket = DaemonConfig::default_local_socket_path(&config.daemon_id);

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
    assert!(config
        .local_socket_path
        .starts_with(format!("/tmp/chariox-{}", unsafe { libc::geteuid() })));
    assert_eq!(config.local_socket_path, expected_socket);
}

#[test]
fn relay_peer_public_key_claim_survives_restart_and_rejects_rebinding() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let temp_home = std::env::temp_dir().join(format!(
        "chariox-relay-peer-key-test-{}",
        generate_identity_suffix()
    ));
    let old_chariox_home = env::var_os("CHARIOX_HOME");
    unsafe {
        env::set_var("CHARIOX_HOME", &temp_home);
    }

    let first_private_key = relay_crypto::generate_private_key_base64();
    let first_public_key = relay_crypto::public_key_from_private_key_base64(&first_private_key)
        .expect("first public key should derive");
    let second_private_key = relay_crypto::generate_private_key_base64();
    let second_public_key = relay_crypto::public_key_from_private_key_base64(&second_private_key)
        .expect("second public key should derive");

    assert!(
        DaemonConfig::claim_relay_peer_public_key("worker-1", &first_public_key)
            .expect("first authenticated key should persist")
    );
    assert!(
        DaemonConfig::claim_relay_peer_public_key("worker-1", &first_public_key)
            .expect("the same authenticated key should be idempotent")
    );
    assert!(
        !DaemonConfig::claim_relay_peer_public_key("worker-1", &second_public_key)
            .expect("a different key should be rejected")
    );
    assert_eq!(
        DaemonConfig::relay_peer_public_key_entries()
            .get("worker-1")
            .map(String::as_str),
        Some(first_public_key.as_str())
    );

    unsafe {
        restore_env_var("CHARIOX_HOME", old_chariox_home);
    }
    let _ = fs::remove_dir_all(temp_home);
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
    crate::test_support::isolated_env_test!();
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
    assert!(load_persisted_daemon_config().cloud_relay.is_some());
    assert!(DaemonConfig::load_from_env().cloud_relay.is_none());

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
        kernel_id: None,
        kernel_credential: None,
        kernel_public_key_thumbprint: None,
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
    crate::test_support::isolated_env_test!();
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
    crate::test_support::isolated_env_test!();
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
fn load_from_env_never_imports_cli_cloud_credentials() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    std::env::remove_var("CHARIOX_HOME");
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
        env::set_var("CHARIOX_HOME", temp_home.join(".chariox"));
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

    assert!(config.cloud_relay.is_none());
    assert_eq!(config.relay_url, None);
    assert_eq!(config.relay_token, None);
}

#[test]
fn persisted_daemon_cloud_profile_takes_precedence_over_cli_profile() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    std::env::remove_var("CHARIOX_HOME");
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
        env::set_var("CHARIOX_HOME", temp_home.join(".chariox"));
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

#[test]
fn for_tests_resolves_temporary_aliases_before_opening_private_app_uploads() {
    crate::test_support::isolated_env_test!();
    let _environment = crate::env_lock::lock();
    let root = fs::canonicalize(env::temp_dir()).unwrap().join(format!(
        "chariox-test-temp-alias-{}",
        generate_identity_suffix()
    ));
    fs::create_dir(&root).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    let real = root.join("real");
    fs::create_dir(&real).unwrap();
    let alias = root.join("system-alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    env::set_var("TMPDIR", &alias);
    let config = DaemonConfig::for_tests();
    let database = config.durable_state_path();
    assert!(database.starts_with(&real));
    fs::create_dir_all(database.parent().unwrap()).unwrap();
    let uploads = chariox_app_runtime::package_upload::PackageUploadStore::open_or_create(
        &database,
        Default::default(),
        0,
    )
    .expect("canonical test-state root must admit private App uploads");
    uploads
        .begin(
            "alice",
            "alias-upload",
            1,
            &format!("sha256:{}", "0".repeat(64)),
            1_000,
            0,
        )
        .unwrap();
}
