use super::*;
use crate::{app::DaemonApp, config::DaemonConfig};

#[test]
fn leased_projection_compacts_finalized_output_keys_and_survives_store_restart() {
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
