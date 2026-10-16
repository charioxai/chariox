use super::*;
use crate::{app::DaemonApp, config::DaemonConfig};

#[test]
fn leased_projection_compacts_finalized_output_keys_and_survives_store_restart() {
    let (mut app, leased) = worker_lease();
    for index in 0..200 {
        let merge_key = format!("message-{index}");
        app.terminal_mut().fan_out_output(
            &leased.backing_session_id,
            "run",
            Some(&leased.backing_agent_id),
            TerminalOutputKind::ProviderOutput,
            Some(merge_key.clone()),
            vec![leased.backing_attachment_id.clone()],
            b"streamed output",
        );
        let first = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased.id, "run", false)
            .unwrap()
            .unwrap();
        let RelayPeerEvent::LeasedRuntimeProjection { output_chunks, .. } = first.1;
        assert_eq!(output_chunks.len(), 1);
        app.append_history_entry(
            &leased.backing_session_id,
            crate::history::SessionHistoryEntry::provider_output(
                &leased.backing_session_id,
                "run",
                Some(&leased.backing_agent_id),
                TerminalOutputKind::ProviderOutput,
                Some(merge_key),
                "streamed output".to_string(),
            ),
        );
        assert!(RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased.id, "run", false)
            .unwrap()
            .is_none());
        assert!(RemoteLeaseRuntime::new(&mut app)
            .leased_agent_snapshot_for_test(&leased.id)
            .unwrap()
            .projected_output_history_keys
            .is_empty());
    }
    // Reopen the worker's durable store, retaining only normal lease identity.
    let path = app.operational_history_store().path().to_path_buf();
    app.operational_history = crate::history::OperationalHistoryStore::open(path).unwrap();
    assert!(RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased.id, "run", false)
        .unwrap()
        .is_none());
    // A duplicated finalized transcript must also remain suppressed after reopen.
    app.append_history_entry(
        &leased.backing_session_id,
        crate::history::SessionHistoryEntry::provider_output(
            &leased.backing_session_id,
            "run",
            Some(&leased.backing_agent_id),
            TerminalOutputKind::ProviderOutput,
            Some("message-0".into()),
            "streamed output".to_string(),
        ),
    );
    assert!(RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased.id, "run", false)
        .unwrap()
        .is_none());
    RemoteLeaseRuntime::new(&mut app)
        .destroy_leased_agent(&leased.id)
        .unwrap();
}

#[test]
fn leased_projection_compacts_tool_deltas_with_different_transcript_bytes() {
    let (mut app, leased) = worker_lease();
    let mut output = String::new();
    let mut first_delta = None;
    let mut last_delta = None;
    for index in 0..1000 {
        output.push_str(&format!("line-{index}\n"));
        let bytes = serde_json::to_vec_pretty(&serde_json::json!({
            "id": "tool", "status": "running", "output": output,
        }))
        .unwrap();
        let record = crate::app::provider_output_fanout::ProviderOutputFanout::new(&app)
            .fan_out_for_agent(
                &leased.backing_session_id,
                "run",
                Some(&leased.backing_agent_id),
                TerminalOutputKind::ProviderTool,
                Some("tool".into()),
                vec![leased.backing_attachment_id.clone()],
                &bytes,
            );
        if index == 1 {
            first_delta = Some(record.bytes.clone());
        }
        last_delta = Some(record.bytes);
        let projection = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased.id, "run", false)
            .unwrap()
            .unwrap();
        let RelayPeerEvent::LeasedRuntimeProjection { output_chunks, .. } = projection.1;
        assert_eq!(
            output_chunks.len(),
            1,
            "each changed tool update must project"
        );
        assert_eq!(
            app.operational_history_store()
                .pending_leased_projection_counts(),
            (0, 0)
        );
    }
    let path = app.operational_history_store().path().to_path_buf();
    app.operational_history = crate::history::OperationalHistoryStore::open(path).unwrap();
    for duplicate in [first_delta.unwrap(), last_delta.unwrap()] {
        app.terminal_mut().fan_out_output(
            &leased.backing_session_id,
            "run",
            Some(&leased.backing_agent_id),
            TerminalOutputKind::ProviderTool,
            Some("tool".into()),
            vec![leased.backing_attachment_id.clone()],
            &duplicate,
        );
        assert!(RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased.id, "run", false)
            .unwrap()
            .is_none());
    }
    assert_eq!(
        app.operational_history_store()
            .pending_leased_projection_counts(),
        (0, 0)
    );
    RemoteLeaseRuntime::new(&mut app)
        .destroy_leased_agent(&leased.id)
        .unwrap();
}

