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
    assert_eq!(
        store
            .agent_deadline("room", "parent")
            .unwrap()
            .unwrap()
            .deadline_ms,
        90
    );
    assert_eq!(
        store.agent_deadline_intent("room", "parent").unwrap(),
        Some(90)
    );
    assert_eq!(store.agent_deadline_intent("room", "child").unwrap(), None);
    assert!(store.agent_deadline("room", "cleared").unwrap().is_none());
    store
        .agent_lifecycle(Operation::ReachDeadline {
            room: "room".into(),
            agent: "parent".into(),
            now: 91,
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
