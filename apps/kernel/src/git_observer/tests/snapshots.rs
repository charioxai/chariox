use super::super::GitTurnSnapshotStore;
use super::support::tracked_snapshot;

#[test]
fn pending_turn_snapshot_lookup_does_not_consume_snapshot() {
    let snapshots = GitTurnSnapshotStore::default();
    let snapshot = tracked_snapshot(false, "");
    snapshots.insert(snapshot.clone());

    assert_eq!(
        snapshots.get("provider-run-1", "prompt-1"),
        Some(snapshot.clone())
    );
    assert_eq!(
        snapshots.get_for_provider_run("provider-run-1"),
        Some(snapshot.clone())
    );
    assert_eq!(
        snapshots.remove("provider-run-1", "prompt-1"),
        Some(snapshot)
    );
    assert_eq!(snapshots.get_for_provider_run("provider-run-1"), None);
}
