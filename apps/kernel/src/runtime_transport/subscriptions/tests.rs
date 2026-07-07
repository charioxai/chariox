use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use super::*;
use crate::session::RuntimeSession;
use crate::terminal::TerminalOutputKind;

#[test]
fn subscription_wait_duration_uses_next_explicit_deadline() {
    let now = Instant::now();
    let wait = next_subscription_wait_duration(
        now + Duration::from_secs(1),
        now + Duration::from_secs(30),
    );

    assert!(
        wait <= Duration::from_secs(1),
        "wait should be bounded by the next heartbeat deadline"
    );
    assert!(
        wait > Duration::from_millis(900),
        "wait should not fall back to a short active-work poll"
    );
}

#[test]
fn subscription_wait_duration_is_ready_for_due_deadlines() {
    let now = Instant::now();
    let wait = next_subscription_wait_duration(
        now - Duration::from_millis(1),
        now + Duration::from_secs(30),
    );

    assert_eq!(wait, Duration::ZERO);
}

#[test]
fn subscription_deadline_intervals_preserve_previous_cadence() {
    assert_eq!(subscription_heartbeat_interval(), Duration::from_secs(5));
    assert_eq!(
        subscription_snapshot_reconciliation_interval(),
        Duration::from_secs(30)
    );
}

#[test]
fn advance_subscription_deadline_skips_missed_intervals() {
    let advanced = advance_subscription_deadline(
        Instant::now() - Duration::from_secs(11),
        Duration::from_secs(5),
    );

    assert!(advanced > Instant::now());
}

#[test]
fn compact_replay_events_preserves_heartbeat_output_and_latest_snapshot_only() {
    let events = vec![
        logged_event(
            1,
            KernelEvent::Heartbeat {
                session_id: "session-a".to_string(),
            },
        ),
        logged_event(2, session_snapshot_event("session-a", "snapshot-a")),
        logged_event(3, terminal_output_event("session-a", "first")),
        logged_event(4, session_snapshot_event("session-a", "snapshot-b")),
        logged_event(
            5,
            KernelEvent::TransportResumed {
                session_id: "session-a".to_string(),
                resumed_from_event_id: Some(1),
            },
        ),
        logged_event(6, terminal_output_event("session-a", "second")),
    ];

    let compacted = compact_replay_events(events);

    assert_eq!(
        compacted
            .iter()
            .map(|event| event.event_id)
            .collect::<Vec<_>>(),
        vec![1, 3, 4, 6]
    );
    assert!(matches!(compacted[0].event, KernelEvent::Heartbeat { .. }));
    assert!(matches!(
        compacted[1].event,
        KernelEvent::TerminalOutput { .. }
    ));
    assert!(matches!(
        compacted[2].event,
        KernelEvent::SessionSnapshot { .. }
    ));
    assert!(matches!(
        compacted[3].event,
        KernelEvent::TerminalOutput { .. }
    ));
}

fn logged_event(event_id: u64, event: KernelEvent) -> LoggedEvent<KernelEvent> {
    LoggedEvent {
        event_id,
        stream_id: "session:session-a:attachment:attachment-a".to_string(),
        stream_seq: event_id,
        recorded_at_ms: event_id * 1_000,
        event,
    }
}

fn session_snapshot_event(session_id: &str, alias: &str) -> KernelEvent {
    KernelEvent::SessionSnapshot {
        session: Box::new(RuntimeSession::new(
            session_id,
            Some(alias.to_string()),
            "workspace-a",
            "worktree-a",
            "machine-a",
            "daemon-a",
        )),
        provider_run: Box::new(None),
        agent_activity: Box::new(BTreeMap::new()),
        agent_activity_revision: 0,
    }
}

fn terminal_output_event(session_id: &str, marker: &str) -> KernelEvent {
    KernelEvent::TerminalOutput {
        records: vec![TerminalOutputRecord {
            record_id: None,
            timestamp_ms: 1_000,
            session_id: session_id.to_string(),
            provider_run_id: "provider-run-a".to_string(),
            agent_id: Some("agent-a".to_string()),
            prompt_id: None,
            prompt_origin: None,
            source_attachment_id: None,
            kind: TerminalOutputKind::ProviderOutput,
            merge_key: Some(marker.to_string()),
            recipient_attachment_ids: vec!["attachment-a".to_string()],
            pending_recipient_attachment_ids: vec!["attachment-a".to_string()],
            bytes: marker.as_bytes().to_vec(),
            external_observation_metadata: None,
        }],
    }
}
