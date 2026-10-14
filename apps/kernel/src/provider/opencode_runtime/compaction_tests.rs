//! MP-08 / MP-10: the live long-history drill compacted while a new user turn was queued.
use std::sync::mpsc;

use serde_json::json;

use crate::extension::RemoteExtensionManifest;
use crate::provider::opencode_client::{OpenCodeEventSubscription, OpenCodeMessage};

use super::snapshot::{collect_new_completed_assistant_messages, render_snapshot_output_chunks};
use super::OpenCodeRuntimeState;

fn messages() -> Vec<OpenCodeMessage> {
    serde_json::from_value(json!([
        {"info":{"id":"compact","sessionID":"session-1","role":"user","time":{"created":80}},
         "parts":[{"id":"compact-part","sessionID":"session-1","messageID":"compact","type":"compaction","auto":true}]},
        {"info":{"id":"summary","sessionID":"session-1","role":"assistant","parentID":"compact","summary":true,"finish":"stop","time":{"created":81,"completed":200}},
         "parts":[{"id":"summary-part","sessionID":"session-1","messageID":"summary","type":"text","text":"private provider compaction context"}]},
        {"info":{"id":"current","sessionID":"session-1","role":"user","time":{"created":100}},"parts":[]},
        {"info":{"id":"continue","sessionID":"session-1","role":"user","time":{"created":201}},
         "parts":[{"id":"continue-part","sessionID":"session-1","messageID":"continue","type":"text","synthetic":true,"text":"Continue"}]},
        {"info":{"id":"answer","sessionID":"session-1","role":"assistant","parentID":"continue","finish":"stop","time":{"created":202,"completed":250}},
         "parts":[{"id":"answer-part","sessionID":"session-1","messageID":"answer","type":"text","text":"real completion"}]}
    ])).unwrap()
}

fn state() -> OpenCodeRuntimeState {
    let (_, receiver) = mpsc::channel();
    let mut state = OpenCodeRuntimeState::new(
        "http://127.0.0.1:1".into(),
        "session-1".into(),
        OpenCodeEventSubscription::for_tests(receiver),
    );
    state.note_prompt_submitted("current".into());
    state
}

#[test]
fn queued_turn_survives_native_auto_compaction_continuation() {
    let mut state = state();
    let messages = messages();
    let chunks =
        render_snapshot_output_chunks(&mut state, &RemoteExtensionManifest::default(), &messages)
            .chunks;
    assert_eq!(
        chunks
            .iter()
            .map(|c| String::from_utf8_lossy(&c.bytes))
            .collect::<Vec<_>>(),
        ["real completion"]
    );
    let completions = collect_new_completed_assistant_messages(&mut state, &messages);
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].message_id, "answer");
    assert_eq!(state.active_user_message_id.as_deref(), Some("current"));
}

#[test]
fn rejects_unrelated_manual_stale_and_baseline_continuations() {
    for case in [
        "ordinary-user",
        "manual",
        "stale",
        "foreign-session",
        "not-synthetic",
        "baseline",
        "failed-summary",
        "foreign-trigger-part",
        "foreign-continuation-part",
    ] {
        let mut state = state();
        let mut messages = messages();
        match case {
            "ordinary-user" => messages.insert(3, serde_json::from_value(json!({"info":{"id":"other-user","sessionID":"session-1","role":"user","time":{"created":150}},"parts":[]})).unwrap()),
            "manual" => messages[0].parts[0].auto = false,
            "stale" => messages[1].info.time.completed = Some(90),
            "foreign-session" => messages[1].info.session_id = "other-session".into(),
            "not-synthetic" => messages[3].parts[0].synthetic = false,
            "baseline" => { state.preexisting_message_ids.insert("continue".into()); }
            "failed-summary" => messages[1].info.error = Some(json!({"name":"ProviderError"})),
            "foreign-trigger-part" => messages[0].parts[0].session_id = "other-session".into(),
            "foreign-continuation-part" => messages[3].parts[0].message_id = "other-user".into(),
            _ => unreachable!(),
        }
        let chunks = render_snapshot_output_chunks(
            &mut state,
            &RemoteExtensionManifest::default(),
            &messages,
        )
        .chunks;
        assert!(chunks.is_empty(), "{case}");
        assert!(!state.is_active_prompt_parent("continue"), "{case}");
    }
}

#[test]
fn accepted_continuation_does_not_cross_the_next_prompt_or_cancellation() {
    let mut state = state();
    let messages = messages();
    super::snapshot::record_snapshot_message_metadata(&mut state, &messages);
    assert!(state.is_active_prompt_parent("continue"));
    assert!(!state.is_active_prompt_parent("foreign"));
    state.note_prompt_submitted("next".into());
    assert!(!state.is_active_prompt_parent("continue"));
    state.note_prompt_submitted("current".into());
    super::snapshot::record_snapshot_message_metadata(&mut state, &messages);
    state.settle_aborted_turn(&messages);
    assert!(!state.is_active_prompt_parent("continue"));
}

#[test]
fn compaction_summary_is_never_the_current_answer_or_terminal_completion() {
    let messages = messages();
    assert!(!messages[1].info.is_terminal_assistant_completion());
    // A provider can append a compaction marker to the submitted user itself.
    let mut messages = messages;
    messages[1].info.parent_id = Some("current".into());
    messages[2].parts = messages[0].parts.clone();
    messages[2].parts[0].message_id = "current".into();
    let mut state = state();
    let chunks =
        render_snapshot_output_chunks(&mut state, &RemoteExtensionManifest::default(), &messages)
            .chunks;
    assert_eq!(
        chunks
            .iter()
            .map(|c| String::from_utf8_lossy(&c.bytes))
            .collect::<Vec<_>>(),
        ["real completion"]
    );
}
