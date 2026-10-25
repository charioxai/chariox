use super::*;

#[tokio::test]
async fn mk03_move_peer_binds_generation_to_authenticated_home_owner_and_key() {
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
                request: crate::agent_move::peer::AgentMovePeerRequest::Probe { identity: claims },
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
    assert!(app.lock().await.agent_moves.targets().is_empty());
}
