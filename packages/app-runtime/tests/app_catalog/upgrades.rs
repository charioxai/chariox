use super::*;
use chariox_app_runtime::managed_state::{ManagedStateStore, StateChanges, StateScope, StateWrite};
use sha2::{Digest, Sha256};

// MP-08 / MP-10 / MP-11: an unchanged archive must retain its pre-D1 release identity.
#[test]
fn persisted_v2_capability_digest_keeps_installed_catalog_current_after_upgrade() {
    let package = Package::new();
    let root = std::env::temp_dir().join(format!(
        "chariox-catalog-upgrade-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("kernel.sqlite");
    let mut connection = Connection::open(&path).unwrap();
    InstallationRegistry::new(&mut connection)
        .initialize()
        .unwrap();
    let mut publishers = PublisherTrustRegistry::new(&mut connection);
    publishers.initialize().unwrap();
    let publisher = package.publisher();
    publishers
        .enroll("owner", &publisher, 0, &decision("enroll"), 1)
        .unwrap();
    let trust = publishers
        .trusted_publisher("owner", &publisher.publisher_id, &publisher.key_id)
        .unwrap();
    let token = install(&mut connection, "installed", &package, &trust);
    let scope = StateScope::new("owner", "installed", token.generation).unwrap();
    let mut state = ManagedStateStore::new(&mut connection);
    state.initialize().unwrap();
    state
        .transaction(
            scope,
            &StateChanges::new(
                0,
                vec![],
                vec![StateWrite::Put {
                    key: "document".into(),
                    value: json!({"text":"user work"}),
                }],
            )
            .unwrap(),
        )
        .unwrap();
    let data = state.get(scope, "document").unwrap();
    let bytes = package.bytes();
    let verified = verify(
        &bytes,
        &VerificationPolicy::new(500, vec![package.publisher()]),
    )
    .unwrap();
    let declarations = verified.declarations();
    // The fingerprint used by releases before D1, independent of candidate construction.
    let legacy = json!({
        "schema": "chariox.installation-capabilities.v2",
        "capabilities": verified.manifest().capabilities,
        "actions": declarations.actions,
        "toolActions": declarations.tools.iter().filter_map(|tool| tool.action.as_ref().map(|action|
            json!({"tool": tool.name, "action": action}))).collect::<Vec<_>>()
    });
    let legacy_digest = format!(
        "sha256:{:x}",
        Sha256::digest(serde_json_canonicalizer::to_vec(&legacy).unwrap())
    );
    let mut before = InstallationRegistry::new(&mut connection)
        .get("installed")
        .unwrap();
    before.active.as_mut().unwrap().release.capabilities_digest = legacy_digest;
    connection
        .execute(
            "UPDATE app_installations SET active_json=?1 WHERE installation_id='installed'",
            [serde_json::to_string(before.active.as_ref().unwrap()).unwrap()],
        )
        .unwrap();
    connection
        .execute_batch("DROP TABLE app_installation_authority; DROP TABLE app_release_versions;")
        .unwrap();

    drop(connection);
    let mut connection = Connection::open(&path).unwrap();
    InstallationRegistry::new(&mut connection)
        .initialize()
        .unwrap();
    assert_eq!(
        InstallationRegistry::new(&mut connection)
            .get("installed")
            .unwrap(),
        before
    );
    assert_eq!(
        ManagedStateStore::new(&mut connection)
            .get(scope, "document")
            .unwrap(),
        data
    );
    let catalog = package.catalog(&mut connection, &token, &trust).unwrap();
    let transaction = connection.transaction().unwrap();
    catalog.require_current(&transaction, "owner").unwrap();
    assert_eq!(catalog.tools().count(), 1);
    transaction.commit().unwrap();
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}
