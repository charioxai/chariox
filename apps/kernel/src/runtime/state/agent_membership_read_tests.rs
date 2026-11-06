//! MP-08 / MP-09 / MP-10 / MP-11: membership publication must not stall another Room's reads.
use super::*;
use crate::durable_state::agent_lifecycle::outside::Mutation;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[tokio::test]
async fn a_membership_commit_does_not_block_another_rooms_session_state() {
    let state = crate::test_support::allocation::runtime_with_rooms(2);
    let rooms = state.owned.session_store.list_all_sessions();
    let creating = rooms[0].clone();
    let reading = rooms[1].id().to_owned();
    let store = &state.owned.durable_state_store;
    store
        .outside_mutate(Mutation::Subscribe {
            id: "membership-read".into(),
            home: true,
            targets: vec![crate::local::EventTarget {
                kernel: "membership-test".into(),
                session_id: creating.id().into(),
                agent_id: None,
            }],
            baseline: vec![],
            open_decisions: vec![],
        })
        .unwrap();
    store
        .outside_mutate(Mutation::Identity {
            kernel: "membership-test".into(),
            machine: "membership-test".into(),
        })
        .unwrap();

    // Hold SQLite's real writer, leaving its WAL reader free. The membership operation
    // must await its durable result, while an unrelated Room can still read its roster.
    let db = rusqlite::Connection::open(store.path()).unwrap();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    let received = store.writer_requests_received();
    let agents = state.owned.agent_store.clone();
    let creator = std::thread::spawn(move || {
        agents.create_agent_for_session(
            crate::agent::CreateAgentRequest::new(creating.id(), "claude"),
            &creating,
        )
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while store.writer_requests_received() == received && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    let publication_started = store.writer_requests_received() > received;
    let (tx, rx) = mpsc::channel();
    let reader = state.clone();
    let runtime = tokio::runtime::Handle::current();
    let read = std::thread::spawn(move || {
        let _runtime = runtime.enter();
        let result = reader.session_state_response(crate::local::GetSessionStateRequest {
            session_id: reading,
        });
        let _ = tx.send(result);
    });
    let during_commit = rx.recv_timeout(Duration::from_millis(300));
    // Always release the writer and join both threads before asserting, including on red.
    db.execute_batch("COMMIT").unwrap();
    let created = creator.join().unwrap().unwrap();
    read.join().unwrap();
    assert!(
        publication_started,
        "the real membership writer was reached"
    );
    assert!(
        matches!(
            during_commit,
            Ok(Ok(LocalDaemonResponse::SessionState { .. }))
        ),
        "GetSessionState waited for another Room's membership commit"
    );
    let events = store
        .outside_list("membership-read", None, 100)
        .unwrap()
        .unwrap();
    assert!(events.events.iter().any(|event| {
        event.agent_id.as_deref() == Some(created.id())
            && matches!(
                event.value,
                crate::local::OutsideEventValue::Session {
                    state: crate::local::EventSessionState::AgentAdded
                }
            )
    }));
}
