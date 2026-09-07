use super::*;

#[tokio::test]
async fn forwarded_workspace_requests_reject_a_different_relay_sender_before_dispatch() {
    let app = crate::DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
    let target_key = app.config().relay_public_key.clone();
    let router = Arc::new(CommandRouter::with_interactive_capacity(
        Arc::new(tokio::sync::Mutex::new(app)),
        1,
    ));
    let state = Arc::new(RwLock::new(RelayClientState::default()));
    let (outgoing, _priority, _events) = RelayOutgoingSender::channel(1);
    let context = crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext {
        home_kernel_id: router.relay_daemon_id(),
        home_session_id: "missing-session".into(),
        home_agent_id: "agent".into(),
        leased_agent_id: "leased".into(),
        worker_kernel_id: "bound-worker".into(),
        worker_machine_id: "machine".into(),
        worker_provider_run_id: "run".into(),
        worker_worktree_path: "/workspace".into(),
        worker_workspace_identity: crate::io::WorkspaceIdentity::local("/workspace"),
    };
    let metadata = crate::transport::relay_peer::RemoteWorkspaceLiveSyncInvocationMetadata {
        invocation_id: "write".into(),
        provider_tool_call_id: None,
        attempt: 1,
        idempotency_key: None,
    };
    let requests = [
        RelayPeerRequest::ForwardWorkspaceLiveSyncRuntimeTool {
            context: context.clone(),
            metadata: metadata.clone(),
            tool_name: "write_artifact".into(),
            arguments: serde_json::json!({}),
            artifact_states: vec![],
        },
        RelayPeerRequest::FinalizeWorkspaceLiveSyncRuntimeTool {
            context,
            metadata,
            tool_name: "write_artifact".into(),
            arguments: serde_json::json!({}),
            initial_artifact_states: vec![],
            final_artifact_states: vec![],
        },
    ];
    let sender_key = relay_crypto::generate_private_key_base64();
    for request in requests {
        for sender in ["different-worker", ""] {
            let encrypted = relay_crypto::encrypt_payload_for_peer(
                &sender_key,
                &target_key,
                &serde_json::to_vec(&request).unwrap(),
            )
            .unwrap();
            let outcome =
                handle_daemon_peer_request(&router, &state, &outgoing, sender, None, encrypted)
                    .await;
            let error = outcome.error.expect("mismatched sender must be rejected");
            assert_eq!(error.code, "unauthorized");
            assert!(!error.retryable);
            assert!(outcome.encrypted_response.is_none());
        }
        let encrypted = relay_crypto::encrypt_payload_for_peer(
            &sender_key,
            &target_key,
            &serde_json::to_vec(&request).unwrap(),
        )
        .unwrap();
        let outcome =
            handle_daemon_peer_request(&router, &state, &outgoing, "bound-worker", None, encrypted)
                .await;
        assert_eq!(
            outcome.error.unwrap().code,
            "session_not_found",
            "a matching legacy relay sender should reach normal kernel dispatch"
        );
    }
}
