//! A message read inside another task receives an outcome without borrowing that task's answer.
use super::*;
use tests::Fixture;

fn message(from: &str, to: &str, key: &str, text: &str, reply_requested: bool) -> InboxEvent {
    let mut event = occurrence(
        "room",
        to,
        from,
        key,
        "message",
        serde_json::json!({"message": text, "sender_room_id": "room"}),
    );
    event.reply_requested = reply_requested;
    event
}

/// The parent asks peer; peer is busy in its own task `pt`.
fn ask(f: &Fixture) -> InboxEvent {
    f.begin("p");
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "room".into(),
        agent: "peer".into(),
        prompt: "pt".into(),
        run: None,
        now: 1,
    });
    let Outcome::Event(question) = f.apply(Operation::Send {
        task: "p".into(),
        prompt: "p".into(),
        event: message("parent", "peer", "q", "Are you covering PCD-04?", true),
    }) else {
        panic!("question admitted")
    };
    question
}

fn send_update(f: &Fixture) -> InboxEvent {
    let Outcome::Event(reply) = f.apply(Operation::Send {
        task: "pt".into(),
        prompt: "pt".into(),
        event: message(
            "peer",
            "parent",
            "a",
            "Yes, both, with geometry samples.",
            false,
        ),
    }) else {
        panic!("answer admitted")
    };
    reply
}

fn reply_obligation(f: &Fixture) -> AgentObligation {
    f.task()
        .obligations
        .into_iter()
        .find(|o| o.kind == "reply")
        .expect("the question holds a reply obligation")
}

/// A separate message cannot establish a reply while the exact delivery is unresolved.
#[test]
fn a_separate_message_does_not_settle_an_in_flight_request() {
    for state in ["submitting", "uncertain"] {
        let f = Fixture::new();
        let question = ask(&f);
        f.apply(Operation::Attempt {
            room: "room".into(),
            agent: "peer".into(),
            sequence: question.sequence,
            prompt: "pt".into(),
            target: Some("pt".into()),
            run: None,
            now: 2,
            work: None,
            through: 0,
        });
        if state == "uncertain" {
            f.apply(Operation::Receipt {
                room: "room".into(),
                agent: "peer".into(),
                sequence: question.sequence,
                state: "uncertain".into(),
                now: 3,
            });
        }
        send_update(&f);
        assert_eq!(
            reply_obligation(&f).status,
            "open",
            "delivery {state}: {:?}",
            reply_obligation(&f)
        );
    }
}

#[test]
fn a_message_before_the_question_was_read_is_not_its_reply() {
    let f = Fixture::new();
    ask(&f);
    send_update(&f);
    assert_eq!(reply_obligation(&f).status, "open");
}

fn read(f: &Fixture, question: &InboxEvent) {
    f.apply(Operation::MarkShown {
        room: "room".into(),
        agent: "peer".into(),
        sequences: vec![question.sequence],
    });
}

fn ack_question(f: &Fixture, question: &InboxEvent, handled: bool) {
    f.apply(Operation::Ack {
        room: "room".into(),
        agent: "peer".into(),
        sequence: question.sequence,
        handled,
        now: 5,
    });
}

/// The peer's task `pt` ends the way the runtime records it, on the task's own source.
fn peer_task_ends(f: &Fixture, success: bool, answer: Option<serde_json::Value>) {
    f.apply(Operation::SourceOutcome {
        room: "room".into(),
        source: "pt".into(),
        occurrence: "task-terminal-pt".into(),
        success,
        public_answer: answer,
        now: 9,
    });
}

#[test]
fn a_separate_message_does_not_settle_an_acknowledged_request() {
    for handled in [false, true] {
        let f = Fixture::new();
        let question = ask(&f);
        read(&f, &question);
        ack_question(&f, &question, handled);
        send_update(&f);
        assert_eq!(
            reply_obligation(&f).status,
            "open",
            "acked with handled={handled}: {:?}",
            reply_obligation(&f)
        );
    }
}

