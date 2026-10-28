use super::*;

fn stage_legacy(
    connection: &mut Connection,
    package: &Package,
    trust: &TrustedPublisherSnapshot,
) -> StageToken {
    let token = Installations::new(connection)
        .create_and_stage_verified("installation", "alice", &package.candidate(trust), 1)
        .unwrap()
        .token;
    activate(connection, &token);
    token
}

fn update(connection: &mut Connection, package: &Package, trust: &TrustedPublisherSnapshot) {
    let candidate = package.candidate(trust);
    let generation = Installations::new(connection)
        .get("installation")
        .unwrap()
        .generation;
    let tx = connection.transaction().unwrap();
    let record = candidate
        .stage_update_in(&tx, "alice", "installation", generation, 10)
        .unwrap();
    let binding = chariox_app_runtime::installation::StageTrustBinding::staged_in(
        &tx,
        "alice",
        &record.token,
    )
    .unwrap();
    binding
        .decide_in(
            &tx,
            "alice",
            trust,
            CapabilityDecision::Approved {
                approval: CapabilityApproval {
                    decision_id: "approved".into(),
                    authority_ref: "kernel-confirmation".into(),
                },
            },
            11,
        )
        .unwrap();
    let approval = binding.quiesce_in(&tx, "alice", trust, 12).unwrap();
    binding
        .commit_in(&tx, "alice", trust, &approval, 13)
        .unwrap();
    tx.commit().unwrap();
}

fn refuse_changed_version(
    connection: &mut Connection,
    package: &mut Package,
    trust: &TrustedPublisherSnapshot,
) {
    package.manifest.version = "1.0.0".into();
    package
        .files
        .insert("ui/index.html".into(), b"changed v1 bytes".to_vec());
    let candidate = package.candidate(trust);
    let generation = Installations::new(connection)
        .get("installation")
        .unwrap()
        .generation;
    let tx = connection.transaction().unwrap();
    assert!(matches!(
        candidate.stage_update_in(&tx, "alice", "installation", generation, 20),
        Err(Error::Installation(InstallationError::Invalid(
            "immutable release version"
        )))
    ));
}

// MP-08 / MP-10 / MP-11: v1 predates the ledger; committing v2 must not free v1's identity.
#[test]
fn migrated_active_release_stays_immutable_after_registering_v2() {
    for existing_ledger in [false, true] {
        let database = Database::new();
        let mut connection = database.open();
        let mut package = Package::new();
        let trust = enroll(&mut connection, "alice", &package, 0, "enroll");
        stage_legacy(&mut connection, &package, &trust);
        if existing_ledger {
            connection
                .execute_batch("DELETE FROM app_release_versions;")
                .unwrap();
        } else {
            connection.execute_batch("DROP TABLE app_release_versions; ALTER TABLE app_installations DROP COLUMN dev_room_id; ALTER TABLE app_installations DROP COLUMN channel;").unwrap();
        }
        drop(connection);
        let mut connection = database.open();
        package.manifest.version = "2.0.0".into();
        update(&mut connection, &package, &trust);
        refuse_changed_version(&mut connection, &mut package, &trust);
    }
}

#[test]
fn migration_seeds_retained_releases_without_update_history() {
    let database = Database::new();
    let mut connection = database.open();
    let package = Package::new();
    let trust = enroll(&mut connection, "alice", &package, 0, "enroll");
    stage_legacy(&mut connection, &package, &trust);
    Installations::new(&mut connection)
        .uninstall("installation", 1, 8)
        .unwrap();
    let retained = Installations::new(&mut connection)
        .get("installation")
        .unwrap();
    connection
        .execute_batch("DROP TABLE app_release_versions; DELETE FROM app_installation_updates;")
        .unwrap();
    Installations::new(&mut connection).initialize().unwrap();
    let digest: String = connection.query_row("SELECT package_digest FROM app_release_versions WHERE owner_id='alice' AND version='1.0.0'", [], |r| r.get(0)).unwrap();
    assert_eq!(
        digest,
        retained.retained.as_ref().unwrap().release.package_digest
    );
    assert_eq!(
        Installations::new(&mut connection)
            .get("installation")
            .unwrap(),
        retained
    );
    Installations::new(&mut connection).initialize().unwrap();
}

#[test]
fn migration_seeds_committed_history_but_not_aborted_candidates_or_dev() {
    let database = Database::new();
    let mut connection = database.open();
    let mut package = Package::new();
    let trust = enroll(&mut connection, "alice", &package, 0, "enroll");
    stage_legacy(&mut connection, &package, &trust);
    package.manifest.version = "2.0.0".into();
    update(&mut connection, &package, &trust);
    package.manifest.version = "3.0.0".into();
    let pending = Installations::new(&mut connection)
        .stage_verified("installation", "alice", 2, &package.candidate(&trust), 15)
        .unwrap();
    Installations::new(&mut connection)
        .abort(&pending.token, "test abort", 16)
        .unwrap();
    // Dev builds may legitimately reuse a version with different bytes.
    connection.execute_batch("INSERT INTO app_installations(installation_id,app_id,owner_id,channel,dev_room_id,active_json) SELECT 'dev',app_id,owner_id,'dev','room',json_set(active_json,'$.release.publisher_id','local_dev') FROM app_installations WHERE installation_id='installation'; DROP TABLE app_release_versions;").unwrap();
    Installations::new(&mut connection).initialize().unwrap();
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM app_release_versions", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 2);
    refuse_changed_version(&mut connection, &mut package, &trust);
}

// MP-08 / MP-10 / MP-11: pre-D1 allowed version reuse; preserve those Apps on upgrade.
#[test]
fn migration_pins_current_bytes_when_legacy_history_reused_a_version() {
    let database = Database::new();
    let mut connection = database.open();
    let mut package = Package::new();
    let trust = enroll(&mut connection, "alice", &package, 0, "enroll");
    stage_legacy(&mut connection, &package, &trust);
    package
        .files
        .insert("ui/index.html".into(), b"legacy v1 revision".to_vec());
    let token = Installations::new(&mut connection)
        .stage_verified("installation", "alice", 1, &package.candidate(&trust), 10)
        .unwrap()
        .token;
    activate(&mut connection, &token);
    let current = Installations::new(&mut connection)
        .get("installation")
        .unwrap();
    connection
        .execute_batch("DROP TABLE app_release_versions;")
        .unwrap();
    Installations::new(&mut connection).initialize().unwrap();
    let digest: String = connection
        .query_row(
            "SELECT package_digest FROM app_release_versions WHERE version='1.0.0'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        digest,
        current.active.as_ref().unwrap().release.package_digest
    );
    assert_eq!(
        Installations::new(&mut connection)
            .get("installation")
            .unwrap(),
        current
    );
    package.manifest.version = "2.0.0".into();
    update(&mut connection, &package, &trust);
    refuse_changed_version(&mut connection, &mut package, &trust);
}
