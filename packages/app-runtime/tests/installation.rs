use chariox_app_runtime::installation::{
    ActiveGeneration, CapabilityApproval, CapabilityDecision, InstallationError,
    InstallationRegistry, ReleaseMetadata, StageToken, UpdatePhase, RETAINED_UPDATE_RECORDS,
};
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DATABASE: AtomicU64 = AtomicU64::new(0);

struct Database(PathBuf);

impl Database {
    fn new() -> Self {
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "chariox-app-installation-{}-{time}-{}",
            std::process::id(),
            NEXT_DATABASE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn open(&self) -> Connection {
        let mut connection = Connection::open(self.0.join("kernel.sqlite")).unwrap();
        connection
            .execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
            .unwrap();
        InstallationRegistry::new(&mut connection)
            .initialize()
            .unwrap();
        connection
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn release(version: u32) -> ReleaseMetadata {
    ReleaseMetadata {
        app_id: "com.chariox.todo".into(),
        version: format!("{version}.0.0"),
        publisher_id: "developer-public-key".into(),
        package_digest: format!("sha256:{version:064x}"),
        schema_version: version,
        capabilities_digest: format!("sha256:{:064x}", version + 10),
        catalog_digest: format!("sha256:{:064x}", version + 20),
        view_digest: format!("sha256:{:064x}", version + 30),
    }
}

fn approved() -> CapabilityDecision {
    CapabilityDecision::Approved {
        approval: CapabilityApproval {
            decision_id: "kernel-decision-1".into(),
            authority_ref: "human:owner".into(),
        },
    }
}

fn initialize(registry: &mut InstallationRegistry<'_>) {
    registry
        .create("todo", "com.chariox.todo", "owner")
        .unwrap();
}

fn prepare(registry: &mut InstallationRegistry<'_>, token: &StageToken) {
    registry.decide(token, approved(), 2).unwrap();
    registry.quiesce(token, 3).unwrap();
    registry.mark_prepared(token, 4).unwrap();
}

fn first_install(registry: &mut InstallationRegistry<'_>) -> ActiveGeneration {
    initialize(registry);
    let token = registry.stage("todo", 0, release(1), 1).unwrap().token;
    prepare(registry, &token);
    registry.commit(&token, 5).unwrap()
}

#[test]
fn stage_approval_and_preparation_recover_after_reopen() {
    let database = Database::new();
    let mut connection = database.open();
    let mut registry = InstallationRegistry::new(&mut connection);
    let old = first_install(&mut registry);
    let stage = registry
        .stage("todo", old.generation, release(2), 6)
        .unwrap();
    assert_eq!(
        registry.require_active("todo", old.generation).unwrap(),
        old
    );
    drop(connection);

    let mut connection = database.open();
    let mut registry = InstallationRegistry::new(&mut connection);
    assert_eq!(registry.journal("todo").unwrap()[0], stage);
    registry.decide(&stage.token, approved(), 7).unwrap();
    registry.quiesce(&stage.token, 8).unwrap();
    assert!(matches!(
        registry.require_active("todo", old.generation),
        Err(InstallationError::AdmissionPaused)
    ));
    drop(connection);

    let mut connection = database.open();
    let mut registry = InstallationRegistry::new(&mut connection);
    assert!(registry.get("todo").unwrap().admission_paused);
    assert_eq!(
        registry.journal("todo").unwrap()[0].phase,
        UpdatePhase::Quiescing
    );
    registry.mark_prepared(&stage.token, 9).unwrap();
    let current = registry.commit(&stage.token, 10).unwrap();
    assert_eq!(current.release, release(2));
    assert_eq!(current.generation, stage.token.generation);
    assert_eq!(
        registry.require_active("todo", current.generation).unwrap(),
        current
    );
    assert!(matches!(
        registry.require_active("todo", old.generation),
        Err(InstallationError::Conflict)
    ));
    drop(connection);

    let mut connection = database.open();
    let registry = InstallationRegistry::new(&mut connection);
    assert_eq!(registry.get("todo").unwrap().active, Some(current));
    assert_eq!(
        registry.journal("todo").unwrap()[0].phase,
        UpdatePhase::Committed
    );
}

#[test]
fn database_failure_between_active_switch_and_journal_commit_rolls_back_everything() {
    let database = Database::new();
    let mut connection = database.open();
    let mut registry = InstallationRegistry::new(&mut connection);
    let old = first_install(&mut registry);
    let token = registry
        .stage("todo", old.generation, release(2), 6)
        .unwrap()
        .token;
    prepare(&mut registry, &token);
    connection
        .execute_batch(
            "CREATE TRIGGER fail_commit BEFORE UPDATE ON app_installation_updates
         WHEN NEW.phase = 'committed' BEGIN
           SELECT RAISE(ABORT, 'simulated storage failure after active switch');
         END;",
        )
        .unwrap();
    let mut registry = InstallationRegistry::new(&mut connection);
    assert!(matches!(
        registry.commit(&token, 10),
        Err(InstallationError::Database(_))
    ));
    drop(connection);

    let mut connection = database.open();
    let registry = InstallationRegistry::new(&mut connection);
    let recovered = registry.get("todo").unwrap();
    assert_eq!(recovered.active, Some(old));
    assert_eq!(recovered.pending_generation, Some(token.generation));
    assert!(recovered.admission_paused);
    assert_eq!(
        registry.journal("todo").unwrap()[0].phase,
        UpdatePhase::Prepared
    );
    connection
        .execute_batch("DROP TRIGGER fail_commit;")
        .unwrap();
    let mut registry = InstallationRegistry::new(&mut connection);
    assert_eq!(registry.commit(&token, 11).unwrap().release, release(2));
}

#[test]
fn interrupted_stage_has_no_half_journal_or_allocated_generation() {
    let database = Database::new();
    let mut connection = database.open();
    initialize(&mut InstallationRegistry::new(&mut connection));
    connection
        .execute_batch(
            "CREATE TRIGGER fail_stage BEFORE UPDATE ON app_installations
         WHEN NEW.pending_generation IS NOT NULL BEGIN
           SELECT RAISE(ABORT, 'simulated staging write failure');
         END;",
        )
        .unwrap();
    let mut registry = InstallationRegistry::new(&mut connection);
    assert!(matches!(
        registry.stage("todo", 0, release(1), 1),
        Err(InstallationError::Database(_))
    ));
    drop(connection);

    let mut connection = database.open();
    let registry = InstallationRegistry::new(&mut connection);
    assert!(registry.journal("todo").unwrap().is_empty());
    assert_eq!(registry.get("todo").unwrap().pending_generation, None);
    connection
        .execute_batch("DROP TRIGGER fail_stage;")
        .unwrap();
    assert_eq!(
        InstallationRegistry::new(&mut connection)
            .stage("todo", 0, release(1), 2)
            .unwrap()
            .token
            .generation,
        1
    );
}

#[test]
fn every_precommit_abort_preserves_the_active_release_and_never_reuses_tokens() {
    let database = Database::new();
    let mut connection = database.open();
    let mut registry = InstallationRegistry::new(&mut connection);
    let old = first_install(&mut registry);
    let mut previous_candidate = old.generation;
    for phase in [
        UpdatePhase::Staged,
        UpdatePhase::Quiescing,
        UpdatePhase::Prepared,
    ] {
        let token = registry
            .stage("todo", old.generation, release(2), 6)
            .unwrap()
            .token;
        assert!(token.generation > previous_candidate);
        registry.decide(&token, approved(), 7).unwrap();
        if phase != UpdatePhase::Staged {
            registry.quiesce(&token, 8).unwrap();
        }
        if phase == UpdatePhase::Prepared {
            registry.mark_prepared(&token, 9).unwrap();
        }
        assert_eq!(
            registry
                .abort(&token, "staged health check failed", 10)
                .unwrap()
                .phase,
            UpdatePhase::Aborted
        );
        assert_eq!(
            registry.require_active("todo", old.generation).unwrap(),
            old
        );
        assert!(matches!(
            registry.commit(&token, 11),
            Err(InstallationError::InvalidTransition)
        ));
        previous_candidate = token.generation;
    }
}

#[test]
fn declining_install_or_capability_expansion_keeps_existing_authority_unchanged() {
    let database = Database::new();
    let mut connection = database.open();
    let mut registry = InstallationRegistry::new(&mut connection);
    initialize(&mut registry);
    let declined = CapabilityDecision::Declined {
        decision_id: "decline-1".into(),
        authority_ref: "human:owner".into(),
    };
    let initial = registry.stage("todo", 0, release(1), 1).unwrap();
    registry
        .decide(&initial.token, declined.clone(), 2)
        .unwrap();
    assert!(matches!(
        registry.require_active("todo", 0),
        Err(InstallationError::Inactive)
    ));
    let retry = registry.stage("todo", 0, release(1), 3).unwrap();
    assert!(retry.token.generation > initial.token.generation);
    prepare(&mut registry, &retry.token);
    let old = registry.commit(&retry.token, 5).unwrap();
    let expansion = registry
        .stage("todo", old.generation, release(2), 6)
        .unwrap();
    let record = registry.decide(&expansion.token, declined, 7).unwrap();
    assert_eq!(record.phase, UpdatePhase::Aborted);
    assert_eq!(
        registry.require_active("todo", old.generation).unwrap(),
        old
    );
}

#[test]
fn independent_connections_cannot_stage_or_commit_using_stale_generation() {
    let database = Database::new();
    let mut first = database.open();
    let mut second = database.open();
    let mut first_registry = InstallationRegistry::new(&mut first);
    initialize(&mut first_registry);
    let mut second_registry = InstallationRegistry::new(&mut second);
    let observed = second_registry.get("todo").unwrap().generation;
    let token = first_registry
        .stage("todo", 0, release(1), 1)
        .unwrap()
        .token;
    assert!(matches!(
        second_registry.stage("todo", observed, release(2), 2),
        Err(InstallationError::Conflict)
    ));
    prepare(&mut first_registry, &token);
    let active = first_registry.commit(&token, 5).unwrap();
    assert!(matches!(
        second_registry.stage("todo", observed, release(2), 6),
        Err(InstallationError::Conflict)
    ));
    assert!(matches!(
        second_registry.uninstall("todo", observed, 7),
        Err(InstallationError::Conflict)
    ));
    assert_eq!(
        second_registry
            .require_active("todo", active.generation)
            .unwrap(),
        active
    );
}

#[test]
fn approval_and_preparation_cannot_be_skipped_or_replaced_after_quiesce() {
    let database = Database::new();
    let mut connection = database.open();
    let mut registry = InstallationRegistry::new(&mut connection);
    initialize(&mut registry);
    let token = registry.stage("todo", 0, release(1), 1).unwrap().token;
    assert!(matches!(
        registry.quiesce(&token, 2),
        Err(InstallationError::ApprovalRequired)
    ));
    assert!(matches!(
        registry.commit(&token, 2),
        Err(InstallationError::InvalidTransition)
    ));
    assert!(matches!(
        registry.mark_prepared(&token, 2),
        Err(InstallationError::InvalidTransition)
    ));
    registry.decide(&token, approved(), 3).unwrap();
    registry.quiesce(&token, 4).unwrap();
    assert!(matches!(
        registry.decide(&token, approved(), 5),
        Err(InstallationError::InvalidTransition)
    ));
    assert!(matches!(
        registry.commit(&token, 5),
        Err(InstallationError::InvalidTransition)
    ));
    registry.mark_prepared(&token, 6).unwrap();
    registry.commit(&token, 7).unwrap();
}

#[test]
fn postcommit_abort_cannot_erase_newly_accepted_user_data() {
    let database = Database::new();
    let mut connection = database.open();
    let mut registry = InstallationRegistry::new(&mut connection);
    let old = first_install(&mut registry);
    let token = registry
        .stage("todo", old.generation, release(2), 6)
        .unwrap()
        .token;
    prepare(&mut registry, &token);
    let active = registry.commit(&token, 10).unwrap();
    connection.execute_batch("CREATE TABLE user_app_data(value TEXT); INSERT INTO user_app_data VALUES ('accepted new write');").unwrap();
    let mut registry = InstallationRegistry::new(&mut connection);
    assert!(matches!(
        registry.abort(&token, "late health failure", 11),
        Err(InstallationError::InvalidTransition)
    ));
    assert_eq!(
        registry.require_active("todo", active.generation).unwrap(),
        active
    );
    let retained: String = connection
        .query_row("SELECT value FROM user_app_data", [], |row| row.get(0))
        .unwrap();
    assert_eq!(retained, "accepted new write");
}

#[test]
fn uninstall_fences_pending_and_active_handles_without_deleting_user_assets() {
    let database = Database::new();
    let mut connection = database.open();
    connection.execute_batch("CREATE TABLE user_workflows(id TEXT); INSERT INTO user_workflows VALUES ('workflow-1'); CREATE TABLE user_agents(id TEXT); INSERT INTO user_agents VALUES ('agent-1');").unwrap();
    let mut registry = InstallationRegistry::new(&mut connection);
    let old = first_install(&mut registry);
    let token = registry
        .stage("todo", old.generation, release(2), 6)
        .unwrap()
        .token;
    registry.decide(&token, approved(), 7).unwrap();
    registry.quiesce(&token, 8).unwrap();
    let uninstalled = registry.uninstall("todo", old.generation, 9).unwrap();
    assert!(uninstalled.generation > token.generation);
    assert_eq!(uninstalled.active, None);
    assert_eq!(uninstalled.pending_generation, None);
    assert!(matches!(
        registry.require_active("todo", old.generation),
        Err(InstallationError::Conflict)
    ));
    assert!(matches!(
        registry.require_active("todo", uninstalled.generation),
        Err(InstallationError::Inactive)
    ));
    assert!(matches!(
        registry.mark_prepared(&token, 10),
        Err(InstallationError::InvalidTransition)
    ));
    let reinstall = registry
        .stage("todo", uninstalled.generation, release(2), 11)
        .unwrap();
    assert!(reinstall.token.generation > uninstalled.generation);
    let workflow: String = connection
        .query_row("SELECT id FROM user_workflows", [], |row| row.get(0))
        .unwrap();
    let agent: String = connection
        .query_row("SELECT id FROM user_agents", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        (workflow.as_str(), agent.as_str()),
        ("workflow-1", "agent-1")
    );
}

#[test]
fn retention_bounds_terminal_journal_without_discarding_active_or_pending_state() {
    let database = Database::new();
    let mut connection = database.open();
    let mut registry = InstallationRegistry::new(&mut connection);
    let old = first_install(&mut registry);
    for index in 0..RETAINED_UPDATE_RECORDS + 4 {
        let token = registry
            .stage("todo", old.generation, release(2), index as u64)
            .unwrap()
            .token;
        registry
            .abort(&token, "candidate cancelled", index as u64)
            .unwrap();
    }
    assert_eq!(
        registry.journal("todo").unwrap().len(),
        RETAINED_UPDATE_RECORDS
    );
    let pending = registry
        .stage("todo", old.generation, release(2), 100)
        .unwrap();
    let journal = registry.journal("todo").unwrap();
    assert_eq!(journal.len(), RETAINED_UPDATE_RECORDS + 1);
    assert_eq!(journal[0], pending);
    assert_eq!(
        registry.require_active("todo", old.generation).unwrap(),
        old
    );
}

#[test]
fn identities_digests_and_generation_tokens_are_checked_before_mutation() {
    let database = Database::new();
    let mut connection = database.open();
    let mut registry = InstallationRegistry::new(&mut connection);
    let old = first_install(&mut registry);
    assert!(matches!(
        registry.create("todo", "another-app", "intruder"),
        Err(InstallationError::Conflict)
    ));
    for invalid in [
        ReleaseMetadata {
            app_id: "different-app".into(),
            ..release(2)
        },
        ReleaseMetadata {
            publisher_id: "different-publisher".into(),
            ..release(2)
        },
        ReleaseMetadata {
            view_digest: "sha256:bad".into(),
            ..release(2)
        },
    ] {
        assert!(matches!(
            registry.stage("todo", old.generation, invalid, 6),
            Err(InstallationError::Invalid(_))
        ));
    }
    let token = registry
        .stage("todo", old.generation, release(2), 7)
        .unwrap()
        .token;
    let wrong_base = StageToken {
        base_generation: 0,
        ..token.clone()
    };
    assert!(matches!(
        registry.decide(&wrong_base, approved(), 8),
        Err(InstallationError::Conflict)
    ));
    assert_eq!(registry.get("todo").unwrap().owner_id, "owner");
    assert_eq!(
        registry.journal("todo").unwrap()[0].decision,
        CapabilityDecision::Pending
    );
}

#[test]
fn caller_generations_outside_sqlite_range_are_rejected_without_mutation() {
    let database = Database::new();
    let mut connection = database.open();
    let mut registry = InstallationRegistry::new(&mut connection);
    let old = first_install(&mut registry);
    let token = registry
        .stage("todo", old.generation, release(2), 6)
        .unwrap()
        .token;
    let oversized = u64::try_from(i64::MAX).unwrap() + 1;
    let before = registry.get("todo").unwrap();
    for invalid in [
        StageToken {
            generation: oversized,
            ..token.clone()
        },
        StageToken {
            base_generation: oversized,
            ..token.clone()
        },
    ] {
        assert!(matches!(
            registry.decide(&invalid, approved(), 7),
            Err(InstallationError::Invalid(_))
        ));
        assert!(matches!(
            registry.quiesce(&invalid, 7),
            Err(InstallationError::Invalid(_))
        ));
        assert!(matches!(
            registry.mark_prepared(&invalid, 7),
            Err(InstallationError::Invalid(_))
        ));
        assert!(matches!(
            registry.commit(&invalid, 7),
            Err(InstallationError::Invalid(_))
        ));
        assert!(matches!(
            registry.abort(&invalid, "cancelled", 7),
            Err(InstallationError::Invalid(_))
        ));
    }
    assert!(matches!(
        registry.stage("todo", oversized, release(2), 8),
        Err(InstallationError::Invalid(_))
    ));
    assert!(matches!(
        registry.uninstall("todo", oversized, 8),
        Err(InstallationError::Invalid(_))
    ));
    assert!(matches!(
        registry.require_active("todo", oversized),
        Err(InstallationError::Invalid(_))
    ));
    assert_eq!(registry.get("todo").unwrap(), before);
    assert_eq!(
        registry.journal("todo").unwrap()[0].decision,
        CapabilityDecision::Pending
    );
}

#[test]
fn largest_sqlite_generation_round_trips_and_exhaustion_never_wraps() {
    let database = Database::new();
    let mut connection = database.open();
    initialize(&mut InstallationRegistry::new(&mut connection));
    connection
        .execute(
            "UPDATE app_installations SET allocated_generation = ?1 WHERE installation_id = 'todo'",
            [i64::MAX - 1],
        )
        .unwrap();
    let mut registry = InstallationRegistry::new(&mut connection);
    let token = registry.stage("todo", 0, release(1), 1).unwrap().token;
    assert_eq!(token.generation, u64::try_from(i64::MAX).unwrap());
    prepare(&mut registry, &token);
    let active = registry.commit(&token, 5).unwrap();
    drop(connection);

    let mut connection = database.open();
    let mut registry = InstallationRegistry::new(&mut connection);
    assert_eq!(
        registry.require_active("todo", active.generation).unwrap(),
        active
    );
    assert!(matches!(
        registry.stage("todo", active.generation, release(2), 6),
        Err(InstallationError::Invalid("generation exhausted"))
    ));
    assert!(matches!(
        registry.uninstall("todo", active.generation, 6),
        Err(InstallationError::Invalid("generation exhausted"))
    ));
    assert_eq!(registry.journal("todo").unwrap().len(), 1);
    assert_eq!(registry.get("todo").unwrap().active, Some(active));
}

#[test]
fn counter_exhaustion_rolls_back_uninstall_of_a_pending_update() {
    let database = Database::new();
    let mut connection = database.open();
    let mut registry = InstallationRegistry::new(&mut connection);
    let old = first_install(&mut registry);
    let token = registry
        .stage("todo", old.generation, release(2), 6)
        .unwrap()
        .token;
    prepare(&mut registry, &token);
    connection
        .execute(
            "UPDATE app_installations SET allocated_generation = ?1 WHERE installation_id = 'todo'",
            [i64::MAX],
        )
        .unwrap();
    let mut registry = InstallationRegistry::new(&mut connection);
    let before = registry.get("todo").unwrap();
    assert!(matches!(
        registry.uninstall("todo", old.generation, 9),
        Err(InstallationError::Invalid("generation exhausted"))
    ));
    assert_eq!(registry.get("todo").unwrap(), before);
    assert_eq!(
        registry.journal("todo").unwrap()[0].phase,
        UpdatePhase::Prepared
    );
}

#[test]
fn negative_stored_generations_are_rejected_instead_of_wrapping() {
    let database = Database::new();
    let mut connection = database.open();
    initialize(&mut InstallationRegistry::new(&mut connection));
    connection
        .execute(
            "UPDATE app_installations SET pending_generation = -1 WHERE installation_id = 'todo'",
            [],
        )
        .unwrap();
    assert!(matches!(
        InstallationRegistry::new(&mut connection).get("todo"),
        Err(InstallationError::Invalid("negative stored generation"))
    ));
}

#[test]
fn initial_stage_failure_rolls_back_identity_and_generation() {
    let database = Database::new();
    let mut connection = database.open();
    connection
        .execute_batch(
            "CREATE TRIGGER fail_initial_stage BEFORE UPDATE OF pending_generation
         ON app_installations WHEN NEW.pending_generation IS NOT NULL BEGIN
            SELECT RAISE(ABORT, 'stage failed after journal insert');
         END;",
        )
        .unwrap();
    let mut registry = InstallationRegistry::new(&mut connection);
    assert!(matches!(
        registry.create_and_stage("todo", "owner", release(1), 1),
        Err(InstallationError::Database(_))
    ));
    assert!(matches!(
        registry.get("todo"),
        Err(InstallationError::NotFound)
    ));
    drop(connection);

    let mut connection = database.open();
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM app_installation_updates", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    connection
        .execute_batch("DROP TRIGGER fail_initial_stage")
        .unwrap();
    let mut registry = InstallationRegistry::new(&mut connection);
    let stage = registry
        .create_and_stage("todo", "owner", release(1), 2)
        .unwrap();
    assert_eq!(stage.token.generation, 1);
    assert!(matches!(
        registry.create_and_stage("todo", "other", release(2), 3),
        Err(InstallationError::Conflict)
    ));
    assert_eq!(registry.get("todo").unwrap().owner_id, "owner");
    assert_eq!(registry.journal("todo").unwrap(), vec![stage]);
}

#[test]
fn owner_pages_are_bounded_stable_and_work_on_read_only_connection() {
    let database = Database::new();
    let mut connection = database.open();
    let mut registry = InstallationRegistry::new(&mut connection);
    for (id, owner) in [
        ("a", "owner"),
        ("b", "other"),
        ("c", "owner"),
        ("d", "owner"),
    ] {
        registry.create_and_stage(id, owner, release(1), 1).unwrap();
    }
    registry.uninstall("a", 0, 2).unwrap();
    connection.pragma_update(None, "query_only", true).unwrap();
    let registry = InstallationRegistry::new(&mut connection);
    let first = registry.list("owner", None, 2).unwrap();
    assert_eq!(
        first
            .installations
            .iter()
            .map(|item| item.installation_id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "c"]
    );
    assert_eq!(first.next_cursor.as_deref(), Some("c"));
    assert!(first.installations[0].active.is_none());
    let second = registry
        .list("owner", first.next_cursor.as_deref(), 2)
        .unwrap();
    assert_eq!(second.installations.len(), 1);
    assert_eq!(second.installations[0].installation_id, "d");
    assert_eq!(second.next_cursor, None);
    assert_eq!(
        registry.list("other", None, 2).unwrap().installations.len(),
        1
    );
    assert!(registry
        .list("unknown", None, 2)
        .unwrap()
        .installations
        .is_empty());
    for limit in [0, 101, usize::MAX] {
        assert!(matches!(
            registry.list("owner", None, limit),
            Err(InstallationError::Invalid(_))
        ));
    }
    assert!(registry.list("", None, 2).is_err());
    assert!(registry.list("owner", Some("\n"), 2).is_err());
}
