//! `files.snapshot` through a real worker's private data: what a snapshot
//! holds, its label and retention, the quiescent fence, and a restore into an
//! isolated second installation.
use super::*;
use crate::runtime::app_snapshot_broker::{self, AppSnapshotBroker, LIMITS};
use chariox_app_runtime::worker_peer::{Broker, BrokerFuture, BrokerRequest};
use serde_json::Value;
use std::{os::unix::fs::DirBuilderExt, sync::Arc, time::Duration};

struct Snapshots(AppSnapshotBroker);
impl Broker for Snapshots {
    fn handle(&self, request: BrokerRequest) -> BrokerFuture {
        let service = self.0.clone();
        Box::pin(async move { service.dispatch(request).await })
    }
}

fn private_dir(path: &std::path::Path) {
    std::fs::DirBuilder::new().mode(0o700).create(path).unwrap();
}

fn seed(fixture: &Fixture) {
    let source = std::fs::canonicalize(std::env::temp_dir())
        .unwrap()
        .join(format!("chariox-seed-{:016x}", rand::random::<u64>()));
    private_dir(&source);
    private_dir(&source.join("notes"));
    std::fs::write(source.join("notes/plan.md"), "# Plan").unwrap();
    std::fs::write(source.join("fixture-file"), "x").unwrap();
    std::os::unix::fs::symlink("notes", source.join("link")).unwrap();
    // Replaces what the worker fixture wrote at start.
    let restored = fixture.data().restore_tree(&source, LIMITS).unwrap();
    assert_eq!((restored.files.len(), restored.skipped), (2, 1));
    assert!(fixture.data().read_file("ready-ack", 64).is_err());
    std::fs::remove_dir_all(source).unwrap();
    let installation = fixture.catalog.installation_id();
    rusqlite::Connection::open(fixture.store.path())
        .unwrap()
        .execute_batch(&format!(
            "INSERT INTO app_state_values(installation_id,key,version,value_json)
               VALUES('{installation}','todos',3,'[\"buy milk\"]');
             INSERT INTO app_state_heads(installation_id,revision,key_count,payload_bytes)
               VALUES('{installation}',3,1,12);
             INSERT INTO app_wakes(owner_id,installation_id,wake_id,due_at_ms,revision,attempts,next_attempt_at_ms)
               VALUES('alice','{installation}','reminder',1000,'r1',2,5000);"
        ))
        .unwrap();
}

fn snapshots(fixture: &Fixture) -> AppSnapshotBroker {
    AppSnapshotBroker::new(
        fixture.store.clone(),
        "alice".into(),
        fixture.catalog.clone(),
        fixture.admission.clone(),
        fixture.data(),
        Arc::new(tokio::sync::RwLock::new(())),
    )
}

