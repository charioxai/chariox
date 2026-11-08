//! MP-08 / MP-09 / MP-10 / MP-11 A1: one wake carries every row it may deliver.
use super::*;
use tests::Fixture;

const ROOM: &str = "room";
const AGENT: &str = "child";

fn occur(f: &Fixture, id: &str, kind: &str, urgent: bool) -> InboxEvent {
    let mut event = occurrence(ROOM, AGENT, "peer", id, kind, serde_json::json!({"n": id}));
    event.urgent = urgent;
    let Outcome::Event(event) = f.apply(Operation::Occur(event)) else {
        panic!("event")
    };
    event
}

/// A reply request is admitted through a sender's turn, as in production.
fn request_reply(f: &Fixture, id: &str) -> InboxEvent {
    let mut event = occurrence(
        ROOM,
        AGENT,
        "parent",
        id,
        "message",
        serde_json::json!({"message": "return findings"}),
    );
    event.reply_requested = true;
    let Outcome::Event(event) = f.apply(Operation::Send {
        task: "p".into(),
        prompt: "p".into(),
        event,
    }) else {
        panic!("event")
    };
    event
}

fn attempt(
    f: &Fixture,
    lead: &InboxEvent,
    through: u64,
    target: Option<&str>,
    work: Option<&str>,
) -> InboxEvent {
    let Outcome::Event(event) = f.apply(Operation::Attempt {
        room: ROOM.into(),
        agent: AGENT.into(),
        sequence: lead.sequence,
        prompt: format!("agent-event-{AGENT}-{}", lead.sequence),
        target: target.map(str::to_owned),
        run: None,
        now: 10,
        work: work.map(str::to_owned),
        through,
    }) else {
        panic!("event")
    };
    event
}

fn states(f: &Fixture) -> Vec<(u64, String, Option<String>)> {
    f.store
        .agent_inbox(ROOM, AGENT, 0)
        .unwrap()
        .into_iter()
        .map(|e| (e.sequence, e.state, e.prompt_id))
        .collect()
}

fn receipt(f: &Fixture, sequence: u64, state: &str) {
    f.apply(Operation::Receipt {
        room: ROOM.into(),
        agent: AGENT.into(),
        sequence,
        state: state.into(),
        now: 11,
    });
}

#[test]
fn three_pending_events_make_one_attempt_with_one_prompt_and_one_receipt() {
    let f = Fixture::new();
    let rows = [
        occur(&f, "a", "peer_event", false),
        occur(&f, "b", "peer_event", false),
        occur(&f, "c", "peer_event", false),
    ];
    let lead = f
        .store
        .agent_work_delivery_front(ROOM, AGENT, None)
        .unwrap()
        .unwrap();
    assert_eq!(lead.sequence, rows[0].sequence);
    let batch = f.store.agent_delivery_batch(&lead, false, None).unwrap();
    assert_eq!(batch.len(), 2);
    attempt(&f, &lead, batch[1].sequence, None, None);
    let prompt = format!("agent-event-{AGENT}-{}", lead.sequence);
    let delivered = f
        .store
        .agent_events_for_prompt(ROOM, AGENT, &prompt)
        .unwrap();
    assert_eq!(delivered.len(), 3, "the batch is the rows of one prompt");
    assert!(delivered.iter().all(|e| e.state == "submitting"));
    // The lead's receipt is the receipt of every row of the batch.
    receipt(&f, lead.sequence, "accepted");
    assert!(states(&f)
        .iter()
        .all(|(_, state, p)| state == "accepted" && p.as_deref() == Some(prompt.as_str())));
    for row in &rows {
        f.apply(Operation::Ack {
            room: ROOM.into(),
            agent: AGENT.into(),
            sequence: row.sequence,
            handled: true,
            now: 12,
        });
    }
    assert!(states(&f).iter().all(|(_, state, _)| state == "handled"));
}

#[test]
fn a_rejected_receipt_returns_every_row_of_the_batch_to_pending() {
    let f = Fixture::new();
    let first = occur(&f, "a", "peer_event", false);
    let last = occur(&f, "b", "peer_event", false);
    attempt(&f, &first, last.sequence, None, None);
    receipt(&f, first.sequence, "rejected");
    assert!(states(&f)
        .iter()
        .all(|(_, state, p)| state == "pending" && p.is_none()));
}

