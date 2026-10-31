use super::*;

const RUN: &str = "run-1";

fn context() -> HistoryEventTurnContext {
    HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("prompt-1".to_string()),
        prompt_id: Some("prompt-1".to_string()),
        provider_run_id: Some(RUN.to_string()),
        ..HistoryEventTurnContext::default()
    }
}

fn prompt(sequence: u64, timestamp_ms: u64) -> HistoryEvent {
    let mut event = HistoryEvent::transcript(
        sequence,
        &SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "write a story"),
        context(),
    );
    event.timestamp_ms = timestamp_ms;
    event
}

fn fragment(sequence: u64, timestamp_ms: u64, text: &str) -> HistoryEvent {
    let mut event = HistoryEvent::transcript(sequence, &fragment_entry(text), context());
    event.timestamp_ms = timestamp_ms;
    event
}

fn fragment_entry(text: &str) -> SessionHistoryEntry {
    SessionHistoryEntry::provider_output(
        "session-1",
        RUN,
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        Some("message-1".to_string()),
        text,
    )
}

fn store_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "chariox-incremental-outline-{}-{}.db",
        std::process::id(),
        rand::random::<u64>()
    ))
}

fn outline(store: &OperationalHistoryStore, latest_prompt_count: usize) -> serde_json::Value {
    serde_json::to_value(
        load_agent_outline(store, "session-1", "agent-1", latest_prompt_count, None)
            .expect("outline should load"),
    )
    .expect("outline should serialize")
}

/// The outline a store that has read nothing before builds from the same file.
fn full_outline(path: &std::path::Path, latest_prompt_count: usize) -> serde_json::Value {
    let fresh = OperationalHistoryStore::open(path.to_path_buf()).expect("reopen history");
    outline(&fresh, latest_prompt_count)
}

#[test]
fn a_second_outline_read_of_a_growing_reply_decodes_only_the_new_rows() {
    let path = store_path();
    let store = OperationalHistoryStore::open(path.clone()).expect("open history");
    let events = std::iter::once(prompt(10, 5_000))
        .chain((0..200).map(|index| fragment(11 + index, 5_001 + index, "streamed words ")))
        .collect::<Vec<_>>();
    store.append_many(&events).expect("append the reply so far");
    outline(&store, 1);
    assert_eq!(store.rows_decoded(), 201, "the first read loads the turn");

    let more = (200..205)
        .map(|index| fragment(11 + index, 5_001 + index, "more words "))
        .collect::<Vec<_>>();
    store.append_many(&more).expect("append five fragments");
    let before = store.rows_decoded();
    let grown = outline(&store, 1);

    assert_eq!(
        store.rows_decoded() - before,
        5,
        "the second read must not load the first part of the reply again"
    );
    assert!(
        grown == full_outline(&path, 1),
        "the incremental outline differs from the full one"
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn an_incremental_outline_equals_the_full_one_after_every_change() {
    let path = store_path();
    let store = OperationalHistoryStore::open(path.clone()).expect("open history");
    let check = |step: &str| {
        for count in [1, 3] {
            assert!(
                outline(&store, count) == full_outline(&path, count),
                "outline of the latest {count} prompts differs after {step}"
            );
        }
    };
    // The first three fragments are old enough to be pruned later.
    let mut events = vec![prompt(10, 5_000)];
    events.extend((0..3).map(|index| fragment(11 + index, 100 + index, "early ")));
    store.append_many(&events).expect("append");
    check("the first fragments");

    for index in 3..40 {
        store
            .append(&fragment(11 + index, 5_001 + index, "word "))
            .expect("append a fragment");
        check(&format!("fragment {index}"));
    }

    // A producer reserved sequence 100 before the one after it, and its row commits last.
    store
        .append(&fragment(102, 5_200, "late "))
        .expect("append");
    check("a later sequence");
    store
        .append(&fragment(100, 5_199, "behind "))
        .expect("append behind");
    check("a row committed behind a later sequence");

    let mut second = prompt(200, 6_000);
    second.prompt_id = Some("prompt-2".to_string());
    second.turn_id = Some("prompt-2".to_string());
    store.append(&second).expect("append a newer prompt");
    check("a newer prompt");

    store
        .replace_transcript_by_merge_key(
            "session-1",
            Some("agent-1"),
            "message-1",
            &fragment_entry("rewritten "),
            context(),
        )
        .expect("rewrite the first fragment");
    check("a rewrite");

    store
        .prune_events_before(1_000, true)
        .expect("prune the oldest rows");
    check("a prune");
    let _ = std::fs::remove_file(path);
}
