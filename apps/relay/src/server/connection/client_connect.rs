//! MP-08/MP-10/MP-11: one client admission path, optional initial535 transport.
use super::*;

pub(super) struct ClientConnectContext<'a> {
    pub registry: &'a Arc<RwLock<RelayRegistry>>,
    pub routes: &'a Arc<crate::registry::RelayRouteIndex>,
    pub auth_verifier: &'a RelayAuthVerifier,
    pub peer_addr: SocketAddr,
    pub outgoing_tx: &'a RelaySender,
    pub relay_request_counter: &'a AtomicU64,
    pub connection_meter: &'a ConnectionMeter,
    pub verified_identity: &'a mut Option<VerifiedRelayIdentity>,
    pub auth_expiry_deadline: &'a mut Option<Instant>,
    pub initial_resume_negotiated: bool,
}

pub(super) async fn admit(
    envelope: RelayEnvelope,
    context: ClientConnectContext<'_>,
) -> Result<bool, std::io::Error> {
    let ClientConnectContext {
        registry,
        routes,
        auth_verifier,
        peer_addr,
        outgoing_tx,
        relay_request_counter,
        connection_meter,
        verified_identity,
        auth_expiry_deadline,
        initial_resume_negotiated,
    } = context;
    let (auth_token, target, initial) = match envelope {
        RelayEnvelope::ClientConnect { auth_token, target } => (auth_token, target, None),
        RelayEnvelope::ClientConnectDisplayResume {
            auth_token,
            target,
            expected_daemon_public_key,
            registration,
        } => {
            if !initial_resume_negotiated || verified_identity.is_some() {
                send_close(
                    outgoing_tx,
                    "initial display resume requires a fresh negotiated535 connection".into(),
                );
                return Ok(false);
            }
            (
                auth_token,
                target,
                Some((expected_daemon_public_key, registration)),
            )
        }
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "expected client connect envelope",
            ))
        }
    };
    let identity = verify_relay_token(
        auth_verifier,
        &auth_token,
        RelayAction::ClientConnect,
        target
            .daemon_id
            .as_deref()
            .or(target.daemon_alias.as_deref()),
    )?;
    if !connection_meter.bind_realm(&identity.realm_id) {
        send_close(outgoing_tx, REALM_CHANGED.into());
        return Ok(false);
    }
    if verified_identity.as_ref().is_some_and(|previous| {
        !super::super::client_renewal::retains_client_authority(previous, &identity)
    }) {
        send_close(
            outgoing_tx,
            "relay authorization renewal changed identity or reduced permissions".into(),
        );
        return Ok(false);
    }
    // Preserve ordinary renewal's original ordering: arm the new expiry and
    // authority before any target-resolution await or acknowledgement write.
    *auth_expiry_deadline = relay_auth_expiry_deadline(auth_verifier, identity.expires_at_ms);
    *verified_identity = Some(identity.clone());
    let Some(daemon_key) =
        resolve_target_daemon_key(registry, auth_verifier, &identity.realm_id, &target).await
    else {
        log_target_not_connected(
            "client_connect",
            registry,
            peer_addr,
            &identity.realm_id,
            &target,
        )
        .await;
        send_close(
            outgoing_tx,
            "target daemon is not connected to relay".into(),
        );
        return Ok(false);
    };
    let existing_client_key = {
        let guard = registry.read().await;
        guard
            .peers
            .get(&peer_addr)
            .filter(|peer| peer.role == RelayConnectionRole::Client)
            .and_then(|peer| peer.client_daemon_key.clone())
    };
    if existing_client_key
        .as_ref()
        .is_some_and(|existing| existing != &daemon_key)
    {
        send_close(
            outgoing_tx,
            "client connection already bound to target".into(),
        );
        return Ok(false);
    }
    let daemon_public_key = {
        let mut guard = registry.write().await;
        let key = guard
            .daemons
            .get(&daemon_key)
            .map(|registration| registration.public_key.clone());
        // Pin comparison and client binding share the registry transaction.
        // No initial registration reaches a differently keyed target.
        if initial
            .as_ref()
            .is_some_and(|(expected, _)| key.as_ref() != Some(expected))
        {
            send_close(
                outgoing_tx,
                "initial display resume target key changed".into(),
            );
            return Ok(false);
        }
        let allowed_actions = identity.allowed_actions.clone();
        let allowed_targets = identity.allowed_targets.clone();
        guard.peers.insert(
            peer_addr,
            PeerHandle {
                sender: outgoing_tx.clone(),
                role: RelayConnectionRole::Client,
                realm_id: Some(identity.realm_id.clone()),
                identity: Some(identity.into()),
                allowed_actions,
                allowed_targets,
                daemon_registration: None,
                client_daemon_key: Some(daemon_key),
            },
        );
        key.unwrap_or_default()
    };
    routes.set_client_sender(peer_addr, outgoing_tx.clone());
    send_envelope(
        outgoing_tx,
        &RelayEnvelope::ClientConnected {
            target: target.clone(),
            daemon_public_key,
        },
    )?;
    if let Some((pin, registration)) = initial {
        // Composition has no second inbound-message boundary. Recheck token
        // expiry/revocation immediately before the ordinary route admission.
        verify_relay_token(
            auth_verifier,
            &auth_token,
            RelayAction::ClientConnect,
            target
                .daemon_id
                .as_deref()
                .or(target.daemon_alias.as_deref()),
        )?;
        // Transport composition only: reuse every ordinary subscribe action,
        // target, identifier, caller, capacity and response-ownership check.
        if handle_client_packet_route_envelope_with_target_pin(
            RelayEnvelope::ClientSubscribe {
                request_id: registration.request_id,
                subscription_id: registration.subscription_id,
                target,
                session_id: registration.session_id,
                attachment_id: registration.attachment_id,
                client_public_key: registration.client_public_key,
                subscription_scope: Some(crate::frame_transport::DISPLAY_RESUME_SCOPE.into()),
                resume_from_event_id: None,
                heartbeat_interval_ms: None,
            },
            registry,
            routes,
            auth_verifier,
            ClientPacketPeer {
                peer_addr,
                expected_daemon_public_key: Some(&pin),
            },
            outgoing_tx,
            relay_request_counter,
        )
        .await?
        .should_break()
        {
            return Ok(false);
        }
    }
    Ok(true)
}
