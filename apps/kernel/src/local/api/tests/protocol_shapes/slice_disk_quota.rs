use crate::config::CharioxUserConfig;
use crate::local::{LocalDaemonResponse, LOCAL_DAEMON_PROTOCOL_VERSION};

#[test]
fn paired_slice_disk_quota_config_fields_are_versioned_and_serialized() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 459);

    let mut config = CharioxUserConfig::default();
    config.slices.linux.disk_layer_mb = Some(512);
    config.slices.linux.disk_home_mb = Some(2_048);
    let response = LocalDaemonResponse::UserConfig {
        path: "/home/chariox/.chariox/config.toml".into(),
        config,
    };
    let snapshot = serde_json::to_value(response).expect("user config response should encode");

    assert_eq!(
        snapshot.pointer("/UserConfig/config/slices/linux/disk_layer_mb"),
        Some(&serde_json::json!(512)),
    );
    assert_eq!(
        snapshot.pointer("/UserConfig/config/slices/linux/disk_home_mb"),
        Some(&serde_json::json!(2_048)),
    );
}
