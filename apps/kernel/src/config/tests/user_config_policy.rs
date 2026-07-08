use super::*;

#[test]
fn machine_pairing_metadata_preserves_approval_state() {
    let mut entries = Vec::new();
    {
        let entry = upsert_machine_registration(&mut entries, "machine-1");
        entry.approved = true;
        entry.alias = Some("worker".to_string());
    }
    {
        let entry = upsert_machine_registration(&mut entries, "machine-1");
        entry.public_key_thumbprint = Some("thumbprint-1".to_string());
        entry.paired_at_ms = Some(42);
    }

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].machine_id, "machine-1");
    assert_eq!(entries[0].alias.as_deref(), Some("worker"));
    assert_eq!(
        entries[0].public_key_thumbprint.as_deref(),
        Some("thumbprint-1")
    );
    assert_eq!(entries[0].paired_at_ms, Some(42));
    assert!(entries[0].approved);
    assert!(!entries[0].forgotten);
}

#[test]
fn client_pairing_upsert_reopens_revoked_client() {
    let mut entries = Vec::new();
    {
        let entry = upsert_client_pairing(&mut entries, "client-1");
        entry.alias = Some("laptop".to_string());
        entry.public_key_thumbprint = "old-thumbprint".to_string();
        entry.paired_at_ms = 10;
        entry.revoked = true;
    }
    {
        let entry = upsert_client_pairing(&mut entries, "client-1");
        entry.public_key_thumbprint = "new-thumbprint".to_string();
        entry.paired_at_ms = 20;
        entry.revoked = false;
    }

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].client_id, "client-1");
    assert_eq!(entries[0].alias.as_deref(), Some("laptop"));
    assert_eq!(entries[0].public_key_thumbprint, "new-thumbprint");
    assert_eq!(entries[0].paired_at_ms, 20);
    assert!(!entries[0].revoked);
}

#[test]
fn user_config_parses_slice_defaults() {
    let payload = r#"
version = 1

[slices]
root = "~/.arroba/slices-dev"

[slices.linux]
docker_image = "arroba-slice-linux-custom:local"
	build_image = "never"
	extension_dockerfile = "~/.arroba/slices/extensions/Dockerfile"
	allow_unconfined_seccomp = true
	memory_mb = 4096
cpus = "2.5"
idle_timeout_minutes = 45
screen_width = 1440
screen_height = 900
"#;

    let config = toml::from_str::<ArrobaUserConfig>(payload).expect("slice config should parse");
    config.validate().expect("slice config should validate");

    assert_eq!(config.slices.root.as_deref(), Some("~/.arroba/slices-dev"));
    assert_eq!(
        config.slices.linux.docker_image.as_deref(),
        Some("arroba-slice-linux-custom:local")
    );
    assert_eq!(
        config.slices.linux.build_image,
        Some(SliceImageBuildPolicy::Never)
    );
    assert_eq!(config.slices.linux.allow_unconfined_seccomp, Some(true));
    assert_eq!(config.slices.linux.memory_mb, Some(4096));
    assert_eq!(config.slices.linux.cpus.as_deref(), Some("2.5"));
    assert_eq!(config.slices.linux.screen_width, Some(1440));
    assert_eq!(config.slices.linux.screen_height, Some(900));
}

#[test]
fn user_config_defaults_to_versioned_slice_image() {
    let config = ArrobaUserConfig::default();

    assert_eq!(
        config.slices.linux.docker_image.as_deref(),
        Some(DEFAULT_LINUX_SLICE_DOCKER_IMAGE)
    );
    assert_ne!(DEFAULT_LINUX_SLICE_DOCKER_IMAGE, "arroba-slice-linux:local");
}

#[test]
fn user_config_parses_credential_vault_service() {
    let payload = r#"
version = 1

[credential_vault]
service = "arroba-test"
"#;

    let config =
        toml::from_str::<ArrobaUserConfig>(payload).expect("credential vault should parse");
    config.validate().expect("credential vault should validate");
    assert_eq!(config.credential_vault.service, "arroba-test");
}