#[test]
fn an_asker_waiting_on_the_reply_and_another_source_resumes_when_both_end() {
    let f = Fixture::new();
    let question = ask(&f);
    f.apply(Operation::RegisterObligation {
        owner: "owner".into(),
        room: "room".into(),
        agent: "parent".into(),
        prompt: "p".into(),
        run: Some("run".into()),
        id: "build".into(),
        kind: "delegate".into(),
        resource: Some("build".into()),
        now: 1,
    });
    f.apply(Operation::Subscribe {
        task: "p".into(),
        prompt: "p".into(),
        registration: Registration {
            id: "build-reg".into(),
            task_id: "p".into(),
            source_id: "build".into(),
            obligation_id: Some("build".into()),
            source_cursor: 0,
            live: true,
            peer: false,
        },
    });
    let reply_registration = format!("completion-reply-{}", question.sequence);
    f.apply(Operation::Yield {
        task: "p".into(),
        prompt: "p".into(),
        registrations: vec![reply_registration.clone(), "build-reg".into()],
        cursor: 0,
        deadline: Some(60_000),
        reason: "the answer and the build".into(),
        now: 2,
    });
    f.apply(Operation::Settle {
        room: "room".into(),
        agent: "parent".into(),
        prompt: "p".into(),
        run: "run".into(),
        has_answer: true,
        cancelled: false,
        now: 3,
    });
    assert_eq!(f.task().state, ExecutionState::Waiting);
    read(&f, &question);
    ack_question(&f, &question, true);
    peer_task_ends(&f, true, Some(serde_json::json!({"excerpt": "unrelated"})));
    f.apply(Operation::SourceOutcome {
        room: "room".into(),
        source: "build".into(),
        occurrence: "exit".into(),
        success: true,
        public_answer: Some(serde_json::json!({"exit_code": 0})),
        now: 4,
    });
    let task = f.task();
    assert!(
        task.obligations.iter().all(|o| o.status != "open"),
        "{:?}",
        task.obligations
    );
    let events = f.store.agent_inbox("room", "parent", 0).unwrap();
    assert!(
        events.iter().any(|e| e.kind == "source_lost"
            && e.source_id == format!("agent-event-peer-{}", question.sequence)),
        "the asker hears the reply: {events:?}"
    );
    assert!(
        events.iter().any(|e| e.source_id == "build"),
        "the asker hears the build: {events:?}"
    );
}

#[test]
fn a_recipient_task_cancelled_after_reading_the_question_ends_the_askers_reply() {
    let f = Fixture::new();
    let question = ask(&f);
    read(&f, &question);
    ack_question(&f, &question, false);
    peer_task_ends(&f, false, None);
    assert_eq!(
        reply_obligation(&f).status,
        "failed",
        "{:?}",
        reply_obligation(&f)
    );
    let told = f
        .store
        .agent_inbox("room", "parent", 0)
        .unwrap()
        .into_iter()
        .find(|e| e.kind == "source_lost")
        .expect("the asker hears that its recipient is gone");
    assert_eq!(
        told.payload["public_answer"],
        DelegateGoneReason::SourceLost.answer("peer")
    );
}

#[test]
fn a_recipient_finishing_other_work_returns_no_answer_to_the_request() {
    let f = Fixture::new();
    let question = ask(&f);
    read(&f, &question);
    ack_question(&f, &question, true);
    peer_task_ends(
        &f,
        true,
        Some(serde_json::json!({"excerpt": "done, covered"})),
    );
    assert_ne!(
        reply_obligation(&f).status,
        "open",
        "{:?}",
        reply_obligation(&f)
    );
    let told = f
        .store
        .agent_inbox("room", "parent", 0)
        .unwrap()
        .into_iter()
        .find(|e| e.kind == "source_lost")
        .expect("the asker hears why the request has no answer");
    assert_eq!(
        told.payload["public_answer"],
        ReplyOutcome::no_answer("peer")
    );
}

