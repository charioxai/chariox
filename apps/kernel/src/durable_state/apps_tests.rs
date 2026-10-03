use super::apps::{AppRegistryError, AppRegistryMutation, AppRegistryOutcome, AppRegistryRequest};
use super::*;
use chariox_app_runtime::installation::{
    CapabilityApproval, CapabilityDecision, InstallationError, StageToken, UpdatePhase,
};

struct Database(PathBuf);

impl Database {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-durable-apps-{}-{}",
            std::process::id(),
            rand_suffix(),
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn open(&self) -> DurableKernelStateStore {
        DurableKernelStateStore::open_owned(self.0.join("kernel.db")).unwrap()
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn release() -> chariox_app_runtime::installation::ReleaseMetadata {
    chariox_app_runtime::installation::ReleaseMetadata {
        app_id: "com.chariox.todo".into(),
        version: "1.0.0".into(),
        publisher_id: "publisher".into(),
        package_digest: format!("sha256:{:064x}", 1),
        schema_version: 1,
        capabilities_digest: format!("sha256:{:064x}", 2),
        catalog_digest: format!("sha256:{:064x}", 3),
        view_digest: format!("sha256:{:064x}", 4),
    }
}

fn create(id: &str) -> AppRegistryMutation {
    AppRegistryMutation::CreateAndStage {
        installation_id: id.into(),
        release: release(),
        now_ms: 1,
    }
}

fn approval() -> CapabilityDecision {
    CapabilityDecision::Approved {
        approval: CapabilityApproval {
            decision_id: "decision".into(),
            authority_ref: "kernel-policy".into(),
        },
    }
}

fn staged_token(outcome: AppRegistryOutcome) -> StageToken {
    match outcome {
        AppRegistryOutcome::Update(update) => update.token,
        _ => panic!("expected staged update"),
    }
}

#[test]
fn uninstall_then_reinstall_without_expiry_cannot_restore_file_grants() {
    use super::app_file_grants::{FileGrantCommand, FilePick, GrantedFile, PickState};

    let database = Database::new();
    let store = database.open();
    let activate = |token: StageToken, now_ms| {
        for mutation in [
            AppRegistryMutation::Decide {
                token: token.clone(),
                decision: approval(),
                now_ms,
            },
            AppRegistryMutation::Quiesce {
                token: token.clone(),
                now_ms,
            },
            AppRegistryMutation::MarkPrepared {
                token: token.clone(),
                now_ms,
            },
            AppRegistryMutation::Commit { token, now_ms },
        ] {
            store.mutate_app_installation("owner", mutation).unwrap();
        }
    };
    let token = staged_token(
        store
            .mutate_app_installation("owner", create("todo"))
            .unwrap(),
    );
    let generation = token.generation;
    activate(token, 5);
    for id in ["granted", "pending"] {
        store
            .app_file_grant(FileGrantCommand::Create(FilePick {
                operation_id: id.into(),
                owner: "owner".into(),
                installation: "todo".into(),
                generation,
                accept: vec![],
                multiple: false,
                state: PickState::Pending,
                expires_ms: 1_000,
                grants: vec![],
            }))
            .unwrap();
    }
    let grant = store
        .app_file_grant(FileGrantCommand::Grant {
            owner: "owner".into(),
            operation_id: "granted".into(),
            files: vec![GrantedFile {
                name: "notes.md".into(),
                contents: b"private notes".to_vec(),
            }],
            now_ms: 10,
        })
        .unwrap()
        .unwrap()
        .grants[0]
        .clone();
    store
        .mutate_app_installation(
            "owner",
            AppRegistryMutation::Uninstall {
                installation_id: "todo".into(),
                expected_generation: generation,
                now_ms: 20,
            },
        )
        .unwrap();
    let uninstalled = store.get_app_installation("owner", "todo").unwrap();
    assert!(uninstalled.retained.is_some());
    // Reinstall the same installation without an Expire pass in between.
    let token = staged_token(
        store
            .mutate_app_installation(
                "owner",
                AppRegistryMutation::Stage {
                    installation_id: "todo".into(),
                    expected_generation: uninstalled.generation,
                    release: release(),
                    now_ms: 25,
                },
            )
            .unwrap(),
    );
    activate(token, 30);
    store
        .app_file_grant(FileGrantCommand::Expire { now_ms: 40 })
        .unwrap();
    assert_eq!(
        store.claim_app_file_grant(FileGrantCommand::Claim {
            owner: "owner".into(),
            installation: "todo".into(),
            grant_id: grant,
            now_ms: 41,
        }),
        Err("NOT_FOUND")
    );
    for id in ["granted", "pending"] {
        assert_eq!(
            store
                .app_file_pick("owner", "todo", id)
                .unwrap()
                .unwrap()
                .state,
            PickState::Expired
        );
    }
    let contents: Option<Vec<u8>> = rusqlite::Connection::open(store.path())
        .unwrap()
        .query_row(
            "SELECT contents FROM app_file_grants WHERE operation_id='granted'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(contents.is_none());
}

#[test]
fn app_writer_uses_existing_authority_and_recovers_staged_and_active_state() {
    let database = Database::new();
    let store = database.open();
    let read_guard = store.connection.lock().unwrap();
    assert!(read_guard
        .execute("DELETE FROM app_installations", [])
        .is_err());
    // Completion while holding the reader mutex proves writes do not use it.
    let token = staged_token(
        store
            .mutate_app_installation("owner", create("todo"))
            .unwrap(),
    );
    drop(read_guard);
    store
        .append_event("app.test", None, serde_json::json!({"ok": true}))
        .unwrap();
    drop(store);

    let store = database.open();
    let staged = store.get_app_installation("owner", "todo").unwrap();
    assert_eq!(staged.pending_generation, Some(token.generation));
    assert!(staged.active.is_none());
    assert_eq!(
        store.app_installation_journal("owner", "todo").unwrap()[0].phase,
        UpdatePhase::Staged
    );
    for mutation in [
        AppRegistryMutation::Decide {
            token: token.clone(),
            decision: approval(),
            now_ms: 2,
        },
        AppRegistryMutation::Quiesce {
            token: token.clone(),
            now_ms: 3,
        },
        AppRegistryMutation::MarkPrepared {
            token: token.clone(),
            now_ms: 4,
        },
        AppRegistryMutation::Commit {
            token: token.clone(),
            now_ms: 5,
        },
    ] {
        store.mutate_app_installation("owner", mutation).unwrap();
    }
    drop(store);
    let store = database.open();
    let active = store.get_app_installation("owner", "todo").unwrap();
    assert_eq!(active.active.unwrap().generation, token.generation);
    assert!(active.pending_generation.is_none());
    assert_eq!(store.load_events_after(0).unwrap().len(), 1);
}

#[test]
fn app_owner_filter_applies_to_every_mutation_and_read() {
    let database = Database::new();
    let store = database.open();
    let token = staged_token(
        store
            .mutate_app_installation("owner", create("todo"))
            .unwrap(),
    );
    store
        .mutate_app_installation("other", create("other-app"))
        .unwrap();
    let before = store.get_app_installation("owner", "todo").unwrap();
    for mutation in [
        create("todo"),
        AppRegistryMutation::Stage {
            installation_id: "todo".into(),
            expected_generation: 0,
            release: release(),
            now_ms: 2,
        },
        AppRegistryMutation::Decide {
            token: token.clone(),
            decision: approval(),
            now_ms: 2,
        },
        AppRegistryMutation::Quiesce {
            token: token.clone(),
            now_ms: 2,
        },
        AppRegistryMutation::MarkPrepared {
            token: token.clone(),
            now_ms: 2,
        },
        AppRegistryMutation::Commit {
            token: token.clone(),
            now_ms: 2,
        },
        AppRegistryMutation::Abort {
            token: token.clone(),
            reason: "cancel".into(),
            now_ms: 2,
        },
        AppRegistryMutation::Uninstall {
            installation_id: "todo".into(),
            expected_generation: 0,
            now_ms: 2,
        },
    ] {
        assert!(matches!(
            store.mutate_app_installation("other", mutation),
            Err(AppRegistryError::Registry(InstallationError::NotFound))
        ));
    }
    assert!(matches!(
        store.get_app_installation("other", "todo"),
        Err(AppRegistryError::Registry(InstallationError::NotFound))
    ));
    assert!(matches!(
        store.app_installation_journal("other", "todo"),
        Err(AppRegistryError::Registry(InstallationError::NotFound))
    ));
    assert_eq!(store.get_app_installation("owner", "todo").unwrap(), before);
    let page = store.list_app_installations("owner", None, 1).unwrap();
    assert_eq!(page.installations, vec![before]);
    assert!(page.next_cursor.is_none());
    assert!(store.list_app_installations("owner", None, 101).is_err());
    assert!(store
        .mutate_app_installation("", create("invalid-owner"))
        .is_err());
    assert!(store.get_app_installation("", "todo").is_err());
    assert!(store.app_installation_journal("", "todo").is_err());
    assert!(store.list_app_installations("", None, 1).is_err());
}

/// Protocol 367: the App list pages past deployment copies, so a page is
/// short only at the end.
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
#[test]
fn the_app_list_excludes_deployment_copies_in_its_page_query() {
    let database = Database::new();
    let store = database.open();
    for id in ["a", "b", "c", "d"] {
        store.mutate_app_installation("owner", create(id)).unwrap();
    }
    store.fixture_tag_app_installation("owner", "a", "deployment");
    store.fixture_tag_app_installation("owner", "c", "deployment");
    let ids = |page: &chariox_app_runtime::installation::InstallationPage| {
        page.installations
            .iter()
            .map(|installation| installation.installation_id.clone())
            .collect::<Vec<_>>()
    };
    let first = store
        .list_app_installations_without_copies("owner", None, 1)
        .unwrap();
    assert_eq!(ids(&first), ["b"]);
    assert_eq!(first.next_cursor.as_deref(), Some("b"));
    let last = store
        .list_app_installations_without_copies("owner", Some("b"), 1)
        .unwrap();
    assert_eq!(ids(&last), ["d"]);
    assert!(last.next_cursor.is_none());
    assert!(store
        .list_app_installations_without_copies("", None, 1)
        .is_err());
}

fn enqueue_event(
    sender: &SyncSender<DurableWriterRequest>,
    id: &str,
) -> Receiver<Result<u64, String>> {
    let (response, receiver) = mpsc::channel();
    sender
        .send(DurableWriterRequest::Ordinary(DurableWriteRequest {
            operation: DurableWriteOperation::Event {
                event_id: id.into(),
                kind: "app.test".into(),
                subject_id: None,
                timestamp_ms: 1,
                payload_json: "{}".into(),
            },
            response,
        }))
        .unwrap();
    receiver
}

fn enqueue_app(
    sender: &SyncSender<DurableWriterRequest>,
    mutation: AppRegistryMutation,
) -> Receiver<Result<AppRegistryOutcome, InstallationError>> {
    let (response, receiver) = mpsc::channel();
    sender
        .send(DurableWriterRequest::App(Box::new(AppRegistryRequest {
            owner_id: "owner".into(),
            mutation,
            response,
        })))
        .unwrap();
    receiver
}

fn writer_connection() -> Connection {
    let mut connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(DURABLE_STATE_SCHEMA).unwrap();
    apps::initialize(&mut connection).unwrap();
    connection
}

fn run_preloaded_writer(
    connection: Connection,
    receiver: Receiver<DurableWriterRequest>,
    health: Arc<DurableWriterHealth>,
) {
    // Each caller preloads and closes the queue: no timed wait occurs. Give
    // batching a long deadline so scheduler pauses cannot split a fault batch.
    run_durable_writer(connection, receiver, health, Duration::from_secs(60));
}

#[test]
fn app_barriers_preserve_fifo_and_batching_while_isolating_cas_failure() {
    let connection = writer_connection();
    // TEMP triggers are local to this connection, so they also prove the App
    // operation ran on the same connection as ordinary durable writes.
    connection.execute_batch(
        "CREATE TEMP TRIGGER app_requires_prior_events BEFORE INSERT ON app_installations
         WHEN (SELECT COUNT(*) FROM durable_state_events) != 2 BEGIN
            SELECT RAISE(ABORT, 'App overtook preceding events');
         END;
         CREATE TEMP TRIGGER event_requires_app BEFORE INSERT ON durable_state_events
         WHEN NEW.event_id IN ('after', 'last') AND NOT EXISTS (
            SELECT 1 FROM app_installations WHERE installation_id = 'todo' AND pending_generation = 1
         ) BEGIN SELECT RAISE(ABORT, 'event overtook App stage'); END;",
    ).unwrap();
    let (sender, receiver) = mpsc::sync_channel(16);
    let before = enqueue_event(&sender, "before");
    let before_two = enqueue_event(&sender, "before-two");
    let stage = enqueue_app(&sender, create("todo"));
    let after = enqueue_event(&sender, "after");
    let conflict = enqueue_app(
        &sender,
        AppRegistryMutation::Stage {
            installation_id: "todo".into(),
            expected_generation: 9,
            release: release(),
            now_ms: 2,
        },
    );
    let last = enqueue_event(&sender, "last");
    drop(sender);
    let health = Arc::new(DurableWriterHealth::default());
    run_preloaded_writer(connection, receiver, health.clone());
    assert_eq!(before.recv().unwrap().unwrap(), 1);
    assert_eq!(before_two.recv().unwrap().unwrap(), 2);
    assert_eq!(staged_token(stage.recv().unwrap().unwrap()).generation, 1);
    assert_eq!(after.recv().unwrap().unwrap(), 3);
    assert!(matches!(
        conflict.recv().unwrap(),
        Err(InstallationError::Conflict)
    ));
    assert_eq!(last.recv().unwrap().unwrap(), 4);
    assert_eq!(health.committed_records.load(Ordering::Acquire), 4);
    assert_eq!(health.committed_batches.load(Ordering::Acquire), 3);
    assert_eq!(health.max_batch_records.load(Ordering::Acquire), 2);
}

#[test]
fn failed_ordinary_batch_rolls_back_before_next_app_barrier_and_then_recovers() {
    let connection = writer_connection();
    connection
        .execute_batch(
            "CREATE TEMP TRIGGER reject_event BEFORE INSERT ON durable_state_events
         WHEN NEW.event_id = 'reject' BEGIN SELECT RAISE(ABORT, 'disk fault'); END;
         CREATE TEMP TRIGGER check_prior_rollback BEFORE INSERT ON app_installations
         WHEN EXISTS (SELECT 1 FROM durable_state_events) BEGIN
            SELECT RAISE(ABORT, 'failed batch leaked partial writes');
         END;",
        )
        .unwrap();
    let (sender, receiver) = mpsc::sync_channel(16);
    let rolled_back = enqueue_event(&sender, "rolled-back");
    let rejected = enqueue_event(&sender, "reject");
    let stage = enqueue_app(&sender, create("todo"));
    let after = enqueue_event(&sender, "after");
    drop(sender);
    let health = Arc::new(DurableWriterHealth::default());
    run_preloaded_writer(connection, receiver, health.clone());
    assert!(rolled_back.recv().unwrap().is_err());
    assert!(rejected.recv().unwrap().is_err());
    assert!(stage.recv().unwrap().is_ok());
    assert_eq!(after.recv().unwrap().unwrap(), 1);
    assert_eq!(health.committed_records.load(Ordering::Acquire), 1);
}

#[test]
fn failed_app_transaction_keeps_ordinary_writes_and_leaves_no_initial_identity() {
    let connection = writer_connection();
    connection.execute_batch(
        "CREATE TEMP TRIGGER fail_app_stage BEFORE UPDATE OF pending_generation ON app_installations
         BEGIN SELECT RAISE(ABORT, 'stage fault'); END;
         CREATE TEMP TRIGGER check_app_rollback BEFORE INSERT ON durable_state_events
         WHEN NEW.event_id = 'after' AND (EXISTS (SELECT 1 FROM app_installations)
            OR EXISTS (SELECT 1 FROM app_installation_updates)) BEGIN
            SELECT RAISE(ABORT, 'failed App leaked partial writes');
         END;",
    ).unwrap();
    let (sender, receiver) = mpsc::sync_channel(16);
    let before = enqueue_event(&sender, "before");
    let stage = enqueue_app(&sender, create("todo"));
    let after = enqueue_event(&sender, "after");
    drop(sender);
    run_preloaded_writer(
        connection,
        receiver,
        Arc::new(DurableWriterHealth::default()),
    );
    assert_eq!(before.recv().unwrap().unwrap(), 1);
    assert!(matches!(
        stage.recv().unwrap(),
        Err(InstallationError::Database(_))
    ));
    assert_eq!(after.recv().unwrap().unwrap(), 2);
}

#[test]
fn inbox_accepts_only_for_the_active_generation_and_settles_failed_occurrences_with_a_notice() {
    use super::app_inbox::{AppInboxOperation, AppInboxOutcome};
    use chariox_app_runtime::app_inbox::{
        Accepted, InboxError, InboxRoute, InboxState, MAX_ATTEMPTS,
    };
    let database = Database::new();
    let store = database.open();
    let token = staged_token(
        store
            .mutate_app_installation("owner", create("todo"))
            .unwrap(),
    );
    for mutation in [
        AppRegistryMutation::Decide {
            token: token.clone(),
            decision: approval(),
            now_ms: 2,
        },
        AppRegistryMutation::Quiesce {
            token: token.clone(),
            now_ms: 3,
        },
        AppRegistryMutation::MarkPrepared {
            token: token.clone(),
            now_ms: 4,
        },
        AppRegistryMutation::Commit {
            token: token.clone(),
            now_ms: 5,
        },
    ] {
        store.mutate_app_installation("owner", mutation).unwrap();
    }
    // The trust binding a signed install records for its active generation.
    rusqlite::Connection::open(database.0.join("kernel.db"))
        .unwrap()
        .execute(
            "INSERT INTO app_installation_stage_trust(installation_id,generation,base_generation,
             owner_id,package_digest,publisher_id,key_id,trust_revision,public_key_fingerprint)
             VALUES('todo',?1,?2,'owner',?3,'publisher','key',1,?4)",
            rusqlite::params![
                token.generation as i64,
                token.base_generation as i64,
                release().package_digest,
                format!("sha256:{:064x}", 5)
            ],
        )
        .unwrap();
    store
        .app_inbox(AppInboxOperation::CreateRoute {
            route: InboxRoute {
                route_id: "slack".into(),
                owner_id: "owner".into(),
                installation_id: "todo".into(),
                event_name: "todo_requested".into(),
                source_event_type: "slack.message".into(),
                source_event_version: 1,
                active: true,
                source: None,
            },
            now_ms: 10,
        })
        .unwrap();
    let accept = |route: &str, occurrence: &str, generation: u64| {
        store.app_inbox(AppInboxOperation::Accept {
            owner: "owner".into(),
            installation: "todo".into(),
            route_id: route.into(),
            occurrence_id: occurrence.into(),
            payload: serde_json::json!({ "title": "x" }),
            generation,
            now_ms: 20,
        })
    };
    // Validated against another generation than the active one: refused.
    let stale = accept("slack", "occ", token.generation + 1);
    assert!(matches!(stale, Err(InboxError::Conflict)), "{stale:?}");
    assert!(matches!(
        accept("missing", "occ", token.generation),
        Err(InboxError::NotFound)
    ));
    let Ok(AppInboxOutcome::Accepted(Accepted::New(sequence))) =
        accept("slack", "occ", token.generation)
    else {
        panic!("the active generation's occurrence must be accepted");
    };
    // A delivery the App can no longer take (its generation moved on) ends
    // failed rather than retrying, and the owner is told which one and why.
    assert!(matches!(
        store.app_inbox(AppInboxOperation::Undeliverable {
            sequence,
            now_ms: 30
        }),
        Ok(AppInboxOutcome::Recorded(Some(InboxState::Failed)))
    ));
    let notices = store.app_logs("owner", "todo", 0, 10).unwrap();
    assert_eq!(notices.len(), 1);
    assert_eq!((notices[0].at_ms, notices[0].level.as_str()), (30, "warn"));
    assert_eq!(
        notices[0].message,
        "An incoming todo_requested event from route slack was not delivered and will not be \
         retried: the App's current version no longer accepts it (an update removed the event \
         or changed its schema)."
    );
    assert_eq!(
        notices[0].fields,
        serde_json::json!({
            "kernel": true, "route_id": "slack", "event_name": "todo_requested",
            "occurrence_id": "occ", "attempts": 0,
        })
    );
    // A poison occurrence: retried attempts add no notice; the last one adds
    // one naming the occurrence and the handler's error.
    let Ok(AppInboxOutcome::Accepted(Accepted::New(poison))) =
        accept("slack", "poison", token.generation)
    else {
        panic!("a second occurrence must be accepted");
    };
    let fail = |now_ms: u64| {
        store.app_inbox(AppInboxOperation::Failed {
            sequence: poison,
            now_ms,
            reason: "app_handler_failed: Error: boom".into(),
        })
    };
    for attempt in 1..MAX_ATTEMPTS {
        assert!(matches!(
            fail(40 + u64::from(attempt)),
            Ok(AppInboxOutcome::Recorded(Some(InboxState::Retryable)))
        ));
    }
    assert_eq!(store.app_logs("owner", "todo", 0, 10).unwrap().len(), 1);
    assert!(matches!(
        fail(100),
        Ok(AppInboxOutcome::Recorded(Some(InboxState::Failed)))
    ));
    let notices = store
        .app_logs("owner", "todo", notices[0].sequence, 10)
        .unwrap();
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notices[0].message,
        "An incoming todo_requested event from route slack was not delivered and will not be \
         retried: its last of 8 delivery attempts failed."
    );
    assert_eq!(
        notices[0].fields,
        serde_json::json!({
            "kernel": true, "route_id": "slack", "event_name": "todo_requested",
            "occurrence_id": "poison", "attempts": 8,
            "reason": "app_handler_failed: Error: boom",
        })
    );
    // A settled occurrence is not failed again, and adds no notice.
    assert!(matches!(fail(101), Err(InboxError::NotFound)));
    assert_eq!(
        store
            .app_logs("owner", "todo", notices[0].sequence, 10)
            .unwrap()
            .len(),
        0
    );
    let Ok(AppInboxOutcome::Routes(routes)) = store.app_inbox(AppInboxOperation::Routes {
        owner: "owner".into(),
        installation: "todo".into(),
    }) else {
        panic!("routes must list");
    };
    assert_eq!((routes[0].1.pending, routes[0].1.failed), (0, 2));
}
