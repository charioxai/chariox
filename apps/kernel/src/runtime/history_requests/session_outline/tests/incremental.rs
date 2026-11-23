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

/// A store file and the directory that removes it, with its journal files.
fn store_path() -> (crate::test_support::TestTempDir, std::path::PathBuf) {
    let dir = crate::test_support::TestTempDir::new("incremental-outline");
    let path = dir.join("outline.db");
    (dir, path)
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
    let (_dir, path) = store_path();
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
}

#[test]
fn an_outline_read_builds_one_entry_per_streamed_fragment() {
    let builds_for = |fragments: u64| {
        let (_dir, path) = store_path();
        let store = OperationalHistoryStore::open(path).expect("open history");
        let events = std::iter::once(prompt(10, 5_000))
            .chain(
                (0..fragments).map(|index| fragment(11 + index, 5_001 + index, "streamed words ")),
            )
            .collect::<Vec<_>>();
        store.append_many(&events).expect("append the reply");
        let builds =
            || crate::history::session_history_entry_builds::BUILDS.with(|count| count.get());
        let before = builds();
        outline(&store, 1);
        builds() - before
    };

    let added = builds_for(300) - builds_for(100);
    assert!(
        added <= 200,
        "200 more fragments built {added} more entries; checks must read fields"
    );
}

#[test]
fn an_incremental_outline_equals_the_full_one_after_every_change() {
    let (_dir, path) = store_path();
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
}

#[test]
fn an_outline_read_after_the_database_was_reclaimed_loads_the_range_again() {
    let (_dir, path) = store_path();
    let store = OperationalHistoryStore::open(path.clone()).expect("open history");
    // Rows of another agent come first and go: the reply's row ids start after a gap.
    let filler = (0..300)
        .map(|index| {
            let mut event = fragment(1_000 + index, 100 + index, "filler ");
            event.agent_id = Some("agent-2".to_string());
            event
        })
        .collect::<Vec<_>>();
    store.append_many(&filler).expect("append the filler");
    let reply = std::iter::once(prompt(10, 5_000))
        .chain((0..40).map(|index| fragment(11 + index, 5_001 + index, "streamed words ")))
        .collect::<Vec<_>>();
    store.append_many(&reply).expect("append the reply so far");
    store
        .prune_events_before(1_000, true)
        .expect("prune the filler");
    // Warmed after the prune, as an outline can be between the prune and the vacuum.
    outline(&store, 1);
    assert_eq!(store.rows_decoded(), 41, "the first read loads the reply");

    // VACUUM may renumber the row ids the cache's frontier counts on.
    store.reclaim_disk_space().expect("reclaim the database");
    let more = (40..45)
        .map(|index| fragment(11 + index, 5_001 + index, "more words "))
        .collect::<Vec<_>>();
    store.append_many(&more).expect("append five fragments");
    let before = store.rows_decoded();
    let grown = outline(&store, 1);

    assert_eq!(
        store.rows_decoded() - before,
        46,
        "the read after a reclaim must load the whole range, not continue from a remembered row id"
    );
    assert!(
        grown == full_outline(&path, 1),
        "the cached outline differs from the full one after the database was reclaimed"
    );
}

async fn read_delta_page(
    store: &OperationalHistoryStore,
    cursors: serde_json::Value,
) -> serde_json::Value {
    let request = serde_json::from_value(serde_json::json!({
        "session_id": "session-1",
        "agent_ids": ["agent-1"],
        "after_sequence": cursors,
    }))
    .unwrap();
    let response = execute_session_history_outline_request(store.clone(), request)
        .await
        .unwrap();
    serde_json::to_value(response).unwrap()["SessionHistoryOutline"]["agents"][0].clone()
}

#[tokio::test]
async fn outline_delta_sends_replacements_and_recovers_from_a_stale_cursor() {
    let (_dir, path) = store_path();
    let store = OperationalHistoryStore::open(path).expect("open history");
    store
        .append_many(&[prompt(10, 5_000), fragment(11, 5_001, "working")])
        .unwrap();

    let full = read_delta_page(&store, serde_json::json!({})).await;
    assert_eq!(full["turns"][0]["summary"]["entry"]["text"], "working");
    let before = &full["delta_cursor"];
    assert_eq!(before["after_sequence"], 11);

    let mut settled = fragment_entry("settled");
    settled.timestamp_ms = 5_001;
    let replacement = store
        .replace_transcript_by_merge_key(
            "session-1",
            Some("agent-1"),
            "message-1",
            &settled,
            context(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(replacement.sequence, 11);
    let delta = read_delta_page(&store, serde_json::json!({"agent-1": before})).await;
    assert_eq!(delta["turns"], serde_json::json!([]));
    assert!(delta["turn_deltas"][0]["header"].is_number());
    assert_eq!(
        delta["turn_deltas"][0]["summary"]["entry"]["text"],
        "settled"
    );
    assert_eq!(delta["delta_cursor"]["after_sequence"], 11);

    let stale = read_delta_page(
        &store,
        serde_json::json!({
            "agent-1": {"after_sequence": 11, "revision": "expired"}
        }),
    )
    .await;
    assert!(stale.get("turn_deltas").is_none());
    assert_eq!(stale["turns"][0]["summary"]["entry"]["text"], "settled");

    let unchanged = read_delta_page(
        &store,
        serde_json::json!({
            "agent-1": stale["delta_cursor"]
        }),
    )
    .await;
    assert!(unchanged["turn_deltas"][0]["summary"].is_number());

    store.append(&fragment(12, 5_002, " now")).unwrap();
    let grown = read_delta_page(
        &store,
        serde_json::json!({
            "agent-1": stale["delta_cursor"]
        }),
    )
    .await;
    assert_eq!(grown["delta_cursor"]["after_sequence"], 12);
    assert_eq!(
        grown["turn_deltas"][0]["summary"]["suffix"]["entry"]["text"],
        " now"
    );
}

#[tokio::test]
async fn outline_delta_includes_a_blob_whose_display_metadata_did_not_change() {
    let (_dir, path) = store_path();
    let store = OperationalHistoryStore::open(path).expect("open history");
    let tool = |output| {
        let mut entry = SessionHistoryEntry::provider_output(
            "session-1",
            RUN,
            Some("agent-1"),
            TerminalOutputKind::ProviderTool,
            Some("call-1".to_string()),
            format!(r#"{{"id":"call-1","tool":"bash","status":"completed","output":"{output}"}}"#),
        );
        entry.timestamp_ms = 5_001;
        entry
    };
    store
        .append_many(&[
            prompt(10, 5_000),
            HistoryEvent::transcript(11, &tool("yes"), context()),
        ])
        .unwrap();
    let full = read_delta_page(&store, serde_json::json!({})).await;
    let original = &full["turns"][0]["blobs"][0];
    assert!(!original.is_null());
    store
        .replace_transcript_by_merge_key(
            "session-1",
            Some("agent-1"),
            "call-1",
            &tool("new"),
            context(),
        )
        .unwrap()
        .expect("tool replaced in place");
    let delta = read_delta_page(
        &store,
        serde_json::json!({ "agent-1": full["delta_cursor"] }),
    )
    .await;
    let updated = &delta["turn_deltas"][0]["blobs"][0];
    assert_eq!(updated["blob_id"], original["blob_id"]);
    assert_eq!(updated["preview"], original["preview"]);
    assert_eq!(updated["total_chars"], original["total_chars"]);
    assert_ne!(updated["revision"], original["revision"]);
}
