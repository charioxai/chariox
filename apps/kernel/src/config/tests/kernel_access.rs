use super::*;

#[test]
fn kernel_access_defaults_and_partial_config_round_trip() {
    let config = toml::from_str::<CharioxUserConfig>("").unwrap();
    assert_eq!(config.kernel_access.grant_default_minutes, 30);
    assert_eq!(config.kernel_access.grant_max_minutes, 240);
    assert_eq!(config.kernel_access.grant_extend_notice_minutes, 5);
    assert_eq!(config.kernel_access.request_timeout_minutes, 10);
    let config = toml::from_str::<CharioxUserConfig>(
        "[kernel_access]\ngrant_default_minutes = 45\nrequest_timeout_minutes = 12",
    ).unwrap();
    config.validate().unwrap();
    assert_eq!(config.kernel_access.grant_default_minutes, 45);
    assert_eq!(config.kernel_access.grant_max_minutes, 240);
    assert_eq!(config.kernel_access.grant_extend_notice_minutes, 5);
    let encoded = toml::to_string(&config).unwrap();
    assert_eq!(toml::from_str::<CharioxUserConfig>(&encoded).unwrap(), config);
}

#[test]
fn kernel_access_config_mutations_validate_and_unset_to_defaults() {
    let mut config = CharioxUserConfig::default();
    for (key, value) in [
        ("grant_default_minutes", 45),
        ("grant_max_minutes", 300),
        ("grant_extend_notice_minutes", 7),
        ("request_timeout_minutes", 12),
    ] {
        let path = format!("kernel_access.{key}");
        config.set_value(&path, value.to_string()).unwrap();
        assert_eq!(serde_json::to_value(&config.kernel_access).unwrap()[key], value);
        config.unset_value(&path).unwrap();
        assert_eq!(config.kernel_access, UserKernelAccessConfig::default());
        for value in ["0", "-1", "bad", "4294967296"] {
            assert!(config.set_value(&path, value.into()).is_err());
        }
        config = CharioxUserConfig::default();
    }
    for payload in [
        "grant_default_minutes = 241",
        "grant_max_minutes = 29",
        "grant_extend_notice_minutes = 30",
        "request_timeout_minutes = 0",
    ] {
        let config = toml::from_str::<CharioxUserConfig>(&format!("[kernel_access]\n{payload}")).unwrap();
        assert!(config.validate().is_err(), "{payload}");
    }
}
