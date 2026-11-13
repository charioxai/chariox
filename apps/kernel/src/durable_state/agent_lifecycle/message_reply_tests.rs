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