#[test]
fn user_config_parses_credential_vault_backend() {
    let payload = r#"
version = 1

[credential_vault]
backend = "process_memory"
"#;

    let config =
        toml::from_str::<ArrobaUserConfig>(payload).expect("credential vault should parse");
    config.validate().expect("credential vault should validate");
    assert_eq!(
        config.credential_vault.backend,
        CredentialVaultBackend::ProcessMemory
    );
}

#[test]
fn user_config_rejects_unimplemented_credential_vault_unlock_scopes() {
    for unlock_policy in ["session", "agent"] {
        let payload = format!(
            r#"
version = 1

[credential_vault]
unlock_policy = "{unlock_policy}"
"#
        );

        let error = toml::from_str::<ArrobaUserConfig>(&payload)
            .expect_err("unimplemented unlock scope should not parse");
        assert!(error.to_string().contains("kernel_init, ttl, or always"));
    }
}

#[test]
fn persisted_daemon_config_loads_legacy_machine_registry_without_pairing_fields() {
    let payload = r#"{
          "relay_url": "ws://relay",
          "relay_token": "secret",
          "machines": [
            {
              "machine_id": "machine-1",
              "alias": "worker",
              "approved": true,
              "forgotten": false
            }
          ]
        }"#;

    let persisted = serde_json::from_str::<PersistedDaemonConfig>(payload)
        .expect("legacy daemon config should decode");

    assert_eq!(persisted.clients, Vec::<PersistedClientPairing>::new());
    assert_eq!(persisted.machines.len(), 1);
    assert_eq!(persisted.machines[0].machine_id, "machine-1");
    assert_eq!(persisted.machines[0].alias.as_deref(), Some("worker"));
    assert_eq!(persisted.machines[0].public_key_thumbprint, None);
    assert_eq!(persisted.machines[0].paired_at_ms, None);
    assert!(persisted.machines[0].approved);
}

#[test]
fn workspace_live_sync_policy_serializes_default_as_off() {
    let config = DaemonConfig::new("daemon", "machine", "tester");

    assert!(!config.provider_requires_workspace_live_sync("codex"));
    assert!(!config.provider_requires_workspace_live_sync("opencode"));
    assert!(!config.provider_requires_workspace_live_sync("default"));
    assert!(!config.provider_tracks_workspace_live_sync("codex"));
}

#[test]
fn test_config_defaults_to_unrestricted_workspace_live_sync() {
    let config = DaemonConfig::for_tests();

    assert!(!config.provider_requires_workspace_live_sync("dev-stub"));
    assert!(!config.provider_tracks_workspace_live_sync("dev-stub"));
}

