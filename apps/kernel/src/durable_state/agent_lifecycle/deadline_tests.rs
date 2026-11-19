//! MP-08 / MP-09 / MP-10 / MP-11: durable hard-stop intent and restart receipts.
use super::*;
use crate::local::AgentDeadlineState;

#[test]
fn agent_deadline_survives_restart_and_retains_reached_without_a_stop_receipt() {
    let root = crate::test_support::TestWorktree::new("agent-deadline-restart");
    {
        let store = DurableKernelStateStore::open_owned(root.path().join("state.sqlite")).unwrap();
        store
            .agent_lifecycle(Operation::SetDeadline {
                room: "room".into(),
                agent: "parent".into(),
                deadline: Some(100),
                inherited: Vec::new(),
            })
            .unwrap();
        store
            .agent_lifecycle(Operation::ReachDeadline {
                room: "room".into(),
                agent: "parent".into(),
                now: 101,
                stopped: false,
            })
            .unwrap();
    }
    let reopened = DurableKernelStateStore::open_owned(root.path().join("state.sqlite")).unwrap();
    let reached = reopened.agent_deadline("room", "parent").unwrap().unwrap();
    assert_eq!(reached.state, AgentDeadlineState::Reached);
    assert_eq!(reached.reached_at_ms, Some(101));
    assert_eq!(reached.stopped_at_ms, None);
    reopened
        .agent_lifecycle(Operation::ReachDeadline {
            room: "room".into(),
            agent: "parent".into(),
            now: 110,
            stopped: true,
        })
        .unwrap();
    let stopped = reopened.agent_deadline("room", "parent").unwrap().unwrap();
    assert_eq!(stopped.reached_at_ms, Some(101));
    assert_eq!(stopped.stopped_at_ms, Some(110));
}

#[test]
fn agent_deadline_can_be_extended_cleared_and_shortened_but_not_fired_early() {
    let fixture = tests::Fixture::new();
    fixture.apply(Operation::SetDeadline {
        room: "room".into(),
        agent: "parent".into(),
        deadline: Some(100),
        inherited: Vec::new(),
    });
    fixture
        .store
        .agent_lifecycle(Operation::SetDeadline {
            room: "room".into(),
            agent: "parent".into(),
            deadline: Some(200),
            inherited: Vec::new(),
        })
        .unwrap();
    assert!(fixture
        .store
        .agent_lifecycle(Operation::ReachDeadline {
            room: "room".into(),
            agent: "parent".into(),
            now: 99,
            stopped: false
        })
        .is_err());
    fixture.apply(Operation::SetDeadline {
        room: "room".into(),
        agent: "parent".into(),
        deadline: None,
        inherited: Vec::new(),
    });
    assert!(fixture
        .store
        .agent_deadline("room", "parent")
        .unwrap()
        .is_none());
    fixture.apply(Operation::SetDeadline {
        room: "room".into(),
        agent: "parent".into(),
        deadline: Some(90),
        inherited: Vec::new(),
    });
    let value = fixture
        .store
        .agent_deadline("room", "parent")
        .unwrap()
        .unwrap();
    assert_eq!(value.deadline_ms, 90);
    assert_eq!(value.state, AgentDeadlineState::Armed);
}

#[test]
fn edited_deadlines_and_explicit_child_limits_survive_restart() {
    let root = crate::test_support::TestWorktree::new("deadline-edit-restart");
    let path = root.path().join("state.sqlite");
    {
        let store = DurableKernelStateStore::open_owned(path.clone()).unwrap();
        for deadline in [Some(100), Some(200), None, Some(90)] {
            store
                .agent_lifecycle(Operation::SetDeadline {
                    room: "room".into(),
                    agent: "parent".into(),
                    deadline,
                    inherited: vec![("child".into(), deadline)],
                })
                .unwrap();
        }
        store
            .agent_lifecycle(Operation::SetDeadline {
                room: "room".into(),
                agent: "cleared".into(),
                deadline: Some(100),
                inherited: Vec::new(),
            })
            .unwrap();
        store
            .agent_lifecycle(Operation::SetDeadline {
                room: "room".into(),
                agent: "cleared".into(),
                deadline: None,
                inherited: Vec::new(),
            })
            .unwrap();
    }
    let store = DurableKernelStateStore::open_owned(path.clone()).unwrap();
    // The restart gave back the few milliseconds it took, to the deadline and to the owner's limit alike.
    let parent = deadline_of(&store, "parent");
    assert!((90..60_000).contains(&parent), "{parent}");
    assert_eq!(
        store.agent_deadline_intent("room", "parent").unwrap(),
        Some(parent)
    );
    assert_eq!(store.agent_deadline_intent("room", "child").unwrap(), None);
    assert!(store.agent_deadline("room", "cleared").unwrap().is_none());
    store
        .agent_lifecycle(Operation::ReachDeadline {
            room: "room".into(),
            agent: "parent".into(),
            now: parent,
            stopped: false,
        })
        .unwrap();
    for deadline in [Some(300), None] {
        assert!(store
            .agent_lifecycle(Operation::SetDeadline {
                room: "room".into(),
                agent: "parent".into(),
                deadline,
                inherited: Vec::new()
            })
            .is_err());
    }
}

