//! OS admission and grant checks for the Unix websocket transport.
use super::*;
use crate::runtime::command::KernelCaller;
use crate::runtime::kernel_access::process::ProcessIdentity;

pub(super) async fn admit_frame(
    connection: &IncomingConnection<'_>,
    peer: &ProcessIdentity,
    frame: &KernelIncomingFrame,
) -> Result<KernelCaller, ()> {
    let IncomingConnection {
        runtime,
        router,
        inbound_request_admission,
        connection_inbound_request_permits,
        outgoing_tx,
        close_tx,
        close_requested,
        bound_grant,
        ..
    } = *connection;

    let runtime_state = router.runtime_state();
    let bound = bound_grant.lock().expect("bound grant poisoned").clone();
    let sudo = if bound.as_deref().is_none_or(|id| id.starts_with("sudo:")) {
        runtime_state
            .sudo_for_peer(peer)
            .ok()
            .filter(|turn| bound.as_ref().is_none_or(|id| id == &turn.entry_id))
    } else {
        None
    };
    if let Some(turn) = sudo {
        let allowed = match frame {
            KernelIncomingFrame::Request { request, .. } => runtime_state
                .authorize_sudo_request(&turn.entry_id, request)
                .is_ok(),
            KernelIncomingFrame::Subscribe {
                session_id,
                attachment_id,
                subscription_scope,
                ..
            } => {
                session_id == &turn.session_id
                    && runtime_state
                        .attachment_owner_user_id(attachment_id)
                        .await
                        .is_ok_and(|owner| owner == turn.owner_user_id)
                    && kernel_subscription_scope(subscription_scope.as_deref())
                        != KernelSubscriptionScope::WaitingRoomInventory
                    && router
                        .session_id_for_attachment_access(attachment_id)
                        .as_deref()
                        == Some(session_id.as_str())
            }
            KernelIncomingFrame::Unsubscribe { .. } => true,
        };
        if allowed {
            bound_grant
                .lock()
                .expect("bound grant poisoned")
                .get_or_insert_with(|| turn.entry_id.clone());
            let mut caller = router
                .local_command_caller(
                    KernelCommandSource::LocalIpc,
                    KernelConnectionClass::KernelAgent,
                )
                .await;
            caller.caller_id = turn.entry_id;
            caller.user_id = Some(turn.owner_user_id);
            return Ok(caller);
        }
        // Refused sudo operations must not fall back to an external grant or
        // raise an access popup through the unauthenticated admission path.
        deny_frame(runtime, outgoing_tx, close_tx, close_requested, frame);
        return Err(());
    }
    if bound.as_deref().is_some_and(|id| id.starts_with("sudo:")) {
        deny_frame(runtime, outgoing_tx, close_tx, close_requested, frame);
        return Err(());
    }
    if let KernelIncomingFrame::Request {
        request_id,
        request: crate::local::LocalDaemonRequest::RequestKernelAccess(request),
        ..
    } = frame
    {
        if bound.is_some() || !runtime_state.access_grants_for(peer).is_empty() {
            deny_frame(runtime, outgoing_tx, close_tx, close_requested, frame);
            return Err(());
        }
        let runtime_state = runtime_state.clone();
        let request_id = request_id.clone();
        let request = request.clone();
        let peer = peer.clone();
        let outgoing_tx = outgoing_tx.clone();
        let close_tx = close_tx.clone();
        let close_requested = Arc::clone(close_requested);
        let health = runtime.transport_health.clone();
        let bound_grant = bound_grant.clone();
        let permit = match inbound_request_admission.try_acquire(
            connection_inbound_request_permits,
            &KernelCommandPriority::Normal,
        ) {
            Ok(permit) => permit,
            Err(error) => {
                health.record_inbound_overload_rejection();
                let _ = try_send_outgoing_frame(
                    &outgoing_tx,
                    &close_tx,
                    &close_requested,
                    &health,
                    KernelOutgoingFrame::Response {
                        request_id,
                        response: Box::new(None),
                        error: Some(KernelTransportError {
                            code: "kernel_request_overloaded".into(),
                            message: format!("kernel request admission queue overloaded: {error}"),
                            retryable: true,
                        }),
                    },
                    None,
                    None,
                );
                return Err(());
            }
        };
        tokio::spawn(async move {
            let _permit = permit;
            let response = runtime_state.request_kernel_access(peer, request).await;
            let (response, error) = match response {
                Ok(grant) => {
                    // A socket never changes authority after its first admission.
                    // Existing subscriptions and queued deliveries retain that grant.
                    bound_grant
                        .lock()
                        .expect("bound grant poisoned")
                        .get_or_insert_with(|| grant.grant_id.clone());
                    (
                        Some(
                            serde_json::to_value(
                                crate::local::LocalDaemonResponse::KernelAccessGranted { grant },
                            )
                            .unwrap(),
                        ),
                        None,
                    )
                }
                Err(e) => (None, Some(map_kernel_error(&e))),
            };
            let _ = try_send_outgoing_frame(
                &outgoing_tx,
                &close_tx,
                &close_requested,
                &health,
                KernelOutgoingFrame::Response {
                    request_id,
                    response: Box::new(response),
                    error,
                },
                None,
                None,
            );
        });
        return Err(());
    }
    let (grant, allowed) = {
        let mut binding = bound_grant.lock().expect("bound grant poisoned");
        let mut candidates = runtime_state.access_grants_for(peer);
        candidates.retain(|grant| {
            binding
                .as_ref()
                .is_none_or(|id| id == &grant.summary.grant_id)
        });
        let grant = candidates.first().cloned();
        let allowed = grant.as_ref().is_some_and(|grant| match frame {
            KernelIncomingFrame::Request { request, .. } => runtime_state
                .authorize_external_request(&grant.summary.grant_id, request)
                .is_ok(),
            KernelIncomingFrame::Subscribe {
                session_id,
                attachment_id,
                subscription_scope,
                ..
            } => {
                kernel_subscription_scope(subscription_scope.as_deref())
                    == KernelSubscriptionScope::WaitingRoomInventory
                    || router
                        .session_id_for_attachment_access(attachment_id)
                        .as_deref()
                        == Some(session_id.as_str())
            }
            KernelIncomingFrame::Unsubscribe { .. } => true,
        });
        if allowed {
            binding.get_or_insert_with(|| {
                grant
                    .as_ref()
                    .expect("allowed grant")
                    .summary
                    .grant_id
                    .clone()
            });
        }
        (grant, allowed)
    };
    if !allowed {
        deny_frame(runtime, outgoing_tx, close_tx, close_requested, frame);
        return Err(());
    }
    let grant = grant.expect("allowed grant");
    let mut caller = router
        .local_command_caller(
            KernelCommandSource::LocalIpc,
            KernelConnectionClass::ExternalAgent,
        )
        .await;
    caller.caller_id = grant.summary.grant_id;
    caller.user_id = Some(grant.summary.owner_user_id);
    Ok(caller)
}