#[test]
fn workspace_live_sync_policy_can_be_changed_and_persisted_in_user_config() {
    let path = std::env::temp_dir().join(format!(
        "arroba-user-config-test-{}-{}.toml",
        std::process::id(),
        generate_identity_suffix()
    ));
    let mut config = DaemonConfig::new("daemon", "machine", "tester");
    config.user_config_path = path.clone();

    config
        .set_user_config_value("providers.workspace_live_sync", "off")
        .expect("workspace live sync policy should update");

    assert!(!config.provider_requires_workspace_live_sync("opencode"));
    assert!(!config.provider_requires_workspace_live_sync("codex"));

    let loaded = load_user_config_from_path(&path);
    assert_eq!(
        loaded.providers.workspace_live_sync.mode,
        WorkspaceLiveSyncMode::Unrestricted
    );

    config
        .set_user_config_value("providers.workspace_live_sync", "tracked")
        .expect("tracked workspace live sync policy should update");
    assert_eq!(
        config.user_config.providers.workspace_live_sync.mode,
        WorkspaceLiveSyncMode::Tracked
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn workspace_live_sync_policy_defaults_to_off() {
    let encoded =
        toml::to_string(&UserProviderConfig::default()).expect("provider config should encode");

    assert!(encoded.contains("workspace_live_sync = \"off\""));
    assert!(!encoded.contains("workspace_live_sync = \"managed\""));
}

#[test]
fn remote_lease_acceptance_defaults_to_enabled_and_user_config_is_live() {
    let path = std::env::temp_dir().join(format!(
        "arroba-remote-lease-config-test-{}-{}.toml",
        std::process::id(),
        generate_identity_suffix()
    ));
    let mut config = DaemonConfig::new("daemon", "machine", "tester");
    config.user_config_path = path.clone();

    assert!(config.accept_remote_leases);

    config
        .set_user_config_value("relay.accept_remote_leases", "false")
        .expect("remote lease setting should persist");
    assert!(!config.accept_remote_leases);
    assert_eq!(config.user_config.relay.accept_remote_leases, Some(false));

    config
        .set_user_config_value("relay.accept_remote_leases", "true")
        .expect("remote lease setting should update");
    assert!(config.accept_remote_leases);

    config
        .unset_user_config_value("relay.accept_remote_leases")
        .expect("remote lease setting should unset");
    assert!(config.accept_remote_leases);
    assert_eq!(config.user_config.relay.accept_remote_leases, None);

    let _ = std::fs::remove_file(path);
}

#[test]
fn workspace_live_sync_policy_accepts_managed_config_spelling() {
    let mut config = DaemonConfig::new("daemon", "machine", "tester");

    config
        .set_user_config_value("providers.workspace_live_sync", "managed")
        .expect("managed workspace live sync config spelling should be accepted");

    assert_eq!(
        config.user_config.providers.workspace_live_sync.mode,
        WorkspaceLiveSyncMode::Managed
    );
}

#[test]
fn workspace_live_sync_policy_rejects_legacy_boolean_aliases() {
    for alias in ["on", "true", "1", "false", "0"] {
        let mut config = DaemonConfig::new("daemon", "machine", "tester");

        let error = config
            .set_user_config_value("providers.workspace_live_sync", alias)
            .expect_err("legacy workspace live sync aliases should be rejected");

        assert!(
            matches!(
                error,
                DaemonError::InvalidConfig {
                    field: "providers.workspace_live_sync",
                    ..
                }
            ),
            "alias {alias:?} should be rejected with an invalid config error"
        );
    }
}

#[test]
fn user_config_schema_lists_settable_kernel_owned_keys() {
    let schema = DaemonConfig::user_config_schema();
    let workspace_live_sync = schema
        .iter()
        .find(|entry| entry.path == "providers.workspace_live_sync")
        .expect("workspace live sync schema entry should exist");

    assert!(workspace_live_sync.settable);
    assert!(workspace_live_sync.unsettable);
    assert_eq!(workspace_live_sync.effect, "provider_reload");
    assert_eq!(
        workspace_live_sync.allowed_values,
        vec!["off", "managed", "tracked"]
    );
    assert!(schema
        .iter()
        .any(|entry| entry.path == "ui.worktree_aliases.<alias>"));
    assert!(schema
        .iter()
        .any(|entry| entry.path == "workflow.session_default_max_agents"));
    for path in [
        "workflow.code.max_concurrent",
        "workflow.code.max_nodes",
        "workflow.code.max_agents",
        "workflow.code.max_edges",
        "workflow.code.max_endpoints",
        "workflow.code.max_queues",
        "workflow.code.max_watchdogs",
        "workflow.code.max_schema_bytes",
        "workflow.code.max_generated_prompt_bytes",
        "workflow.code.script_timeout_ms",
        "workflow.code.script_memory_bytes",
    ] {
        assert!(
            schema.iter().any(|entry| entry.path == path),
            "user config schema should expose `{path}`"
        );
    }
}

#[test]
fn workspace_live_sync_policy_rejects_per_provider_setter_keys() {
    let mut config = DaemonConfig::new("daemon", "machine", "tester");

    let set_error = config
        .set_user_config_value("providers.workspace_live_sync.codex", "unrestricted")
        .expect_err("per-provider workspace live sync setters should be rejected");
    let unset_error = config
        .unset_user_config_value("providers.workspace_live_sync.codex")
        .expect_err("per-provider workspace live sync unsets should be rejected");

    assert!(matches!(
        set_error,
        DaemonError::InvalidConfig {
            field: "user_config",
            ..
        }
    ));
    assert!(matches!(
        unset_error,
        DaemonError::InvalidConfig {
            field: "user_config",
            ..
        }
    ));
    assert!(!config.provider_requires_workspace_live_sync("codex"));
}
