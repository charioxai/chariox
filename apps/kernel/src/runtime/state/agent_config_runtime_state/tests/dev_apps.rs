use super::*;
use crate::extension::ExtensionGrant;

#[tokio::test]
async fn app_dev_owner_revoke_is_preserved_until_a_requested_regrant() {
    for room_agent_tools in [false, true] {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.room_agent_tools = room_agent_tools;
        let (_app, runtime, _, agent) =
            agent_config_runtime_with_config_and_owner(config, "alice").await;
        let store = &runtime.owned.durable_state_store;
        let _catalog = crate::durable_state::app_state::fixture_catalog(store);
        let granted = runtime
            .grant_agent_app(&agent, ExtensionGrant::app("installed"), "alice", None)
            .await
            .unwrap();
        // A failed choice append used to leave a live grant beside the old
        // revoke. A requested retry must heal that durable choice, including
        // the idempotent timed-grant path, without replacing its grant ID.
        store
            .append_event(
                "app.binding.owner_revoked",
                Some(agent.clone()),
                serde_json::json!({"installation_id":"installed"}),
            )
            .unwrap();
        let retried = runtime
            .grant_agent_app(&agent, ExtensionGrant::app("installed"), "alice", None)
            .await
            .unwrap();
        assert_eq!(granted.extension_grants(), retried.extension_grants());
        assert!(
            store
                .dev_app_automatic_binding_enabled(&agent, "installed")
                .unwrap(),
            "owner retry must persist with room_agent_tools={room_agent_tools}"
        );
        runtime
            .revoke_agent_app(&agent, "installed", "alice")
            .await
            .unwrap();
        assert!(!store
            .dev_app_automatic_binding_enabled(&agent, "installed")
            .unwrap());

        // An old grant's late audit must not undo the serialized owner revoke.
        store
            .append_event(
                "home_extension.grant.created",
                Some(agent.clone()),
                serde_json::json!({"grant":{"kind":"app","name":"installed"}}),
            )
            .unwrap();
        assert!(!store
            .dev_app_automatic_binding_enabled(&agent, "installed")
            .unwrap());
        runtime
            .grant_agent_app(&agent, ExtensionGrant::app("installed"), "alice", None)
            .await
            .unwrap();
        assert!(store
            .dev_app_automatic_binding_enabled(&agent, "installed")
            .unwrap());
    }
}