pub(super) fn worker_lease() -> (DaemonApp, LeasedAgent) {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease("home", "session", "agent", false, "user")
        .unwrap();
    let leased = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    (app, leased)
}

#[test]
fn leased_projection_separates_json_tool_identities_without_merge_keys() {
    let (mut app, leased) = worker_lease();
    let mut deltas = Vec::new();
    for (field, id, length) in [
        ("id", "x", 4000),
        ("call_id", "y", 5),
        ("id", "x", 4001),
        ("call_id", "y", 6),
    ] {
        let bytes = serde_json::to_vec(
            &serde_json::json!({field: id, "status": "running", "output": "a".repeat(length)}),
        )
        .unwrap();
        let record = crate::app::provider_output_fanout::ProviderOutputFanout::new(&app)
            .fan_out_for_agent(
                &leased.backing_session_id,
                "run",
                Some(&leased.backing_agent_id),
                TerminalOutputKind::ProviderTool,
                None,
                vec![leased.backing_attachment_id.clone()],
                &bytes,
            );
        let projection = RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased.id, "run", false)
            .unwrap()
            .unwrap();
        let RelayPeerEvent::LeasedRuntimeProjection { output_chunks, .. } = projection.1;
        assert_eq!(
            output_chunks.len(),
            1,
            "independent tools must not share offsets"
        );
        if length == 4001 || length == 6 {
            deltas.push(record.bytes);
        }
        assert_eq!(
            app.operational_history_store()
                .pending_leased_projection_counts(),
            (0, 0)
        );
    }
    let path = app.operational_history_store().path().to_path_buf();
    app.operational_history = crate::history::OperationalHistoryStore::open(path).unwrap();
    for duplicate in deltas {
        app.terminal_mut().fan_out_output(
            &leased.backing_session_id,
            "run",
            Some(&leased.backing_agent_id),
            TerminalOutputKind::ProviderTool,
            None,
            vec![leased.backing_attachment_id.clone()],
            &duplicate,
        );
        assert!(RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection(&leased.id, "run", false)
            .unwrap()
            .is_none());
    }
    RemoteLeaseRuntime::new(&mut app)
        .destroy_leased_agent(&leased.id)
        .unwrap();
}

// MP-08 / MP-10 / MP-11: a real lost peer reply must not consume its output.
#[test]
fn mp08_projection_lost_reply_replays_output_after_store_reopen() {
    let (mut app, leased) = worker_lease();
    let run = super::explicit_completion_tests::provider_run(
        "run",
        &leased.backing_session_id,
        &leased.backing_agent_id,
        "managed-dev-stub",
        "managed-dev-stub",
        crate::provider::ProviderClientInterface::Chariox,
    );
    app.providers_mut().insert_run_for_test(run);
    app.terminal_mut().fan_out_output(
        &leased.backing_session_id,
        "run",
        Some(&leased.backing_agent_id),
        TerminalOutputKind::ProviderOutput,
        Some("answer".into()),
        vec![leased.backing_attachment_id.clone()],
        b"must survive reply loss",
    );
    let first = RemoteLeaseRuntime::new(&mut app)
        .drain_retained_leased_runtime_projection(&leased.id, "run", false, false)
        .unwrap()
        .unwrap();
    let path = app.operational_history_store().path().to_path_buf();
    app.operational_history = crate::history::OperationalHistoryStore::open(path).unwrap();
    let retry = RemoteLeaseRuntime::new(&mut app)
        .drain_retained_leased_runtime_projection(&leased.id, "run", false, false)
        .unwrap();
    assert_eq!(
        serde_json::to_value(retry).unwrap(),
        serde_json::to_value(Some(first)).unwrap(),
        "lost consuming reply discarded the worker output"
    );
}

