//! MP-08 / MP-09 / MP-10 / MP-11: durable hard-stop intent and restart receipts.
use super::*;
use crate::local::AgentDeadlineState;

#[test]
fn lane_deadline_survives_restart_and_retains_reached_without_a_stop_receipt() {
    let root = crate::test_support::TestWorktree::new("lane-deadline-restart");
    {
        let store = DurableKernelStateStore::open_owned(root.path().join("state.sqlite")).unwrap();
        store
            .agent_lifecycle(Operation::SetDeadline {
                room: "room".into(),
                agent: "parent".into(),
                deadline: 100,
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
fn lane_deadline_cannot_be_extended_or_fired_early() {
    let fixture = tests::Fixture::new();
    fixture.apply(Operation::SetDeadline {
        room: "room".into(),
        agent: "parent".into(),
        deadline: 100,
    });
    assert!(fixture
        .store
        .agent_lifecycle(Operation::SetDeadline {
            room: "room".into(),
            agent: "parent".into(),
            deadline: 200
        })
        .is_err());
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
        deadline: 90,
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
fn armed_deadline_reads_advance_through_the_durable_writer() {
    let fixture = tests::Fixture::new();
    fixture.apply(Operation::SetDeadline {
        room: "room".into(),
        agent: "parent".into(),
        deadline: crate::session::unix_epoch_ms() + 60_000,
    });
    std::thread::sleep(std::time::Duration::from_millis(1_100));
    assert_eq!(
        fixture
            .store
            .agent_deadline("room", "parent")
            .unwrap()
            .unwrap()
            .state,
        AgentDeadlineState::Armed
    );
    std::thread::sleep(std::time::Duration::from_millis(1_100));
    assert_eq!(fixture.store.agent_deadlines().unwrap().len(), 1);
}