fn deny_frame(
    runtime: &KernelTransportRuntime,
    outgoing_tx: &KernelOutgoingSender,
    close_tx: &mpsc::UnboundedSender<ConnectionCloseCommand>,
    close_requested: &Arc<AtomicBool>,
    frame: &KernelIncomingFrame,
) {
    let request_id = match frame {
        KernelIncomingFrame::Request { request_id, .. }
        | KernelIncomingFrame::Subscribe { request_id, .. }
        | KernelIncomingFrame::Unsubscribe { request_id } => request_id.clone(),
    };
    let _ = try_send_outgoing_frame(
        outgoing_tx,
        close_tx,
        close_requested,
        &runtime.transport_health,
        KernelOutgoingFrame::Response {
            request_id,
            response: Box::new(None),
            error: Some(KernelTransportError {
                code: "kernel_access_denied".into(),
                message: "Unix peer has no live authority for this request".into(),
                retryable: false,
            }),
        },
        None,
        None,
    );
}

pub(super) fn delivery_live(
    runtime: &crate::runtime::state::KernelRuntimeState,
    peer: Option<&crate::runtime::kernel_access::process::ProcessIdentity>,
    bound: &std::sync::Mutex<Option<String>>,
) -> bool {
    peer.is_none_or(|peer| {
        peer.alive()
            && bound
                .lock()
                .expect("bound grant poisoned")
                .as_deref()
                .is_none_or(|id| {
                    if id.starts_with("sudo:") {
                        runtime.sudo_peer_live(id, peer)
                    } else {
                        runtime.access_grant_live(id, peer)
                    }
                })
    })
}

pub(super) async fn serve_connection(
    runtime: Arc<KernelTransportRuntime>,
    router: Arc<CommandRouter>,
    admission: InboundRequestAdmission,
    stream: tokio::net::UnixStream,
) {
    let Ok(peer) = crate::runtime::kernel_access::process::peer(&stream) else {
        return;
    };
    let socket = tokio::time::timeout(
        Duration::from_secs(10),
        accept_hdr_async(
            stream,
            |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                if request.headers().contains_key("origin")
                    || request.headers().contains_key("authorization")
                {
                    let mut error =
                        ErrorResponse::new(Some("Unix access uses OS identity only".into()));
                    *error.status_mut() = StatusCode::FORBIDDEN;
                    return Err(error);
                }
                Ok(response)
            },
        ),
    )
    .await;
    if let Ok(Ok(socket)) = socket {
        let _ = serve_kernel_socket(
            runtime,
            router,
            admission,
            socket,
            KernelConnectionClass::Unauthenticated,
            Some(peer),
        )
        .await;
    }
}

/// Stable across reconnects, and separate for each OS peer and admitted grant.
pub(super) fn command_cache_id(peer: &ProcessIdentity, command: &KernelCommand) -> String {
    serde_json::json!([
        "unix-peer-command",
        [
            peer.pid,
            peer.uid,
            peer.start,
            peer.version,
            peer.executable
        ],
        command.caller.caller_id,
        command.command_id,
    ])
    .to_string()
}