#[test]
fn an_urgent_event_arriving_while_a_batch_waits_is_delivered_first_and_alone() {
    let f = Fixture::new();
    let early = occur(&f, "a", "peer_event", false);
    let urgent = occur(&f, "b", "peer_event", true);
    let late = occur(&f, "c", "peer_event", false);
    let urgent_late = occur(&f, "d", "peer_event", true);
    let front = f
        .store
        .agent_urgent_delivery_front(ROOM, AGENT)
        .unwrap()
        .unwrap();
    assert_eq!(
        front.sequence, urgent.sequence,
        "urgent passes the waiting rows"
    );
    let steer = f.store.agent_delivery_batch(&front, true, None).unwrap();
    assert_eq!(
        steer.iter().map(|e| e.sequence).collect::<Vec<_>>(),
        vec![urgent_late.sequence],
        "a steer wake covers urgent rows only"
    );
    attempt(&f, &front, urgent_late.sequence, Some("active"), None);
    let by_sequence: std::collections::HashMap<_, _> =
        states(&f).into_iter().map(|(s, st, _)| (s, st)).collect();
    assert_eq!(by_sequence[&urgent.sequence], "submitting");
    assert_eq!(by_sequence[&urgent_late.sequence], "submitting");
    assert_eq!(by_sequence[&early.sequence], "pending");
    assert_eq!(by_sequence[&late.sequence], "pending");
}

#[test]
fn a_reply_request_is_never_in_a_batch() {
    let f = Fixture::new();
    f.begin("p");
    let before = occur(&f, "a", "peer_event", false);
    let reply = request_reply(&f, "r");
    let after = occur(&f, "b", "peer_event", false);
    let batch = f.store.agent_delivery_batch(&before, false, None).unwrap();
    assert!(batch.is_empty(), "the batch stops at a reply request");
    attempt(&f, &before, after.sequence, None, None);
    let by_sequence: std::collections::HashMap<_, _> =
        states(&f).into_iter().map(|(s, st, _)| (s, st)).collect();
    assert_eq!(by_sequence[&before.sequence], "submitting");
    assert_eq!(by_sequence[&reply.sequence], "pending");
    assert_eq!(by_sequence[&after.sequence], "pending");
    receipt(&f, before.sequence, "accepted");
    // The reply request leads its own wake, with nothing behind it.
    assert!(f
        .store
        .agent_delivery_batch(&reply, false, None)
        .unwrap()
        .is_empty());
    attempt(&f, &reply, after.sequence, None, None);
    let by_sequence: std::collections::HashMap<_, _> =
        states(&f).into_iter().map(|(s, st, _)| (s, st)).collect();
    assert_eq!(by_sequence[&reply.sequence], "submitting");
    assert_eq!(by_sequence[&after.sequence], "pending");
}

#[test]
fn a_sudo_bound_batch_holds_only_rows_correlated_to_the_work_task() {
    let f = Fixture::new();
    let work = "work-task";
    let mut correlated = occurrence(
        ROOM,
        AGENT,
        "kernel",
        "w1",
        "timer_fired",
        serde_json::json!({"task_id": work}),
    );
    correlated.urgent = false;
    let Outcome::Event(correlated) = f.apply(Operation::Occur(correlated)) else {
        panic!()
    };
    let unrelated = occur(&f, "u", "peer_event", false);
    let Outcome::Event(also) = f.apply(Operation::Occur(occurrence(
        ROOM,
        AGENT,
        "kernel",
        "w2",
        "timer_fired",
        serde_json::json!({"task_id": work}),
    ))) else {
        panic!()
    };
    let batch = f
        .store
        .agent_delivery_batch(&correlated, false, Some(work))
        .unwrap();
    assert_eq!(
        batch.iter().map(|e| e.sequence).collect::<Vec<_>>(),
        vec![also.sequence]
    );
    attempt(&f, &correlated, also.sequence, None, Some(work));
    let by_sequence: std::collections::HashMap<_, _> =
        states(&f).into_iter().map(|(s, st, _)| (s, st)).collect();
    assert_eq!(by_sequence[&correlated.sequence], "submitting");
    assert_eq!(by_sequence[&also.sequence], "submitting");
    assert_eq!(
        by_sequence[&unrelated.sequence], "pending",
        "unrelated rows stay deferred"
    );
}

#[test]
fn a_restart_between_the_attempt_and_the_receipt_keeps_the_batch_together() {
    let mut f = Fixture::new();
    let first = occur(&f, "a", "peer_event", false);
    occur(&f, "b", "peer_event", false);
    let last = occur(&f, "c", "peer_event", false);
    attempt(&f, &first, last.sequence, None, None);
    let before = f.store.agent_inbox(ROOM, AGENT, 0).unwrap();
    let temporary = DurableKernelStateStore::open_owned(f.root.join("temporary.sqlite")).unwrap();
    drop(std::mem::replace(&mut f.store, temporary));
    f.store = DurableKernelStateStore::open_owned(f.root.join("state.sqlite")).unwrap();
    assert_eq!(f.store.agent_inbox(ROOM, AGENT, 0).unwrap(), before);
    let prompt = format!("agent-event-{AGENT}-{}", first.sequence);
    assert_eq!(
        f.store
            .agent_events_for_prompt(ROOM, AGENT, &prompt)
            .unwrap()
            .len(),
        3
    );
    // The exact receipt, arriving after the restart, reconciles every row.
    receipt(&f, first.sequence, "accepted");
    assert!(states(&f).iter().all(|(_, state, _)| state == "accepted"));
}