#[test]
fn mp08_history_only_tool_snapshots_survive_retained_delivery_and_reopen() {
    let (mut app, leased) = worker_lease();
    let run = super::explicit_completion_tests::provider_run(
        "run",
        &leased.backing_session_id,
        &leased.backing_agent_id,
        "managed-dev-stub",
        "managed-dev-stub",
        crate::provider::ProviderClientInterface::Chariox,
    );
    app.providers_mut().insert_run_for_test(run);
    for (index, status) in ["running", "completed"].into_iter().enumerate() {
        let payload = serde_json::to_string(&serde_json::json!({
            "id": "read-history-only", "name": "Read", "status": status,
            "output": format!("public source line {index}"),
        }))
        .unwrap();
        app.append_history_entry(
            &leased.backing_session_id,
            crate::history::SessionHistoryEntry::provider_output(
                &leased.backing_session_id,
                "run",
                Some(&leased.backing_agent_id),
                TerminalOutputKind::ProviderTool,
                Some("read-history-only".into()),
                payload.clone(),
            ),
        );
        let event = RemoteLeaseRuntime::new(&mut app)
            .drain_retained_leased_runtime_projection(&leased.id, "run", false, false)
            .unwrap()
            .expect("history-only tool must create a delivery")
            .1;
        let RelayPeerEvent::LeasedRuntimeProjection {
            output_chunks,
            delivery: Some(receipt),
            ..
        } = &event
        else {
            panic!("receipted history projection required")
        };
        assert_eq!(
            output_chunks.len(),
            1,
            "a tool absent from terminal records must reach home"
        );
        assert_eq!(output_chunks[0].kind, TerminalOutputKind::ProviderTool);
        assert_eq!(output_chunks[0].bytes, payload.as_bytes());
        let path = app.operational_history_store().path().to_path_buf();
        app.operational_history = crate::history::OperationalHistoryStore::open(path).unwrap();
        let retry = RemoteLeaseRuntime::new(&mut app)
            .drain_retained_leased_runtime_projection(&leased.id, "run", false, false)
            .unwrap()
            .unwrap()
            .1;
        assert_eq!(
            serde_json::to_value(retry).unwrap(),
            serde_json::to_value(&event).unwrap(),
            "lost page ACK replays identical retained bytes"
        );
        assert!(RemoteLeaseRuntime::new(&mut app)
            .acknowledge_leased_runtime_projection(&leased.id, "run", receipt)
            .unwrap());
        assert!(RemoteLeaseRuntime::new(&mut app)
            .drain_retained_leased_runtime_projection(&leased.id, "run", false, false)
            .unwrap()
            .is_none());
    }
    RemoteLeaseRuntime::new(&mut app)
        .destroy_leased_agent(&leased.id)
        .unwrap();
}

#[test]
fn mp08_history_owned_tool_ignores_late_terminal_records() {
    let (mut app, leased) = worker_lease();
    let run = super::explicit_completion_tests::provider_run(
        "run",
        &leased.backing_session_id,
        &leased.backing_agent_id,
        "managed-dev-stub",
        "managed-dev-stub",
        crate::provider::ProviderClientInterface::Chariox,
    );
    app.providers_mut().insert_run_for_test(run);
    let payloads = ["running", "completed"].map(|status| {
        serde_json::to_vec(&serde_json::json!({
            "id": "late-tool", "name": "Read", "status": status, "output": "public source",
        }))
        .unwrap()
    });
    for payload in &payloads {
        app.append_history_entry(
            &leased.backing_session_id,
            crate::history::SessionHistoryEntry::provider_output(
                &leased.backing_session_id,
                "run",
                Some(&leased.backing_agent_id),
                TerminalOutputKind::ProviderTool,
                Some("late-tool".into()),
                String::from_utf8(payload.clone()).unwrap(),
            ),
        );
    }
    let first = RemoteLeaseRuntime::new(&mut app)
        .drain_retained_leased_runtime_projection(&leased.id, "run", false, false)
        .unwrap()
        .expect("history owns a tool whose terminal records are still queued")
        .1;
    let RelayPeerEvent::LeasedRuntimeProjection {
        output_chunks,
        delivery: Some(receipt),
        ..
    } = &first
    else {
        panic!("receipted history projection required")
    };
    assert_eq!(output_chunks.len(), 2);
    assert!(RemoteLeaseRuntime::new(&mut app)
        .acknowledge_leased_runtime_projection(&leased.id, "run", receipt)
        .unwrap());
    // A bounded terminal drain delivers the same snapshots in a later batch.
    for payload in &payloads {
        app.terminal_mut().fan_out_output(
            &leased.backing_session_id,
            "run",
            Some(&leased.backing_agent_id),
            TerminalOutputKind::ProviderTool,
            Some("late-tool".into()),
            vec![leased.backing_attachment_id.clone()],
            payload,
        );
    }
    let late = RemoteLeaseRuntime::new(&mut app)
        .drain_retained_leased_runtime_projection(&leased.id, "run", false, false)
        .unwrap();
    assert!(
        late.is_none_or(|(_, event)| {
            let RelayPeerEvent::LeasedRuntimeProjection { output_chunks, .. } = event;
            output_chunks.is_empty()
        }),
        "late terminal copies of history-projected snapshots must not duplicate home rows"
    );
    RemoteLeaseRuntime::new(&mut app)
        .destroy_leased_agent(&leased.id)
        .unwrap();
}

