use super::*;

#[test]
fn managed_login_copy_protocol_455_snapshot_and_hash() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 455);
    let wire = serde_json::json!({"auth_state":"needs_login","copied_at_ms":1,"renewable_services":["anthropic","openai"],"source_account_id":"source-account","source_kernel_id":"source-kernel","source_machine_id":"source-machine","target_account_id":"target-account","target_kernel_id":"target-kernel","target_machine_id":"target-machine","warning_seen":false});
    let typed: crate::account_profile::ProviderAccountCopyMetadata =
        serde_json::from_value(wire.clone()).unwrap();
    let encoded = serde_json::to_value(typed).unwrap();
    assert_eq!(encoded, wire);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&encoded).unwrap())
        ),
        "3fae9042e0a37db54e1360ee99fe897c9234e4d21fcecb966ee3e18f93939892"
    );
}

#[test]
fn managed_slice_copy_import_455_peer_74_snapshot_and_hash() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 455);
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        74
    );
    let wire = serde_json::json!({"kind":"import_managed_slice_provider_account_copy","slice_id":"slice-1","materialization":{"copy_source":{"machine_id":"source-machine","kernel_id":"source-kernel"},"profile":{"owner_user_id":"owner","provider":"codex","profile_id":"source-account","label":"Source","origin":"chariox_created","is_default":false},"files":[],"generated_at_ms":1}});
    let typed: crate::transport::relay_peer::RelayPeerRequest =
        serde_json::from_value(wire.clone()).unwrap();
    let encoded = serde_json::to_value(typed).unwrap();
    assert_eq!(encoded, wire);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&encoded).unwrap())
        ),
        "17d6cb97d8781c8af016507cb07b365818b8806f3e84b7a87c21880d6d1d2603"
    );
}
