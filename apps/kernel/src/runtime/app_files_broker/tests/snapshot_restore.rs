//! Focused protocol-410 ownership and crash drill using two private SQLite
//! kernels and actual fixed-libc workers. It needs no host enrollment/service.
use super::*;
use crate::{
    durable_state::{
        app_file_grants::{FileGrantCommand, FilePick, GrantedFile, PickState},
        app_snapshot_restore::RestoreFault,
        app_state::AppStateOperation,
    },
    local::AppRequestErrorCode as Error,
    runtime::{app_snapshot_broker::AppSnapshotBroker, app_snapshot_restore as restore},
};
use chariox_app_runtime::{
    managed_state::{StateChanges, StateWrite, Wake, WakeChange},
    worker_peer::{Broker, BrokerFuture, BrokerRequest},
};
use serde_json::{json, Value};
use std::os::unix::fs::DirBuilderExt;

struct Snapshot(AppSnapshotBroker);
impl Broker for Snapshot {
    fn handle(&self, request: BrokerRequest) -> BrokerFuture {
        let broker = self.0.clone();
        Box::pin(async move { broker.dispatch(request).await })
    }
}
fn replace(data: &chariox_app_runtime::worker_process::PrivateData, path: &str, bytes: &[u8]) {
    data.prepare_replace(path, bytes)
        .unwrap()
        .publish()
        .unwrap();
}
fn state(f: &Fixture, key: &str, value: &str) {
    f.store
        .execute_app_state(
            "alice",
            f.catalog.clone(),
            AppStateOperation::Transaction {
                changes: StateChanges::new(
                    0,
                    vec![],
                    vec![StateWrite::Put {
                        key: key.into(),
                        value: json!(value),
                    }],
                )
                .unwrap(),
                occurrences: vec![],
                wakes_count_as_use: false,
                wakes: vec![WakeChange::Set(Wake {
                    id: "reminder".into(),
                    due_at_ms: 1000,
                    revision: "r1".into(),
                })],
            },
            AppOperationBudget::from_supervisor(|| false),
        )
        .unwrap();
}
fn snapshot(f: &Fixture) -> String {
    let check = f.store.path().parent().unwrap().join("copy-check");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&check)
        .unwrap();
    let copied = f.data().copy_tree(
        &check,
        crate::runtime::app_snapshot_broker::LIMITS,
        &mut || true,
    );
    std::fs::remove_dir_all(&check).unwrap();
    assert!(copied.is_ok(), "snapshot copy preflight: {copied:?}");
    let root =
        crate::runtime::app_snapshot_broker::root(&f.store, f.catalog.installation_id()).unwrap();
    assert!(
        crate::runtime::app_snapshot_broker::free_bytes(&root).unwrap() > 3 * 1024 * 1024 * 1024,
        "snapshot reserve preflight"
    );
    let broker = AppSnapshotBroker::new(
        f.store.clone(),
        "alice".into(),
        f.catalog.clone(),
        f.admission.clone(),
        f.data(),
        Arc::new(tokio::sync::RwLock::new(())),
    );
    f.runtime.block_on(async {
        let mut peer = TestPeer::start_with(Arc::new(Snapshot(broker)));
        peer.send(
            "saved",
            "files.snapshot",
            json!({"name":"saved", "consistency":"quiescent"}),
        )
        .await;
        let id = peer.response().await.1.unwrap()["snapshotId"]
            .as_str()
            .unwrap()
            .to_owned();
        peer.close().await;
        id
    })
}
fn reopen(f: &mut Fixture) -> DurableKernelStateStore {
    let path = f.store.path().to_owned();
    f.store.fence_writer().unwrap();
    let parked =
        DurableKernelStateStore::open_owned(path.with_file_name("retired.sqlite")).unwrap();
    drop(std::mem::replace(&mut f.store, parked));
    let reopened = DurableKernelStateStore::open_owned(path).unwrap();
    f.store = reopened.clone();
    reopened
}
fn rows(store: &DurableKernelStateStore, table: &str) -> Vec<String> {
    let connection = rusqlite::Connection::open(store.path()).unwrap();
    let mut statement = connection
        .prepare(&format!("SELECT * FROM \"{table}\""))
        .unwrap();
    let count = statement.column_count();
    let mut rows = statement
        .query_map([], |row| {
            Ok((0..count)
                .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                .collect::<Vec<_>>()
                .join("|"))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    rows.sort();
    rows
}
fn retained_tables(store: &DurableKernelStateStore) -> Vec<(String, Vec<String>)> {
    let connection = rusqlite::Connection::open(store.path()).unwrap();
    let mut statement = connection
        .prepare(
            "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'app_%' ORDER BY name",
        )
        .unwrap();
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|name| name.unwrap())
        .filter(|name| {
            ![
                "app_state_values",
                "app_state_heads",
                "app_wakes",
                "app_restore_receipts",
            ]
            .contains(&name.as_str())
        })
        .map(|name| {
            let values = rows(store, &name);
            (name, values)
        })
        .collect()
}
fn live_grant(f: &Fixture) -> String {
    let now = crate::session::unix_epoch_ms();
    f.store
        .app_file_grant(FileGrantCommand::Create(FilePick {
            operation_id: "keep-live".into(),
            owner: "alice".into(),
            installation: f.catalog.installation_id().into(),
            generation: f.catalog.generation(),
            accept: vec![],
            multiple: false,
            state: PickState::Pending,
            expires_ms: now + 60_000,
            grants: vec![],
        }))
        .unwrap();
    f.store
        .app_file_grant(FileGrantCommand::Grant {
            owner: "alice".into(),
            operation_id: "keep-live".into(),
            files: vec![GrantedFile {
                name: "kept.txt".into(),
                contents: b"external".to_vec(),
            }],
            now_ms: now,
        })
        .unwrap()
        .unwrap()
        .grants[0]
        .clone()
}

#[test]
fn saved_snapshot_restore_overwrites_deletes_preserves_neighbour_and_authority() {
    let mut f = Fixture::new(Mode::Ready);
    let neighbour = Fixture::new(Mode::Ready);
    replace(&f.data(), "fixture-file", b"saved");
    replace(&neighbour.data(), "fixture-file", b"neighbour");
    state(&f, "todo", "saved");
    let id = snapshot(&f);
    let grant = live_grant(&f); // authority created AFTER the snapshot must survive.
    let mut connection = rusqlite::Connection::open(f.store.path()).unwrap();
    let tx = connection.transaction().unwrap();
    chariox_app_runtime::app_outbox::AppOutbox::configure_in(
        &tx,
        &f.catalog,
        "alice",
        "kept",
        0,
        "changed",
        &chariox_app_runtime::app_outbox::AutomationTarget {
            session_id: "room".into(),
            publication_id: "publication".into(),
            endpoint_id: "endpoint".into(),
            queue_id: "queue".into(),
        },
        false,
    )
    .unwrap();
    tx.commit().unwrap();
    let occurred_at_ms = crate::session::unix_epoch_ms();
    let occurrence = chariox_app_runtime::app_outbox::Occurrence {
        automation_id: "kept".into(),
        occurrence_id: chariox_app_runtime::app_outbox::occurrence_id(
            "accepted-after-snapshot",
            occurred_at_ms,
        )
        .unwrap(),
        event_version: 1,
        occurred_at_ms,
        schedule_revision: None,
        payload: json!({"text":"kept"}),
        invocation: chariox_app_runtime::app_outbox::Invocation {
            prompt: "kept".into(),
            artifacts: vec![],
        },
    };
    let receipt = f
        .store
        .execute_app_state(
            "alice",
            f.catalog.clone(),
            AppStateOperation::Emit(occurrence.clone()),
            AppOperationBudget::from_supervisor(|| false),
        )
        .unwrap();
    assert!(matches!(
        receipt,
        crate::durable_state::app_state::AppStateOutcome::Receipt(_)
    ));
    connection.execute_batch("INSERT INTO app_state_values VALUES('neighbour','kept',1,'123'); INSERT INTO app_state_heads VALUES('neighbour',1,1,7);").unwrap();
    replace(&f.data(), "fixture-file", b"new");
    replace(&f.data(), "delete-me", b"post-snapshot");
    state(&f, "todo", "new");
    state(&f, "delete-me", "new");
    let retained = retained_tables(&f.store);
    let data = f.data();
    f.shutdown();
    restore::restore(&f.store, "alice", f.catalog.clone(), &data, &id).unwrap();
    assert_eq!(data.read_file("fixture-file", 64).unwrap(), b"saved");
    assert!(data.read_file("delete-me", 64).is_err());
    let state = f
        .store
        .export_app_state("alice", f.catalog.installation_id())
        .unwrap();
    assert_eq!(state["values"].as_array().unwrap().len(), 1);
    assert_eq!(state["values"][0]["value_json"], "\"saved\"");
    assert!(state["values"][0]["version"].as_u64().unwrap() > 3);
    assert_eq!(state["wakes"][0]["id"], "reminder");
    assert_eq!(
        connection
            .query_row(
                "SELECT value_json FROM app_state_values WHERE installation_id='neighbour'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "123"
    );
    let duplicate = f
        .store
        .execute_app_state(
            "alice",
            f.catalog.clone(),
            AppStateOperation::Emit(occurrence),
            AppOperationBudget::from_supervisor(|| false),
        )
        .unwrap();
    assert_eq!(
        duplicate, receipt,
        "accepted occurrences return the retained receipt after restore"
    );
    assert_eq!(
        retained_tables(&f.store),
        retained,
        "all authority and occurrence tables are unchanged"
    );
    assert!(f
        .store
        .claim_app_file_grant(FileGrantCommand::Claim {
            owner: "alice".into(),
            installation: f.catalog.installation_id().into(),
            grant_id: grant,
            now_ms: crate::session::unix_epoch_ms()
        })
        .is_ok());
    assert_eq!(
        neighbour.data().read_file("fixture-file", 64).unwrap(),
        b"neighbour"
    );
    assert!(!f
        .store
        .path()
        .parent()
        .unwrap()
        .join("app-restores")
        .join(f.catalog.installation_id())
        .exists());
}

#[test]
fn saved_snapshot_restore_refuses_foreign_stale_incompatible_and_corrupt_before_mutation() {
    let f = Fixture::new(Mode::Ready);
    let other = Fixture::new(Mode::Ready);
    replace(&f.data(), "fixture-file", b"saved");
    state(&f, "todo", "saved");
    let id = snapshot(&f);
    assert!(matches!(
        restore::validate(&f.store, "bob", f.catalog.installation_id(), 1, &id),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        restore::validate(
            &other.store,
            "alice",
            other.catalog.installation_id(),
            1,
            &id
        ),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        restore::validate(&f.store, "alice", f.catalog.installation_id(), 2, &id),
        Err(Error::Conflict)
    ));
    assert!(matches!(
        restore::validate(
            &f.store,
            "alice",
            f.catalog.installation_id(),
            1,
            "../snapshot"
        ),
        Err(Error::InvalidRequest)
    ));
    let path = crate::runtime::app_snapshot_broker::root(&f.store, f.catalog.installation_id())
        .unwrap()
        .join(&id);
    let original: Value =
        serde_json::from_slice(&std::fs::read(path.join("manifest.json")).unwrap()).unwrap();
    for (field, value) in [
        ("data_schema", json!(2)),
        ("generation", json!(2)),
        ("installation_id", json!("foreign")),
        ("owner_id", json!("bob")),
        ("schema", json!("chariox.app-snapshot.v1")),
        ("package_digest", json!("sha256:foreign")),
    ] {
        let mut manifest = original.clone();
        manifest[field] = value;
        std::fs::write(
            path.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(
            restore::validate(&f.store, "alice", f.catalog.installation_id(), 1, &id).is_err(),
            "{field}"
        );
        assert_eq!(f.data().read_file("fixture-file", 64).unwrap(), b"saved");
    }
    std::fs::write(
        path.join("manifest.json"),
        serde_json::to_vec(&original).unwrap(),
    )
    .unwrap();
    std::fs::write(path.join("files/fixture-file"), b"corrupt").unwrap();
    assert!(matches!(
        restore::validate(&f.store, "alice", f.catalog.installation_id(), 1, &id),
        Err(Error::DigestMismatch)
    ));
}

#[test]
fn saved_snapshot_restore_crashes_before_and_after_commit_replay_idempotently_after_reopen() {
    for committed in [false, true] {
        let mut f = Fixture::new(Mode::Ready);
        replace(&f.data(), "fixture-file", b"saved");
        state(&f, "todo", "saved");
        let id = snapshot(&f);
        replace(&f.data(), "fixture-file", b"prior");
        replace(&f.data(), "post-snapshot", b"prior-extra");
        state(&f, "todo", "prior");
        let data = f.data();
        f.shutdown();
        let fault = if committed {
            RestoreFault::AfterCommit
        } else {
            RestoreFault::BeforeCommit
        };
        assert!(matches!(
            restore::fixture_restore(&f.store, "alice", f.catalog.clone(), &data, &id, fault),
            Err(Error::StorageUnavailable)
        ));
        assert!(f.store.require_writer_healthy().is_err());
        // The writer commits only structured state and its receipt. Even an
        // after-commit interruption leaves bytes untouched until recovery.
        assert_eq!(data.read_file("fixture-file", 64).unwrap(), b"prior");
        assert_eq!(data.read_file("post-snapshot", 64).unwrap(), b"prior-extra");
        // Reopen uses SQLite restart state rather than trusting the dead writer.
        f.store.fence_writer().unwrap();
        let reopened = reopen(&mut f);
        let expected = if committed {
            b"saved".as_slice()
        } else {
            b"prior".as_slice()
        };
        assert!(restore::fixture_interrupt_replay(&reopened, "alice", &data).is_err());
        let journal = reopened
            .path()
            .parent()
            .unwrap()
            .join("app-restores")
            .join(data.installation_id());
        assert!(
            journal.exists(),
            "an interrupted replay retains its recovery owner"
        );
        restore::recover(&reopened, "alice", &data).unwrap();
        restore::recover(&reopened, "alice", &data).unwrap();
        assert_eq!(data.read_file("fixture-file", 64).unwrap(), expected);
        assert_eq!(data.read_file("post-snapshot", 64).is_err(), committed);
        let state = reopened
            .export_app_state("alice", data.installation_id())
            .unwrap();
        assert_eq!(
            state["values"][0]["value_json"],
            if committed { "\"saved\"" } else { "\"prior\"" }
        );
        assert!(!journal.exists());
    }
}

#[test]
fn saved_snapshot_restore_repeating_the_same_snapshot_cannot_reuse_a_prior_commit_receipt() {
    let mut f = Fixture::new(Mode::Ready);
    replace(&f.data(), "fixture-file", b"saved");
    state(&f, "todo", "saved");
    let id = snapshot(&f);
    let data = f.data();
    f.shutdown();
    restore::restore(&f.store, "alice", f.catalog.clone(), &data, &id).unwrap();
    replace(&data, "fixture-file", b"second-prior");
    assert!(restore::fixture_restore(
        &f.store,
        "alice",
        f.catalog.clone(),
        &data,
        &id,
        RestoreFault::BeforeCommit
    )
    .is_err());
    f.store.fence_writer().unwrap();
    let reopened = reopen(&mut f);
    restore::recover(&reopened, "alice", &data).unwrap();
    assert_eq!(data.read_file("fixture-file", 64).unwrap(), b"second-prior");
}

#[test]
fn saved_snapshot_restore_corrupt_journal_blocks_recovery_and_staged_generation() {
    let mut f = Fixture::new(Mode::Ready);
    replace(&f.data(), "fixture-file", b"saved");
    state(&f, "todo", "saved");
    let id = snapshot(&f);
    replace(&f.data(), "fixture-file", b"prior");
    let data = f.data();
    f.shutdown();
    assert!(restore::fixture_restore(
        &f.store,
        "alice",
        f.catalog.clone(),
        &data,
        &id,
        RestoreFault::AfterCommit
    )
    .is_err());
    f.store.fence_writer().unwrap();
    let reopened = reopen(&mut f);
    assert!(matches!(
        restore::require_recovery_generation(&reopened, "alice", data.installation_id(), 2),
        Err(Error::Conflict)
    ));
    let journal = reopened
        .path()
        .parent()
        .unwrap()
        .join("app-restores")
        .join(data.installation_id());
    let mut value: Value =
        serde_json::from_slice(&std::fs::read(journal.join("journal.json")).unwrap()).unwrap();
    value["restore_id"] = json!("restore-corrupt");
    std::fs::write(
        journal.join("journal.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        restore::recover(&reopened, "alice", &data),
        Err(Error::DigestMismatch)
    ));
    assert!(journal.exists());
    assert_eq!(
        data.read_file("fixture-file", 64).unwrap(),
        b"prior",
        "corrupt recovery leaves the pre-commit private tree untouched"
    );
}
