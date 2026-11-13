//! Machine settings use the same kernel route for every client.
use super::*;
use crate::local::UnsetUserConfigValueRequest;

#[test]
fn resset_local_config_entry_point_applies_and_persists_each_resource() {
    crate::test_support::isolated_env_test!();
    let mut config = DaemonConfig::for_tests();
    let root = std::env::temp_dir().join(format!("resset-rpc-{:016x}", rand::random::<u64>()));
    config.user_config_path = root.join("config.toml");
    let harness = LocalRouterTestHarness::with_config(config);
    for resource in ["memory", "disk", "cpu"] {
        let path = format!("kernel.resource_protection.{resource}");
        let response = harness
            .dispatch(LocalDaemonRequest::SetUserConfigValue(
                SetUserConfigValueRequest {
                    path: path.clone(),
                    value: "false".into(),
                },
            ))
            .unwrap();
        let wire = serde_json::to_value(response).unwrap();
        let settings = &wire["UserConfigUpdated"]["config"]["kernel"]["resource_protection"];
        assert_eq!(settings[resource], false);
        for other in ["memory", "disk", "cpu"]
            .into_iter()
            .filter(|other| *other != resource)
        {
            assert_eq!(settings[other], true);
        }
        assert_eq!(
            wire["UserConfigUpdated"]["effects"][0]["kind"],
            "runtime_config"
        );
        let persisted: crate::config::CharioxUserConfig =
            toml::from_str(&std::fs::read_to_string(root.join("config.toml")).unwrap()).unwrap();
        assert_eq!(
            serde_json::to_value(persisted).unwrap()["kernel"]["resource_protection"],
            *settings
        );
        harness
            .dispatch(LocalDaemonRequest::UnsetUserConfigValue(
                UnsetUserConfigValueRequest { path },
            ))
            .unwrap();
    }
    drop(harness);
    std::fs::remove_dir_all(root).unwrap();
}