#[test]
fn requests_read_during_the_same_turn_settle_once_each_after_reopening() {
    let mut f = Fixture::new();
    let first = ask(&f);
    let Outcome::Event(second) = f.apply(Operation::Send {
        task: "p".into(),
        prompt: "p".into(),
        event: message("parent", "peer", "q2", "Where is the report?", true),
    }) else {
        panic!("second question admitted")
    };
    for request in [&first, &second] {
        read(&f, request);
        ack_question(&f, request, true);
    }
    assert!(f.store.agent_inbox("room", "parent", 0).unwrap().is_empty());

    let temporary = DurableKernelStateStore::open_owned(f.root.join("temporary.sqlite")).unwrap();
    drop(std::mem::replace(&mut f.store, temporary));
    f.store = DurableKernelStateStore::open_owned(f.root.join("state.sqlite")).unwrap();
    for _ in 0..2 {
        peer_task_ends(&f, true, Some(serde_json::json!({"excerpt": "unrelated"})));
    }
    let results = f.store.agent_inbox("room", "parent", 0).unwrap();
    assert_eq!(results.len(), 2, "{results:?}");
    for request in [first, second] {
        let source = format!("agent-event-peer-{}", request.sequence);
        let result = results
            .iter()
            .find(|event| event.source_id == source)
            .unwrap();
        assert_eq!(result.kind, "source_lost");
        assert_eq!(
            result.payload["public_answer"],
            ReplyOutcome::no_answer("peer")
        );
    }
}

#[test]
fn a_recipient_deleted_after_reading_the_question_ends_the_askers_reply() {
    let f = Fixture::new();
    let question = ask(&f);
    read(&f, &question);
    ack_question(&f, &question, true);
    f.apply(Operation::DelegateUnavailable {
        room: "room".into(),
        agent: "peer".into(),
        reason: DelegateGoneReason::Deleted,
        now: 9,
    });
    assert_eq!(
        reply_obligation(&f).status,
        "failed",
        "{:?}",
        reply_obligation(&f)
    );
}

#[test]
fn a_question_read_but_not_acknowledged_stays_deliverable_after_its_task_ends() {
    let f = Fixture::new();
    let question = ask(&f);
    read(&f, &question);
    peer_task_ends(&f, true, Some(serde_json::json!({"excerpt": "unrelated"})));
    assert_eq!(
        reply_obligation(&f).status,
        "open",
        "an unacknowledged question is still owed its own turn"
    );
}

#[test]
fn a_message_naming_the_request_delivers_its_answer_to_the_waiting_sender() {
    let f = Fixture::new();
    let question = ask(&f);
    read(&f, &question);
    ack_question(&f, &question, true);
    let response = format!("Reply to message {}: Yes, both.", question.sequence);

    f.apply(Operation::Send {
        task: "pt".into(),
        prompt: "pt".into(),
        event: message("peer", "parent", "answer", &response, false),
    });

    assert_ne!(reply_obligation(&f).status, "open");
    let inbox = f.store.agent_inbox("room", "parent", 0).unwrap();
    let completion = inbox
        .iter()
        .find(|event| event.kind == "source_completed")
        .expect("the sender task receives the requested result");
    assert_eq!(completion.payload["public_answer"]["excerpt"], "Yes, both.");
    assert!(inbox
        .iter()
        .any(|event| event.payload["message"] == response));
}

#[test]
fn only_the_sender_can_withdraw_a_reply_request_and_retries_leave_it_settled() {
    let f = Fixture::new();
    let question = ask(&f);
    let withdraw = |agent: &str| Operation::WithdrawReply {
        room: "room".into(),
        agent: agent.into(),
        sequence: question.sequence,
        now: 10,
    };
    assert!(f.store.agent_lifecycle(withdraw("peer")).is_err());
    assert_eq!(reply_obligation(&f).status, "open");
    f.apply(withdraw("parent"));
    f.apply(withdraw("parent"));

    assert_ne!(reply_obligation(&f).status, "open");
    assert_eq!(
        f.store
            .agent_inbox_event(question.sequence)
            .unwrap()
            .unwrap()
            .0
            .state,
        "expired"
    );
    let inbox = f.store.agent_inbox("room", "parent", 0).unwrap();
    assert_eq!(
        inbox
            .iter()
            .filter(|event| event.payload["public_answer"]["kind"] == "reply_withdrawn")
            .count(),
        1
    );
}