fn read(path: std::path::PathBuf) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn a_snapshot_holds_files_and_state_and_restores_into_an_isolated_installation() {
    let fixture = Fixture::new(Mode::Ready);
    seed(&fixture);
    let installation = fixture.catalog.installation_id().to_owned();
    let root = app_snapshot_broker::root(&fixture.store, &installation).unwrap();
    let broker = snapshots(&fixture);
    let (quiescent, crash) = fixture.runtime.block_on(async {
        let mut peer = TestPeer::start_with(Arc::new(Snapshots(broker.clone())));
        for (params, code) in [
            (
                serde_json::json!({"name":"bad name","consistency":"quiescent"}),
                "INVALID_ARGUMENT",
            ),
            (
                serde_json::json!({"name":"nightly","consistency":"eventual"}),
                "INVALID_ARGUMENT",
            ),
            (serde_json::json!({"name":"nightly"}), "INVALID_ARGUMENT"),
        ] {
            peer.send("bad", "files.snapshot", params).await;
            assert_eq!(peer.response().await.1.unwrap_err().code, code);
        }
        let mut taken = Vec::new();
        for (name, consistency) in [
            ("nightly", "quiescent"),
            ("before-import", "crash_consistent"),
        ] {
            peer.send(
                name,
                "files.snapshot",
                serde_json::json!({"name": name, "consistency": consistency}),
            )
            .await;
            let reply = peer.response().await.1.unwrap();
            assert_eq!(
                (&reply["consistency"], &reply["files"], &reply["bytes"]),
                (
                    &serde_json::json!(consistency),
                    &serde_json::json!(2),
                    &serde_json::json!(7)
                )
            );
            taken.push(reply["snapshotId"].as_str().unwrap().to_owned());
        }
        peer.close().await;
        (taken[0].clone(), taken[1].clone())
    });
    let manifest = read(root.join(&quiescent).join("manifest.json"));
    assert_eq!(manifest["consistency"], "quiescent");
    assert_eq!(
        read(root.join(&crash).join("manifest.json"))["consistency"],
        "crash_consistent"
    );
    assert_eq!(manifest["generation"], fixture.catalog.generation());
    assert_eq!(
        manifest["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|file| file["path"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["fixture-file", "notes/plan.md"]
    );
    // The seed's symlink never reached App data (see `seed`).
    assert_eq!(manifest["skipped"], 0);
    let state = read(root.join(&quiescent).join("state.json"));
    assert_eq!(state["values"][0]["value_json"], "[\"buy milk\"]");
    assert_eq!(state["wakes"][0]["id"], "reminder");
    // Delivery receipts are not App data.
    assert!(state.get("outbox").is_none() && state.get("inbox").is_none());

    // An isolated installation restored from it shows the same files, state
    // and schedule.
    let isolated = Fixture::new(Mode::Ready);
    let restored = isolated
        .data()
        .restore_tree(&root.join(&quiescent).join("files"), LIMITS)
        .unwrap();
    assert_eq!(restored.files.len(), 2);
    assert_eq!(
        isolated.data().read_file("notes/plan.md", 64).unwrap(),
        b"# Plan"
    );
    let target = isolated.catalog.installation_id();
    isolated
        .store
        .fixture_import_app_state("alice", target, &state)
        .unwrap();
    let again = isolated.store.export_app_state("alice", target).unwrap();
    assert_eq!(
        (&again["values"], &again["head"], &again["wakes"]),
        (&state["values"], &state["head"], &state["wakes"])
    );
    // The isolated worker's own start-up files were replaced.
    assert!(isolated.data().read_file("ready-ack", 64).is_err());
}

#[test]
fn a_quiescent_snapshot_waits_for_sdk_writes_and_only_two_are_kept() {
    let fixture = Fixture::new(Mode::Ready);
    seed(&fixture);
    let root =
        app_snapshot_broker::root(&fixture.store, fixture.catalog.installation_id()).unwrap();
    let fence = Arc::new(tokio::sync::RwLock::new(()));
    let broker = AppSnapshotBroker::new(
        fixture.store.clone(),
        "alice".into(),
        fixture.catalog.clone(),
        fixture.admission.clone(),
        fixture.data(),
        fence.clone(),
    );
    fixture.runtime.block_on(async {
        let mut peer = TestPeer::start_with(Arc::new(Snapshots(broker)));
        // An SDK write in flight holds the fence: the snapshot waits for it.
        let write = fence.clone().read_owned().await;
        peer.send(
            "held",
            "files.snapshot",
            serde_json::json!({"name":"held","consistency":"quiescent"}),
        )
        .await;
        assert!(
            tokio::time::timeout(Duration::from_millis(300), peer.response())
                .await
                .is_err(),
            "the snapshot must wait for the write"
        );
        drop(write);
        assert!(peer.response().await.1.is_ok());
        // A crash-consistent one does not wait.
        let _write = fence.clone().read_owned().await;
        for index in 0..2 {
            peer.send(
                &format!("crash-{index}"),
                "files.snapshot",
                serde_json::json!({"name":"crash","consistency":"crash_consistent"}),
            )
            .await;
            assert!(peer.response().await.1.is_ok());
        }
        peer.close().await;
    });
    let kept = std::fs::read_dir(&root).unwrap().count();
    assert_eq!(kept, app_snapshot_broker::RETAINED, "the oldest is dropped");
}