/// A message held for a busy agent stays unread by an urgent steer that skips
/// its sequence: the steer's read starts before the oldest row still pending.
#[test]
fn a_steer_reads_from_before_the_oldest_pending_row() {
    let f = Fixture::new();
    let held = occur(&f, "a", "message", false);
    let steer = occur(&f, "b", "message", true);
    assert_eq!(
        f.store.agent_read_cursor(&steer, None).unwrap(),
        held.sequence - 1,
        "the held message is in the steer's read"
    );
    f.apply(Operation::Expire {
        room: ROOM.into(),
        agent: AGENT.into(),
        sequence: held.sequence,
    });
    assert_eq!(
        f.store.agent_read_cursor(&steer, None).unwrap(),
        steer.sequence - 1,
        "a row that is no longer pending is not read again"
    );
}

// MP-08 / MP-09 / MP-10 / MP-11: the read state is durable and belongs to one agent's row only.
#[test]
fn a_shown_row_stays_shown_after_a_restart() {
    let f = Fixture::new();
    let shown = occur(&f, "a", "message", false);
    let unseen = occur(&f, "b", "message", false);
    let other = occurrence(ROOM, "other", "peer", "c", "message", serde_json::json!({}));
    let Outcome::Event(other) = f.apply(Operation::Occur(other)) else {
        panic!("event")
    };
    f.apply(Operation::MarkShown {
        room: ROOM.into(),
        agent: AGENT.into(),
        // A sequence of another agent's row is not marked through this agent.
        sequences: vec![shown.sequence, other.sequence],
    });
    let sequences = |store: &DurableKernelStateStore, agent: &str| -> Vec<u64> {
        store
            .agent_unread_inbox(ROOM, agent, 0, u64::MAX >> 1, None)
            .unwrap()
            .into_iter()
            .map(|e| e.sequence)
            .collect()
    };
    assert_eq!(sequences(&f.store, AGENT), vec![unseen.sequence]);
    assert_eq!(sequences(&f.store, "other"), vec![other.sequence]);
    let reopened = DurableKernelStateStore::open(f.root.join("state.sqlite")).unwrap();
    assert_eq!(sequences(&reopened, AGENT), vec![unseen.sequence]);
    assert_eq!(sequences(&reopened, "other"), vec![other.sequence]);
}

// MP-08 / MP-09 / MP-10 / MP-11: admission reads the recipient's rows, not every sequence between
// them. Another agent filled the global sequence between the two pending rows.
#[test]
fn a_batch_is_admitted_across_a_large_gap_of_other_recipients_rows() {
    const GAP: u64 = 300_000;
    let f = Fixture::new();
    let first = occur(&f, "a", "peer_event", false);
    let mut db = Connection::open(f.root.join("state.sqlite")).unwrap();
    let tx = db.transaction().unwrap();
    {
        let mut insert = tx
            .prepare("INSERT INTO agent_inbox(room_id,agent_id,source_id,occurrence_id,payload) VALUES(?1,'other','peer',?2,'{}')")
            .unwrap();
        for n in 0..GAP {
            insert.execute(params![ROOM, n.to_string()]).unwrap();
        }
    }
    tx.commit().unwrap();
    drop(db);
    let second = occur(&f, "b", "peer_event", false);
    assert!(second.sequence - first.sequence > GAP);
    let lead = f
        .store
        .agent_work_delivery_front(ROOM, AGENT, None)
        .unwrap()
        .unwrap();
    assert_eq!(lead.sequence, first.sequence);
    let batch = f.store.agent_delivery_batch(&lead, false, None).unwrap();
    assert_eq!(
        batch.iter().map(|e| e.sequence).collect::<Vec<_>>(),
        vec![second.sequence]
    );
    let started = std::time::Instant::now();
    attempt(&f, &lead, second.sequence, None, None);
    let elapsed = started.elapsed();
    let prompt = format!("agent-event-{AGENT}-{}", lead.sequence);
    let delivered = f
        .store
        .agent_events_for_prompt(ROOM, AGENT, &prompt)
        .unwrap();
    assert_eq!(
        delivered.iter().map(|e| e.sequence).collect::<Vec<_>>(),
        vec![first.sequence, second.sequence]
    );
    // One statement over the recipient's rows takes milliseconds; a lookup per sequence in the
    // gap takes seconds.
    assert!(
        elapsed < std::time::Duration::from_millis(250),
        "admission took {elapsed:?} for a gap of {GAP} sequences"
    );
}
