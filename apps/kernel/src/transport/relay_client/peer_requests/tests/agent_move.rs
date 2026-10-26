use super::*;

#[tokio::test]
async fn mk03_move_peer_binds_generation_and_refuses_stale_mutations() {
    let home = DaemonConfig::for_tests();
    let target = DaemonConfig::for_tests();
    let app = Arc::new(Mutex::new(
        crate::DaemonApp::bootstrap(target.clone()).unwrap(),
    ));
    let router = Arc::new(CommandRouter::with_interactive_capacity(app.clone(), 4));
    let state = Arc::new(RwLock::new(RelayClientState::default()));
    let (sender, _priority, _events) = RelayOutgoingSender::channel(8);
    let thumb = crate::runtime::terminal_pairings::public_key_thumbprint;
    let identity = crate::agent_move::AgentMoveIdentity {
        operation_id: "move-peer-probe".into(),
        generation: 1,
        home_kernel_id: home.daemon_id.clone(),
        home_session_id: "room".into(),
        home_agent_id: "agent".into(),
        owner_user_id: "user-1".into(),
        source_kernel_id: home.daemon_id.clone(),
        target_kernel_id: target.daemon_id.clone(),
        home_key_thumbprint: thumb(&home.relay_public_key),
        source_key_thumbprint: thumb(&home.relay_public_key),
        target_key_thumbprint: thumb(&target.relay_public_key),
        target_public_key: target.relay_public_key.clone(),
    };
    let temporary_home = format!(
        "{}:peer-tmp:daemon-peer-tmp-4242-1767225600123-7",
        home.daemon_id
    );
    let caller = scoped_machine_identity("home-machine", Some(thumb(&home.relay_public_key)));
    for alteration in ["valid", "home", "owner", "key", "target"] {
        let mut claims = identity.clone();
        match alteration {
            "home" => claims.home_kernel_id = "another-home".into(),
            "owner" => claims.owner_user_id = "another-owner".into(),
            "key" => claims.home_key_thumbprint = "another-key".into(),
            "target" => claims.target_kernel_id = "another-target".into(),
            _ => (),
        }
        let response = send_authenticated_peer_request_for_test(
            &router,
            &state,
            &sender,
            &temporary_home,
            caller.clone(),
            &home.relay_private_key,
            &target.relay_public_key,
            RelayPeerRequest::AgentMove {
                request: crate::agent_move::peer::AgentMovePeerRequest::Probe {
                    identity: claims,
                    native: None,
                },
            },
        )
        .await;
        if alteration == "valid" {
            assert!(
                matches!(
                    response,
                    Ok(RelayPeerResponse::AgentMove {
                        response: crate::agent_move::peer::AgentMovePeerResponse::Compatible
                    })
                ),
                "canonical temporary home must be admitted: {response:?}"
            );
        } else {
            assert!(
                !matches!(response, Ok(RelayPeerResponse::AgentMove { .. })),
                "foreign {alteration} cannot acquire Move authority"
            );
        }
    }
    let mut capture = crate::agent_move::AgentMoveCapture {
        identity: identity.clone(),
        source_project_id: "project".into(),
        archive_project_id: "project".into(),
        source_provider_run_id: None,
        source_repositories: Vec::new(),
        workspace: crate::agent_move::AgentMoveArtifact {
            sha256: "0".repeat(64),
            size_bytes: u64::MAX,
        },
        native: None,
        checkpoint_binding: None,
    };
    use crate::agent_move::peer::{AgentMoveChunk, AgentMovePeerRequest as Request};
    let response = send_authenticated_peer_request_for_test(
        &router,
        &state,
        &sender,
        &temporary_home,
        caller.clone(),
        &home.relay_private_key,
        &target.relay_public_key,
        RelayPeerRequest::AgentMove {
            request: Request::BeginTarget {
                capture: Box::new(capture.clone()),
            },
        },
    )
    .await;
    assert!(
        !matches!(&response, Ok(RelayPeerResponse::AgentMove { .. })),
        "staging must refuse a transfer larger than available disk: {response:?}"
    );
    assert!(
        format!("{response:?}").contains("space"),
        "capacity rejection must identify free space"
    );
    assert!(app.lock().await.agent_moves.targets().is_empty());
    assert!(!target
        .private_runtime_state_root()
        .join("move-workspaces")
        .exists());
    assert!(!app
        .lock()
        .await
        .agent_moves
        .artifact_root(&identity.operation_id)
        .exists());

    capture.workspace.size_bytes = 1;
    let prepared = send_authenticated_peer_request_for_test(
        &router,
        &state,
        &sender,
        &temporary_home,
        caller.clone(),
        &home.relay_private_key,
        &target.relay_public_key,
        RelayPeerRequest::AgentMove {
            request: Request::BeginTarget {
                capture: Box::new(capture),
            },
        },
    )
    .await;
    assert!(matches!(
        prepared,
        Ok(RelayPeerResponse::AgentMove {
            response: crate::agent_move::peer::AgentMovePeerResponse::TargetPrepared
        })
    ));
    let mut stale = identity.clone();
    stale.generation += 1;
    for request in [
        Request::WriteChunk {
            identity: stale.clone(),
            kind: crate::agent_move::AgentMoveArtifactKind::Workspace,
            chunk: AgentMoveChunk {
                offset: 0,
                bytes: vec![42],
            },
        },
        Request::PublishTarget {
            identity: stale.clone(),
        },
        Request::ActivateTarget {
            identity: stale.clone(),
        },
    ] {
        let response = send_authenticated_peer_request_for_test(
            &router,
            &state,
            &sender,
            &temporary_home,
            caller.clone(),
            &home.relay_private_key,
            &target.relay_public_key,
            RelayPeerRequest::AgentMove { request },
        )
        .await;
        assert!(
            !matches!(&response, Ok(RelayPeerResponse::AgentMove { .. })),
            "stale generation cannot mutate its target: {response:?}"
        );
        assert!(format!("{response:?}").contains("generation changed"));
    }
    let target = app
        .lock()
        .await
        .agent_moves
        .target(&identity.operation_id)
        .unwrap();
    assert_eq!(target.capture.identity.generation, 1);
    assert!(!target.active && !target.ready && target.receipt.is_none());
    assert!(!target.destination_root.exists());
}
