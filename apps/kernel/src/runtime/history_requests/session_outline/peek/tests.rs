use super::*;
use crate::history::{HistoryEventTurnContext, SessionHistoryEntry};
use crate::terminal::TerminalOutputKind;

struct Store {
    path: std::path::PathBuf,
    store: OperationalHistoryStore,
}

impl Store {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-peek-{name}-{}-{}.db",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let store = OperationalHistoryStore::open(path.clone()).expect("store should open");
        Self { path, store }
    }

    fn context(turn: u64) -> HistoryEventTurnContext {
        HistoryEventTurnContext {
            session_id: Some("room".into()),
            agent_id: Some("agent".into()),
            turn_id: Some(format!("turn-{turn}")),
            prompt_id: Some(format!("prompt-{turn}")),
            provider_run_id: Some("run".into()),
            ..HistoryEventTurnContext::default()
        }
    }

    fn prompt(&self, sequence: u64, turn: u64, text: &str) {
        let entry = SessionHistoryEntry::user_prompt("room", "attachment", "agent", text);
        self.append(sequence, entry, turn);
    }

    fn output(&self, sequence: u64, turn: u64, kind: TerminalOutputKind, key: &str, text: String) {
        let entry = SessionHistoryEntry::provider_output(
            "room",
            "run",
            Some("agent"),
            kind,
            Some(key.into()),
            text,
        );
        self.append(sequence, entry, turn);
    }

    fn tool(&self, sequence: u64, turn: u64, value: serde_json::Value) {
        let key = value["id"].as_str().unwrap().to_string();
        self.output(
            sequence,
            turn,
            TerminalOutputKind::ProviderTool,
            &key,
            value.to_string(),
        );
    }

    fn append(&self, sequence: u64, mut entry: SessionHistoryEntry, turn: u64) {
        entry.timestamp_ms = 1_000_000 + sequence * 1_000;
        self.store
            .append(&HistoryEvent::transcript(
                sequence,
                &entry,
                Self::context(turn),
            ))
            .expect("event should append");
    }

    /// A turn of four rows: its prompt, a thought, a command, and the answer.
    fn turn(&self, turn: u64) {
        let base = turn * 10;
        self.prompt(base, turn, &format!("task {turn}"));
        self.output(
            base + 1,
            turn,
            TerminalOutputKind::ProviderReasoning,
            &format!("thought-{turn}"),
            format!("**Planning step {turn}**\nthen act"),
        );
        self.tool(
            base + 2,
            turn,
            serde_json::json!({"id":format!("call-{turn}"),"tool":"bash","status":"completed",
                "input":{"command":format!("ls dir{turn}")},"output":"a\nb\n",
                "started_at_ms":1_000,"finished_at_ms":1_250}),
        );
        self.output(
            base + 3,
            turn,
            TerminalOutputKind::ProviderOutput,
            &format!("answer-{turn}"),
            format!("Done with {turn}"),
        );
    }

    fn peek(&self, limit: usize, before: Option<String>, now_ms: u64) -> AgentTranscriptPeek {
        peek(
            &self.store,
            &PeekAgentTranscriptRequest {
                session_id: "room".into(),
                agent_id: "agent".into(),
                limit: Some(limit),
                before,
            },
            None,
            now_ms,
            &str::to_owned,
        )
        .expect("peek should succeed")
    }

    fn read(&self, entry_id: &str, offset: usize, limit: usize) -> AgentTranscriptEntryPage {
        read_entry(
            &self.store,
            &ReadAgentTranscriptEntryRequest {
                session_id: "room".into(),
                agent_id: "agent".into(),
                entry_id: entry_id.into(),
                offset,
                limit: Some(limit),
            },
            None,
            &str::to_owned,
        )
        .expect("read should succeed")
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        for path in [
            self.path.clone(),
            self.path.with_extension("db-wal"),
            self.path.with_extension("db-shm"),
        ] {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[test]
fn peek_lists_the_latest_entries_as_small_titled_rows() {
    // MP-08 / MP-10 / MP-11 PEEK-01/08: kinds, pane titles and times, under 4 KB for ten.
    let store = Store::new("latest");
    for turn in 1..=5 {
        store.turn(turn);
    }
    let peek = store.peek(10, None, 2_000_000);
    let kinds = peek.entries.iter().map(|e| e.kind).collect::<Vec<_>>();
    let turn = [
        Kind::Prompt,
        Kind::Thinking,
        Kind::ToolCall,
        Kind::Assistant,
    ];
    let expected = [&turn[2..], &turn[..], &turn[..]].concat();
    assert_eq!(kinds, expected);
    let titles = peek
        .entries
        .iter()
        .map(|e| e.title.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        titles,
        [
            "Listed dir3",
            "Done with 3",
            "task 4",
            "Thought Planning step 4",
            "Listed dir4",
            "Done with 4",
            "task 5",
            "Thought Planning step 5",
            "Listed dir5",
            "Done with 5",
        ]
    );
    let tool = &peek.entries[8];
    assert_eq!(tool.at_ms, 1_000);
    assert_eq!(tool.duration_ms, Some(250));
    assert_eq!(tool.status, Status::Completed);
    let prompt = &peek.entries[6];
    assert_eq!(prompt.at_ms, 1_050_000);
    assert!(peek.before.is_some());
    let wire = serde_json::to_string(&LocalDaemonResponse::AgentTranscriptPeek { peek }).unwrap();
    assert!(wire.len() < 4 * 1024, "peek of ten is {} bytes", wire.len());
}

#[test]
fn a_running_call_shows_its_elapsed_time_then_its_final_duration() {
    // MP-08 / MP-10 / MP-11 PEEK-02.
    let store = Store::new("running");
    store.prompt(10, 1, "build it");
    store.tool(
        11,
        1,
        serde_json::json!({"id":"call","tool":"bash","status":"running",
            "input":{"command":"cargo build"},"started_at_ms":5_000}),
    );
    let running = store.peek(10, None, 65_000);
    let call = running.entries.last().unwrap();
    assert_eq!(
        (
            call.kind,
            call.status,
            call.duration_ms,
            call.title.as_str()
        ),
        (
            Kind::ToolCall,
            Status::Running,
            Some(60_000),
            "Running cargo build"
        )
    );
    assert_eq!(running.entries[0].status, Status::Running);
    store.tool(
        12,
        1,
        serde_json::json!({"id":"call","tool":"bash","status":"failed",
            "input":{"command":"cargo build"},"output":"error","started_at_ms":5_000,"finished_at_ms":95_000}),
    );
    let ended = store.peek(10, None, 500_000);
    let call = ended.entries.last().unwrap();
    assert_eq!(
        (call.status, call.duration_ms, call.title.as_str()),
        (Status::Failed, Some(90_000), "Ran cargo build")
    );
    assert_eq!(
        ended.entries.len(),
        2,
        "one row per call: {:?}",
        ended.entries
    );
}

#[test]
fn an_entry_reads_by_pages_that_add_up_to_its_content() {
    // MP-08 / MP-10 / MP-11 PEEK-03.
    let store = Store::new("pages");
    store.prompt(10, 1, "print");
    let output = (0..400)
        .map(|line| format!("line {line} ✓\n"))
        .collect::<String>();
    store.tool(
        11,
        1,
        serde_json::json!({"id":"call","tool":"bash","status":"completed",
            "input":{"command":"seq 400"},"output":output}),
    );
    let peek = store.peek(10, None, 2_000_000);
    let call = peek.entries.last().unwrap();
    let mut offset = 0;
    let mut joined = String::new();
    let mut pages = 0;
    loop {
        let page = store.read(&call.entry_id, offset, 1_000);
        assert!(page.text.chars().count() <= 1_000);
        assert_eq!(page.kind, Kind::ToolCall);
        joined.push_str(&page.text);
        pages += 1;
        match page.next_offset {
            Some(next) => offset = next,
            None => break,
        }
    }
    let whole = store.read(&call.entry_id, 0, MAX_AGENT_PEEK_PAGE_CHARS);
    assert_eq!(joined, whole.text);
    assert!(pages > 1);
    assert!(
        joined.contains(&output),
        "the output reads as the tool printed it"
    );
    assert!(joined.starts_with("input: {\"command\":\"seq 400\"}\n"));
}

#[test]
fn the_cursor_pages_back_through_every_entry_once() {
    // MP-08 / MP-10 / MP-11 PEEK-04: 50 turns of 4 rows.
    let store = Store::new("cursor");
    for turn in 1..=50 {
        store.turn(turn);
    }
    let mut before = None;
    let mut seen = Vec::new();
    loop {
        let page = store.peek(7, before.take(), 2_000_000);
        assert!(page.entries.len() <= 7);
        let mut ids = page
            .entries
            .iter()
            .map(|e| e.entry_id.clone())
            .collect::<Vec<_>>();
        ids.append(&mut seen);
        seen = ids;
        match page.before {
            Some(cursor) => before = Some(cursor),
            None => break,
        }
    }
    assert_eq!(seen.len(), 200);
    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 200, "no entry twice");
    let starts = seen
        .iter()
        .map(|id| parse_blob_id(id).unwrap().0)
        .collect::<Vec<_>>();
    assert!(
        starts.windows(2).all(|pair| pair[0] < pair[1]),
        "oldest first, no gaps in order"
    );
    assert_eq!(starts.first(), Some(&10));
}

#[test]
fn a_wait_its_deadline_ended_is_a_timed_out_row() {
    // MP-08 / MP-10 / MP-11 PEEK-07: the kernel's wait row, closed by its deadline.
    let store = Store::new("wait");
    store.prompt(10, 1, "wait for the worker");
    store.tool(
        11,
        1,
        serde_json::json!({"id":"wait:task:0","tool":WAIT_TOOL,"status":"completed","background":true,
            "input":{"reason":"the worker","deadline_ms":1_800_000},"started_at_ms":0,
            "finished_at_ms":1_800_000,"woken_by":{"kind":"deadline_reached","source_id":"task"}}),
    );
    let peek = store.peek(10, None, 2_000_000);
    let wait = peek.entries.last().unwrap();
    assert_eq!(
        (
            wait.kind,
            wait.status,
            wait.title.as_str(),
            wait.duration_ms
        ),
        (
            Kind::Wait,
            Status::TimedOut,
            "Timed out after 30 min 0 s",
            Some(1_800_000)
        )
    );
}

#[test]
fn an_unknown_entry_or_cursor_says_what_to_pass() {
    let store = Store::new("unknown");
    store.turn(1);
    let error = read_entry(
        &store.store,
        &ReadAgentTranscriptEntryRequest {
            session_id: "room".into(),
            agent_id: "agent".into(),
            entry_id: "history:900:901".into(),
            offset: 0,
            limit: None,
        },
        None,
        &str::to_owned,
    )
    .unwrap_err();
    assert!(error.to_string().contains("pass an entry_id from its peek"));
    let error = peek(
        &store.store,
        &PeekAgentTranscriptRequest {
            session_id: "room".into(),
            agent_id: "agent".into(),
            limit: None,
            before: Some("yesterday".into()),
        },
        None,
        0,
        &str::to_owned,
    )
    .unwrap_err();
    assert!(error.to_string().contains("invalid before cursor"));
}

#[test]
fn an_interleaved_tool_keeps_the_other_rows_and_its_final_state_when_paging() {
    let store = Store::new("interleaved");
    store.prompt(10, 1, "do the work");
    store.tool(11, 1, serde_json::json!({"id":"call","tool":"bash","status":"running", "input":{"command":"sleep 30"}, "started_at_ms":1000}));
    store.output(
        12,
        1,
        TerminalOutputKind::ProviderOutput,
        "message",
        "Still working".into(),
    );
    store.tool(13, 1, serde_json::json!({"id":"call","tool":"bash","status":"completed", "input":{"command":"sleep 30"}, "started_at_ms":1000, "finished_at_ms":31000}));
    store.output(
        14,
        1,
        TerminalOutputKind::ProviderOutput,
        "answer",
        "Done".into(),
    );
    let whole = store.peek(10, None, 2_000_000);
    assert_eq!(
        whole.entries.len(),
        4,
        "a tool's range must not hide the message between its snapshots"
    );
    let mut before = None;
    let mut paged = Vec::new();
    loop {
        let page = store.peek(1, before.take(), 2_000_000);
        paged.splice(0..0, page.entries);
        match page.before {
            Some(cursor) => before = Some(cursor),
            None => break,
        }
    }
    assert_eq!(
        paged, whole.entries,
        "page boundaries must not split a completed tool into a running snapshot"
    );
}

#[test]
fn tool_snapshots_without_clock_fields_keep_the_first_start_and_final_duration() {
    let store = Store::new("provider-clock-fields");
    store.prompt(10, 1, "run the command");
    store.tool(11, 1, serde_json::json!({"id":"call","tool":"bash","status":"pending"}));
    store.tool(12, 1, serde_json::json!({"id":"call","tool":"bash","status":"running","input":{"command":"openssl speed"}}));
    let active = store.peek(10, None, 1_020_000).entries.last().unwrap().clone();
    store.tool(13, 1, serde_json::json!({"id":"call","tool":"bash","status":"completed","input":{"command":"openssl speed"},"output":"done"}));
    let ended = store.peek(10, None, 2_000_000).entries.last().unwrap().clone();
    assert_eq!((active.at_ms, active.duration_ms, ended.duration_ms), (1_011_000, Some(9_000), Some(2_000)), "providers without clock fields still have history timestamps");
}