#[test]
fn mp08_history_owned_running_tool_keeps_forwarding_new_output() {
    let (mut app, leased) = worker_lease();
    let run = super::explicit_completion_tests::provider_run(
        "run",
        &leased.backing_session_id,
        &leased.backing_agent_id,
        "managed-dev-stub",
        "managed-dev-stub",
        crate::provider::ProviderClientInterface::Chariox,
    );
    app.providers_mut().insert_run_for_test(run);
    let tool = |output: &str| {
        serde_json::to_vec(&serde_json::json!({
            "id": "long-tool", "name": "Bash", "status": "running", "output": output,
        }))
        .unwrap()
    };
    // The durable window projects the first running snapshot while the
    // bounded terminal batch is still queued.
    app.append_history_entry(
        &leased.backing_session_id,
        crate::history::SessionHistoryEntry::provider_output(
            &leased.backing_session_id,
            "run",
            Some(&leased.backing_agent_id),
            TerminalOutputKind::ProviderTool,
            Some("long-tool".into()),
            String::from_utf8(tool("first ")).unwrap(),
        ),
    );
    let drain = |app: &mut DaemonApp| {
        let event = RemoteLeaseRuntime::new(app)
            .drain_retained_leased_runtime_projection(&leased.id, "run", false, false)
            .unwrap()
            .map(|(_, event)| event);
        let Some(RelayPeerEvent::LeasedRuntimeProjection {
            output_chunks,
            delivery: Some(receipt),
            ..
        }) = event
        else {
            return Vec::new();
        };
        assert!(RemoteLeaseRuntime::new(app)
            .acknowledge_leased_runtime_projection(&leased.id, "run", &receipt)
            .unwrap());
        output_chunks
            .into_iter()
            .map(|chunk| String::from_utf8(chunk.bytes).unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(drain(&mut app).len(), 1);
    // The real fan-out gate turns the cumulative output into deltas: the
    // first snapshot repeats what history projected, the later ones are new
    // output with the same running status, which history never persists.
    for output in ["first ", "first second ", "first second third"] {
        app.fan_out_output_for_agent(
            &leased.backing_session_id,
            "run",
            Some(&leased.backing_agent_id),
            TerminalOutputKind::ProviderTool,
            Some("long-tool".into()),
            vec![leased.backing_attachment_id.clone()],
            &tool(output),
        );
    }
    let forwarded = drain(&mut app);
    let outputs: Vec<String> = forwarded
        .iter()
        .map(|bytes| {
            serde_json::from_str::<serde_json::Value>(bytes).unwrap()["output"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(
        outputs,
        ["second ", "third"],
        "new output after history took the tool must reach home exactly once"
    );
    assert!(drain(&mut app).is_empty());
    RemoteLeaseRuntime::new(&mut app)
        .destroy_leased_agent(&leased.id)
        .unwrap();
}
