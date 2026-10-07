use super::*;

#[test]
fn kernel_access_defaults_and_partial_config_round_trip() {
    let config = toml::from_str::<CharioxUserConfig>("").unwrap();
    assert_eq!(config.kernel_access.grant_default_minutes, 480);
    assert_eq!(config.kernel_access.grant_max_minutes, 1440);
    assert_eq!(config.kernel_access.grant_extend_notice_minutes, 5);
    assert_eq!(config.kernel_access.request_timeout_minutes, 10);
    let config = toml::from_str::<CharioxUserConfig>(
        "[kernel_access]\ngrant_default_minutes = 45\nrequest_timeout_minutes = 12",
    )
    .unwrap();
    config.validate().unwrap();
    assert_eq!(config.kernel_access.grant_default_minutes, 45);
    assert_eq!(config.kernel_access.grant_max_minutes, 1440);
    assert_eq!(config.kernel_access.grant_extend_notice_minutes, 5);
    let encoded = toml::to_string(&config).unwrap();
    assert_eq!(
        toml::from_str::<CharioxUserConfig>(&encoded).unwrap(),
        config
    );
}

#[test]
fn kernel_access_config_mutations_validate_and_unset_to_defaults() {
    let mut config = CharioxUserConfig::default();
    for (key, value) in [
        ("grant_default_minutes", 45),
        ("grant_max_minutes", 900),
        ("grant_extend_notice_minutes", 7),
        ("request_timeout_minutes", 12),
    ] {
        let path = format!("kernel_access.{key}");
        config.set_value(&path, value.to_string()).unwrap();
        assert_eq!(
            serde_json::to_value(&config.kernel_access).unwrap()[key],
            value
        );
        config.unset_value(&path).unwrap();
        assert_eq!(config.kernel_access, UserKernelAccessConfig::default());
        for value in ["0", "-1", "bad", "4294967296"] {
            assert!(config.set_value(&path, value.into()).is_err());
            assert_eq!(config.kernel_access, UserKernelAccessConfig::default());
        }
        config = CharioxUserConfig::default();
    }
    for payload in [
        "grant_default_minutes = 1441",
        "grant_max_minutes = 479",
        "grant_max_minutes = 1441",
        "grant_extend_notice_minutes = 480",
        "request_timeout_minutes = 0",
    ] {
        let config =
            toml::from_str::<CharioxUserConfig>(&format!("[kernel_access]\n{payload}")).unwrap();
        assert!(config.validate().is_err(), "{payload}");
    }
}

#[test]
fn kernel_access_rejected_edits_preserve_the_previous_policy() {
    let mut config = CharioxUserConfig::default();
    config
        .set_value("kernel_access.grant_default_minutes", "280".into())
        .unwrap();
    config
        .set_value("kernel_access.grant_max_minutes", "300".into())
        .unwrap();
    let previous = config.kernel_access.clone();
    assert!(config
        .set_value("kernel_access.grant_max_minutes", "200".into())
        .is_err());
    assert_eq!(config.kernel_access, previous);
    assert!(config
        .unset_value("kernel_access.grant_default_minutes")
        .is_err());
    assert_eq!(config.kernel_access, previous);
    assert!(config
        .set_value("kernel_access.unknown", "10".into())
        .is_err());
    assert_eq!(config.kernel_access, previous);
}

#[test]
fn kernel_access_settings_persist_and_are_discoverable() {
    let path = std::env::temp_dir().join(format!(
        "chariox-access-config-{:016x}.toml",
        rand::random::<u64>()
    ));
    let mut config = DaemonConfig::new("daemon", "machine", "tester");
    config.user_config_path = path.clone();
    let schema = DaemonConfig::user_config_schema();
    for (key, value, default) in [
        ("grant_default_minutes", 45, 480),
        ("grant_max_minutes", 900, 1440),
        ("grant_extend_notice_minutes", 7, 5),
        ("request_timeout_minutes", 12, 10),
    ] {
        let key_path = format!("kernel_access.{key}");
        let entry = schema.iter().find(|entry| entry.path == key_path).unwrap();
        assert!(entry.settable && entry.unsettable);
        assert_eq!(entry.effect, "runtime_policy");
        assert_eq!(entry.status, "live");
        config
            .set_user_config_value(&key_path, value.to_string())
            .unwrap();
        let loaded = load_user_config_from_path(&path);
        assert_eq!(
            serde_json::to_value(&loaded.kernel_access).unwrap()[key],
            value
        );
        config.unset_user_config_value(&key_path).unwrap();
        let loaded = load_user_config_from_path(&path);
        assert_eq!(
            serde_json::to_value(&loaded.kernel_access).unwrap()[key],
            default
        );
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn kernel_access_legacy_configs_are_clamped_instead_of_refusing_boot() {
    for (payload, default, max, notice) in [
        // Valid before the 8 h default: only the old maximum was set.
        ("grant_max_minutes = 240", 240, 240, 5),
        // No upper bound existed before the 24 h maximum.
        (
            "grant_default_minutes = 2000\ngrant_max_minutes = 3000\ngrant_extend_notice_minutes = 1800",
            1440,
            1440,
            1439,
        ),
    ] {
        let path = std::env::temp_dir().join(format!(
            "chariox-access-legacy-{:016x}.toml",
            rand::random::<u64>()
        ));
        std::fs::write(&path, format!("[kernel_access]\n{payload}\n")).unwrap();
        let loaded = load_user_config_from_path(&path);
        std::fs::remove_file(path).unwrap();
        loaded.validate().unwrap();
        assert_eq!(
            (
                loaded.kernel_access.grant_default_minutes,
                loaded.kernel_access.grant_max_minutes,
                loaded.kernel_access.grant_extend_notice_minutes,
            ),
            (default, max, notice),
            "{payload}"
        );
    }
}