fn set(
    store: &DurableKernelStateStore,
    agent: &str,
    deadline: u64,
    inherited: Vec<(String, Option<u64>)>,
) {
    store
        .agent_lifecycle(Operation::SetDeadline {
            room: "room".into(),
            agent: agent.into(),
            deadline: Some(deadline),
            inherited,
        })
        .unwrap();
}
fn deadline_of(store: &DurableKernelStateStore, agent: &str) -> u64 {
    store
        .agent_deadline("room", agent)
        .unwrap()
        .unwrap()
        .deadline_ms
}

#[test]
fn a_kernel_start_gives_back_the_time_the_kernel_was_stopped() {
    let fixture = tests::Fixture::new();
    let store = &fixture.store;
    set(
        store,
        "parent",
        1_000_000,
        vec![("child".into(), Some(900_000))],
    );
    store
        .agent_lifecycle(Operation::BeatDeadlineClock { now: 50_000 })
        .unwrap();
    store
        .agent_lifecycle(Operation::ResumeDeadlines { now: 170_000 })
        .unwrap();
    assert_eq!(deadline_of(store, "parent"), 1_120_000);
    assert_eq!(deadline_of(store, "child"), 1_020_000);
    assert_eq!(
        store.agent_deadline_intent("room", "parent").unwrap(),
        Some(1_120_000),
        "a later edit starts from the paused limit"
    );
    // The next start counts only the time since the beat the first one left.
    store
        .agent_lifecycle(Operation::ResumeDeadlines { now: 175_000 })
        .unwrap();
    assert_eq!(deadline_of(store, "parent"), 1_125_000);
}

#[test]
fn opening_the_owned_store_resumes_the_deadlines() {
    let root = crate::test_support::TestWorktree::new("deadline-open-resume");
    let path = root.path().join("state.sqlite");
    {
        let store = DurableKernelStateStore::open_owned(path.clone()).unwrap();
        set(&store, "parent", 1_000, Vec::new());
        store
            .agent_lifecycle(Operation::BeatDeadlineClock { now: 1 })
            .unwrap();
    }
    let store = DurableKernelStateStore::open_owned(path).unwrap();
    // The beat is 1 ms after the epoch, so the whole time since then is given back.
    assert!(deadline_of(&store, "parent") > 1_000_000_000_000);
}

#[test]
fn a_reached_deadline_and_a_deadline_without_a_beat_stay_where_they_are() {
    let fixture = tests::Fixture::new();
    let store = &fixture.store;
    set(store, "parent", 100, Vec::new());
    store
        .agent_lifecycle(Operation::ReachDeadline {
            room: "room".into(),
            agent: "parent".into(),
            now: 101,
            stopped: true,
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::ResumeDeadlines { now: 500_000 })
        .unwrap();
    assert_eq!(deadline_of(store, "parent"), 100);

    let empty = tests::Fixture::new();
    empty
        .store
        .agent_lifecycle(Operation::ResumeDeadlines { now: 500_000 })
        .unwrap();
    assert!(empty.store.agent_deadlines().unwrap().is_empty());
}

#[test]
fn arming_leaves_the_beat_a_kernel_stopped_right_after_resumes_from() {
    let fixture = tests::Fixture::new();
    let armed_at = crate::session::unix_epoch_ms();
    set(&fixture.store, "parent", armed_at + 60_000, Vec::new());
    fixture
        .store
        .agent_lifecycle(Operation::ResumeDeadlines {
            now: armed_at + 120_000,
        })
        .unwrap();
    let moved = deadline_of(&fixture.store, "parent") - (armed_at + 60_000);
    assert!((119_000..=120_000).contains(&moved), "{moved}");
}

#[test]
fn a_shift_past_the_largest_time_is_refused_and_changes_nothing() {
    let fixture = tests::Fixture::new();
    set(&fixture.store, "parent", i64::MAX as u64 - 10, Vec::new());
    fixture
        .store
        .agent_lifecycle(Operation::BeatDeadlineClock { now: 1 })
        .unwrap();
    assert!(fixture
        .store
        .agent_lifecycle(Operation::ResumeDeadlines { now: 100 })
        .is_err());
    assert_eq!(deadline_of(&fixture.store, "parent"), i64::MAX as u64 - 10);
}
