use super::*;
use crate::extension::ExtensionGrant;

#[tokio::test]
async fn app_dev_owner_revoke_is_preserved_until_a_requested_regrant() {
    let (_app, runtime, _, agent) = agent_config_runtime_with_config_and_owner(
        crate::config::DaemonConfig::for_tests(),
        "alice",
    )
    .await;
    let store = &runtime.owned.durable_state_store;
    let _catalog = crate::durable_state::app_state::fixture_catalog(store);
    runtime
        .grant_agent_app(&agent, ExtensionGrant::app("installed"), "alice", None)
        .await
        .unwrap();
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
