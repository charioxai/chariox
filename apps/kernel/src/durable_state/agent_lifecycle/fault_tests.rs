//! MP-09 G05 / G18: the drill hooks behind `CHARIOX_AGENT_FAULT_DIR`.
use super::*;

struct FaultStore {
    root: std::path::PathBuf,
    store: DurableKernelStateStore,
}

impl FaultStore {
    /// The store reads its fault directory when it opens, on this thread only.
    fn open(name: &str, faults: &[&str]) -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-agent-faults-{name}-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(root.join("faults")).unwrap();
        for fault in faults {
            std::fs::write(root.join("faults").join(fault), "").unwrap();
        }
        let store = {
            let _faults = faults::TestFaultDir::set(&root.join("faults"));
            DurableKernelStateStore::open_owned(root.join("state.sqlite")).unwrap()
        };
        Self { store, root }
    }
    fn begin(&self, prompt: &str) -> Result<Outcome, crate::error::DaemonError> {
        self.store.agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: "room".into(),
            agent: "agent".into(),
            prompt: prompt.into(),
            run: Some("run".into()),
            now: 1,
        })
    }
}

impl Drop for FaultStore {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn g18_writer_fail_file_fails_lifecycle_writes_until_removed() {
    let f = FaultStore::open("writer-fail", &["writer-fail"]);
    let error = f.begin("turn-1").unwrap_err().to_string();
    assert!(error.contains("writer-fail"), "{error}");
    assert!(
        f.store.agent_tasks(Some("room"), None).unwrap().is_empty(),
        "nothing committed"
    );
    assert!(
        f.store.require_writer_healthy().is_ok(),
        "a failed commit is not a fence"
    );

    std::fs::remove_file(f.root.join("faults/writer-fail")).unwrap();
    f.begin("turn-1").unwrap();
    assert_eq!(f.store.agent_tasks(Some("room"), None).unwrap().len(), 1);
}

#[test]
fn g05_duplicate_delivery_file_replays_one_occurrence_once_more() {
    let f = FaultStore::open("duplicate-delivery", &["duplicate-delivery"]);
    f.store
        .agent_lifecycle(Operation::Occur(occurrence(
            "room",
            "agent",
            "peer",
            "occurrence-1",
            "message",
            serde_json::json!({"message": "review"}),
        )))
        .unwrap();
    let log = std::fs::read_to_string(f.root.join("faults/duplicate-delivery.log")).unwrap();
    assert_eq!(log.lines().count(), 1, "{log}");
    assert!(log.contains(" occurrence "), "{log}");
    let events = f.store.agent_inbox("room", "agent", 0).unwrap();
    assert_eq!(events.len(), 1, "one logical delivery: {events:?}");

    f.begin("not-a-delivery").unwrap();
    let log = std::fs::read_to_string(f.root.join("faults/duplicate-delivery.log")).unwrap();
    assert_eq!(log.lines().count(), 1, "only deliveries are replayed");
}

#[test]
fn drop_ack_file_commits_one_delivery_and_fails_its_acknowledgement_once() {
    let f = FaultStore::open("drop-ack", &[]);
    let ack = f.root.join("faults/drop-ack");
    std::fs::write(&ack, "occurrence\n").unwrap();
    let deliver = || {
        f.store.agent_lifecycle(Operation::Occur(occurrence(
            "room",
            "agent",
            "peer",
            "occurrence-1",
            "message",
            serde_json::json!({"message": "review"}),
        )))
    };
    f.begin("not-a-delivery").unwrap();
    assert!(
        ack.exists(),
        "a write of another kind keeps the fault armed"
    );

    let error = deliver().unwrap_err().to_string();
    assert!(error.contains("drop-ack"), "{error}");
    assert!(!ack.exists(), "the fault fires once");
    assert_eq!(
        f.store.agent_inbox("room", "agent", 0).unwrap().len(),
        1,
        "the write committed"
    );
    let log = std::fs::read_to_string(f.root.join("faults/drop-ack.log")).unwrap();
    assert!(log.contains("occurrence acknowledgement dropped"), "{log}");

    deliver().unwrap();
    assert_eq!(
        f.store.agent_inbox("room", "agent", 0).unwrap().len(),
        1,
        "the retry is the same delivery"
    );
}
