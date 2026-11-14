//! A reply_requested question that its recipient read inside a running turn is answered by the
//! recipient's next message to the asker: that message settles the asker's reply obligation.
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

fn answer(f: &Fixture) -> InboxEvent {
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

#[test]
fn a_message_answering_a_question_read_in_turn_settles_the_askers_reply() {
    let f = Fixture::new();
    let question = ask(&f);
    // Peer reads the question from its inbox inside its running task and answers before it acks.
    f.apply(Operation::MarkShown {
        room: "room".into(),
        agent: "peer".into(),
        sequences: vec![question.sequence],
    });
    let reply = answer(&f);
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
        .find(|e| e.kind == "source_completed")
        .expect("the asker hears its reply arrived");
    assert_eq!(
        told.payload["public_answer"]["reply_sequence"],
        reply.sequence
    );
    assert_eq!(
        told.payload["public_answer"]["excerpt"],
        "Yes, both, with geometry samples."
    );
    let asked = f
        .store
        .agent_inbox("room", "peer", 0)
        .unwrap()
        .into_iter()
        .find(|e| e.sequence == question.sequence)
        .unwrap();
    assert_eq!(
        asked.state, "handled",
        "the answered question is not delivered again"
    );
}

#[test]
fn a_message_before_the_question_was_read_is_not_its_reply() {
    let f = Fixture::new();
    ask(&f);
    answer(&f);
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
fn a_message_answering_an_acknowledged_question_settles_the_askers_reply() {
    for handled in [false, true] {
        let f = Fixture::new();
        let question = ask(&f);
        read(&f, &question);
        ack_question(&f, &question, handled);
        answer(&f);
        assert_ne!(
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
    // The peer reads the question inside its turn and answers by message, then the build ends.
    read(&f, &question);
    answer(&f);
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
        events.iter().any(|e| e.kind == "source_completed"
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
fn a_recipient_task_ending_without_an_answer_after_reading_the_question_answers_it() {
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
        .find(|e| e.kind == "source_completed")
        .expect("the asker hears its reply arrived");
    assert_eq!(told.payload["public_answer"]["excerpt"], "done, covered");
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
