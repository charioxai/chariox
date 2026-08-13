use crate::history::{HistoryEventKind, HistoryEventQuery, OperationalHistoryStore};

use super::super::{capture_turn_snapshot, observe_after_turn, GitTurnSnapshotStore};
use super::support::{run_git, test_context};

#[test]
fn observes_commit_and_indexes_searchable_metadata() {
    let root = std::env::temp_dir().join(format!(
        "chariox-git-observer-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let history_path = root.join("history.db");
    std::fs::create_dir_all(&root).expect("temp repo should be created");
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "agent@example.com"]);
    run_git(&root, &["config", "user.name", "Agent"]);
    std::fs::write(root.join("README.md"), "seed\n").expect("seed file should write");
    run_git(&root, &["add", "README.md"]);
    run_git(&root, &["commit", "-m", "seed commit"]);

    let before = capture_turn_snapshot(test_context(&root, "prompt-1"))
        .expect("git snapshot should be captured");
    std::fs::write(root.join("feature.txt"), "hello\n").expect("feature file should write");
    run_git(&root, &["add", "feature.txt"]);
    run_git(&root, &["commit", "-m", "add searchable feature"]);
    let after = capture_turn_snapshot(test_context(&root, "prompt-1"))
        .expect("git post snapshot should be captured");

    let snapshots = GitTurnSnapshotStore::default();
    snapshots.insert(before.clone());
    let candidates = snapshots.candidates_for(&before);
    let history =
        OperationalHistoryStore::open(history_path.clone()).expect("history store should open");
    let events =
        observe_after_turn(before, after, candidates, &history).expect("observation should append");
    assert!(events
        .iter()
        .any(|event| event.kind == HistoryEventKind::GitCommitDetected));

    let subject_matches = history
        .query_events(HistoryEventQuery {
            provider: Some("dev-stub".to_string()),
            model: Some("dev-git".to_string()),
            text: Some("add searchable feature".to_string()),
            limit: Some(10),
            ..HistoryEventQuery::default()
        })
        .expect("subject query should work");
    assert_eq!(subject_matches.len(), 1);
    assert_eq!(subject_matches[0].prompt_id.as_deref(), Some("prompt-1"));
    let branch = subject_matches[0]
        .metadata
        .get("branch")
        .and_then(|value| value.as_str());
    assert!(matches!(branch, Some("master" | "main")));

    let path_matches = history
        .query_events(HistoryEventQuery {
            text: Some("feature.txt".to_string()),
            limit: Some(10),
            ..HistoryEventQuery::default()
        })
        .expect("path query should work");
    assert!(path_matches
        .iter()
        .any(|event| event.kind == HistoryEventKind::GitCommitDetected));

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(history_path.with_extension("db-wal"));
    let _ = std::fs::remove_file(history_path.with_extension("db-shm"));
}
