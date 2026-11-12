//! MP-08 / MP-09 / MP-10 / MP-11: a yield never depends on bookkeeping for sources that already
//! ended. Settled, cancelled and deleted sources resolve by themselves.
use super::*;
use tests::Fixture;

/// A second, still running delegate: the source the turn really waits on.
fn running_delegate(f: &Fixture) {
    f.apply(Operation::RegisterObligation {
        owner: "owner".into(),
        room: "room".into(),
        agent: "parent".into(),
        prompt: "p".into(),
        run: Some("run".into()),
        id: "running".into(),
        kind: "delegate".into(),
        resource: Some("worker".into()),
        now: 1,
    });
    f.apply(Operation::Subscribe {
        task: "p".into(),
        prompt: "p".into(),
        registration: Registration {
            id: "running-reg".into(),
            task_id: "p".into(),
            source_id: "worker".into(),
            obligation_id: Some("running".into()),
            source_cursor: 0,
            live: true,
            peer: false,
        },
    });
}

fn yield_on(f: &Fixture, ids: &[&str]) -> Result<Outcome, DaemonError> {
    f.store.agent_lifecycle(Operation::Yield {
        task: "p".into(),
        prompt: "p".into(),
        registrations: ids.iter().map(|id| (*id).into()).collect(),
        cursor: 0,
        deadline: Some(60_000),
        reason: "worker result".into(),
        now: 2,
    })
}

/// The provider ends its turn after the yield: the wait that was admitted must hold, as it did
/// at admission, instead of the turn being corrected or blocked.
fn end_the_turn_waiting(f: &Fixture) {
    let Outcome::Settled { correction, .. } = f.apply(Operation::Settle {
        room: "room".into(),
        agent: "parent".into(),
        prompt: "p".into(),
        run: "run".into(),
        has_answer: true,
        cancelled: false,
        now: 3,
    }) else {
        panic!("the turn did not settle")
    };
    assert!(!correction, "an admitted wait needs no correction");
    assert_eq!(f.task().state, ExecutionState::Waiting);
}

fn ask_for_reply(f: &Fixture) -> InboxEvent {
    let mut request = occurrence(
        "room",
        "peer",
        "parent",
        "request",
        "message",
        serde_json::json!({"message": "answer"}),
    );
    request.reply_requested = true;
    let Outcome::Event(event) = f.apply(Operation::Send {
        task: "p".into(),
        prompt: "p".into(),
        event: request,
    }) else {
        panic!("message not admitted")
    };
    event
}

fn peer_replies(f: &Fixture, request: &InboxEvent) {
    f.apply(Operation::SourceOutcome {
        room: "room".into(),
        source: format!("agent-event-peer-{}", request.sequence),
        occurrence: "completed".into(),
        success: true,
        public_answer: Some(serde_json::json!({"excerpt": "answer"})),
        now: 2,
    });
}

#[test]
fn a_reply_that_arrived_does_not_block_a_yield_on_another_source() {
    let f = Fixture::new();
    f.begin("p");
    running_delegate(&f);
    let request = ask_for_reply(&f);
    peer_replies(&f, &request);
    // The reply event is still unhandled in the inbox: the kernel delivers it on its own.
    yield_on(&f, &["running-reg"]).expect("a yield on the running worker");
    assert_eq!(f.task().wait.unwrap().registration_ids, ["running-reg"]);
    end_the_turn_waiting(&f);
}

#[test]
fn a_handled_reply_registration_is_dropped_from_a_yield_instead_of_refusing_it() {
    let f = Fixture::new();
    f.begin("p");
    running_delegate(&f);
    let request = ask_for_reply(&f);
    peer_replies(&f, &request);
    let reply = f.store.agent_inbox("room", "parent", 0).unwrap().remove(0);
    f.apply(Operation::Ack {
        room: "room".into(),
        agent: "parent".into(),
        sequence: reply.sequence,
        handled: true,
        now: 3,
    });
    let settled = format!("completion-reply-{}", request.sequence);
    yield_on(&f, &[&settled, "running-reg"]).expect("the settled reply is not a wait source");
    assert_eq!(f.task().wait.unwrap().registration_ids, ["running-reg"]);
    end_the_turn_waiting(&f);
}

#[test]
fn a_cancelled_spawn_does_not_block_a_yield_on_another_source() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.subscribe();
    running_delegate(&f);
    f.apply(Operation::ReleaseSpawn {
        room: "room".into(),
        source: "child".into(),
    });
    assert_eq!(f.task().obligations[0].status, "cancelled");
    yield_on(&f, &["reg", "running-reg"]).expect("the cancelled follow is not a wait source");
    assert_eq!(f.task().wait.unwrap().registration_ids, ["running-reg"]);
    end_the_turn_waiting(&f);
}

#[test]
fn a_deleted_child_does_not_block_a_yield_on_another_source() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.subscribe();
    running_delegate(&f);
    f.apply(Operation::SourceOutcome {
        room: "room".into(),
        source: "child".into(),
        occurrence: "agent-gone-child-5".into(),
        success: false,
        public_answer: None,
        now: 5,
    });
    assert_eq!(f.task().obligations[0].status, "failed");
    yield_on(&f, &["running-reg"]).expect("the deleted child ended; its loss is in the inbox");
    end_the_turn_waiting(&f);
}

#[test]
fn a_yield_that_names_only_ended_sources_says_there_is_nothing_to_wait_on() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.subscribe();
    f.apply(Operation::ReleaseSpawn {
        room: "room".into(),
        source: "child".into(),
    });
    let refusal = yield_on(&f, &["reg"]).unwrap_err().to_string();
    assert!(refusal.contains("nothing to wait on"), "{refusal}");
}

#[test]
fn a_running_source_left_out_of_the_yield_is_still_refused() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.subscribe();
    running_delegate(&f);
    let refusal = yield_on(&f, &["reg"]).unwrap_err().to_string();
    assert!(
        refusal.contains("cover every unresolved obligation"),
        "{refusal}"
    );
}
