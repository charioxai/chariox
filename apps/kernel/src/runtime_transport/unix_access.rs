//! OS admission and grant checks for the Unix websocket transport.
use super::*;
use crate::runtime::command::KernelCaller;
use crate::runtime::kernel_access::{process::ProcessIdentity, Refusal};

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
        let verdict = match frame {
            // A sudo window is one turn on this kernel; it does not reach other kernels.
            KernelIncomingFrame::Request {
                kernel: Some(_), ..
            }
            | KernelIncomingFrame::Subscribe {
                kernel: Some(_), ..
            } => Err(Refusal::NotCovered),
            KernelIncomingFrame::Request { request, .. } => runtime_state
                .authorize_sudo_request(&turn.entry_id, request)
                .map(|_| ())
                .map_err(|error| Refusal::of(&error)),
            KernelIncomingFrame::Subscribe {
                session_id,
                attachment_id,
                subscription_scope,
                ..
            } => (session_id == &turn.session_id
                && runtime_state
                    .attachment_owner_user_id(attachment_id)
                    .await
                    .is_ok_and(|owner| owner == turn.owner_user_id)
                && kernel_subscription_scope(subscription_scope.as_deref())
                    != KernelSubscriptionScope::WaitingRoomInventory
                && router
                    .session_id_for_attachment_access(attachment_id)
                    .as_deref()
                    == Some(session_id.as_str()))
            .then_some(())
            .ok_or(Refusal::NotCovered),
            KernelIncomingFrame::Unsubscribe { .. } => Ok(()),
        };
        if let Err(refusal) = verdict {
            // Refused sudo operations must not fall back to an external grant or
            // raise an access popup through the unauthenticated admission path.
            deny_frame(
                runtime,
                outgoing_tx,
                close_tx,
                close_requested,
                frame,
                refusal,
            );
            return Err(());
        }
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
    if bound.as_deref().is_some_and(|id| id.starts_with("sudo:")) {
        deny_frame(
            runtime,
            outgoing_tx,
            close_tx,
            close_requested,
            frame,
            Refusal::Ended,
        );
        return Err(());
    }
    // A grant request that names a kernel is a gateway request: the gateway
    // policy below refuses it, so it never raises a popup on this kernel.
    if let KernelIncomingFrame::Request {
        request_id,
        request: crate::local::LocalDaemonRequest::RequestKernelAccess(request),
        kernel: None,
        ..
    } = frame
    {
        let covered = match runtime_state.access_grants_for(peer) {
            Ok(grants) => !grants.is_empty(),
            Err(refusal) => {
                deny_frame(
                    runtime,
                    outgoing_tx,
                    close_tx,
                    close_requested,
                    frame,
                    refusal,
                );
                return Err(());
            }
        };
        if bound.is_some() || covered {
            let refusal = if covered {
                Refusal::AlreadyGranted
            } else {
                runtime_state.access_end_for(peer).unwrap_or(Refusal::Ended)
            };
            deny_frame(
                runtime,
                outgoing_tx,
                close_tx,
                close_requested,
                frame,
                refusal,
            );
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
                        kernel: None,
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
                Err(e) => (
                    None,
                    Some(match Refusal::exact(&e) {
                        Some(refusal) => refusal.transport_error(),
                        None => map_kernel_error(&e),
                    }),
                ),
            };
            let _ = try_send_outgoing_frame(
                &outgoing_tx,
                &close_tx,
                &close_requested,
                &health,
                KernelOutgoingFrame::Response {
                    kernel: None,
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
    // A covered process names a background process: the covering needs this socket's OS peer.
    if let KernelIncomingFrame::Request {
        request_id,
        request: crate::local::LocalDaemonRequest::CoverKernelAccessProcess(request),
        kernel: None,
        ..
    } = frame
    {
        let covered = {
            let mut binding = bound_grant.lock().expect("bound grant poisoned");
            let covered =
                runtime_state.cover_kernel_access_process(peer, binding.as_deref(), request);
            if let Ok((grant_id, _)) = &covered {
                binding.get_or_insert_with(|| grant_id.clone());
            }
            covered
        };
        let (response, error) = match covered {
            Ok((grant_id, process)) => (
                Some(
                    serde_json::to_value(crate::local::LocalDaemonResponse::KernelAccessCovered {
                        grant_id,
                        process,
                    })
                    .unwrap(),
                ),
                None,
            ),
            Err(e) => (
                None,
                Some(match Refusal::exact(&e) {
                    Some(refusal) => refusal.transport_error(),
                    None => map_kernel_error(&e),
                }),
            ),
        };
        let _ = try_send_outgoing_frame(
            outgoing_tx,
            close_tx,
            close_requested,
            &runtime.transport_health,
            KernelOutgoingFrame::Response {
                kernel: None,
                request_id: request_id.clone(),
                response: Box::new(response),
                error,
            },
            None,
            None,
        );
        return Err(());
    }
    let (grant, verdict) = {
        let mut binding = bound_grant.lock().expect("bound grant poisoned");
        let candidates = runtime_state.access_grants_for(peer);
        let grant = candidates.as_ref().ok().and_then(|candidates| {
            candidates
                .iter()
                .find(|grant| {
                    binding
                        .as_ref()
                        .is_none_or(|id| id == &grant.summary.grant_id)
                })
                .cloned()
        });
        let verdict = match &grant {
            None if candidates.is_err() => Err(Refusal::Unconfirmed),
            // A socket stays bound to its first grant: once that is gone, so is access.
            None => match runtime_state.access_end_for(peer) {
                Some(ended) => Err(ended),
                None if binding.is_some() => Err(Refusal::Ended),
                None => Err(Refusal::NotCovered),
            },
            Some(grant) => match frame {
                // The target kernel owns the interaction and the session; the
                // origin checks the grant and the request shape only.
                KernelIncomingFrame::Request {
                    request,
                    kernel: Some(_),
                    ..
                } => runtime_state
                    .authorize_gateway_request(&grant.summary.grant_id, request)
                    .map_err(|error| Refusal::of(&error)),
                KernelIncomingFrame::Subscribe {
                    kernel: Some(_), ..
                } => Ok(()),
                KernelIncomingFrame::Request { request, .. } => runtime_state
                    .authorize_external_request(&grant.summary.grant_id, request)
                    .map_err(|error| Refusal::of(&error)),
                KernelIncomingFrame::Subscribe {
                    session_id,
                    attachment_id,
                    subscription_scope,
                    ..
                } => (kernel_subscription_scope(subscription_scope.as_deref())
                    == KernelSubscriptionScope::WaitingRoomInventory
                    || router
                        .session_id_for_attachment_access(attachment_id)
                        .as_deref()
                        == Some(session_id.as_str()))
                .then_some(())
                .ok_or(Refusal::NotCovered),
                KernelIncomingFrame::Unsubscribe { .. } => Ok(()),
            },
        };
        if let (Some(grant), Ok(())) = (&grant, &verdict) {
            binding.get_or_insert_with(|| grant.summary.grant_id.clone());
        }
        (grant, verdict)
    };
    if let Err(refusal) = verdict {
        deny_frame(
            runtime,
            outgoing_tx,
            close_tx,
            close_requested,
            frame,
            refusal,
        );
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
    refusal: Refusal,
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
            kernel: None,
            request_id,
            response: Box::new(None),
            error: Some(refusal.transport_error()),
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

/// What a grant holder asks of the kernel gateway.
pub(super) enum GatewayRoute {
    List,
    Forward {
        kernel: String,
        request: Box<crate::local::LocalDaemonRequest>,
    },
}

const SUBSCRIPTION_SILENCE: std::time::Duration = std::time::Duration::from_secs(30);

/// Serves an admitted subscription on another kernel: the target's events are
/// relayed to this connection until the agent unsubscribes or the kernel stops
/// answering, which the agent is told with the subscribe request's own id.
pub(super) async fn serve_gateway_subscription(
    connection: &IncomingConnection<'_>,
    request_id: String,
    kernel: String,
    request: crate::runtime::kernel_gateway::SubscribeRequest,
) {
    let IncomingConnection {
        runtime,
        router,
        connection_state,
        outgoing_tx,
        close_tx,
        close_requested,
        ..
    } = *connection;
    let gateway = Arc::clone(router.kernel_gateway());
    let health = runtime.transport_health.clone();
    let (outgoing_tx, close_tx, close_requested) = (
        outgoing_tx.clone(),
        close_tx.clone(),
        Arc::clone(close_requested),
    );
    let origin = crate::runtime::kernel_gateway::Origin {
        grant_id: request.grant_id.clone(),
        harness: "watch".into(),
    };
    let task = tokio::spawn(async move {
        let send = |frame| {
            try_send_outgoing_frame(
                &outgoing_tx,
                &close_tx,
                &close_requested,
                &health,
                frame,
                None,
                None,
            )
        };
        let subscription = match gateway.subscribe(&kernel, request).await {
            Ok(subscription) => subscription,
            Err(failure) => {
                send(KernelOutgoingFrame::Response {
                    kernel: Some(kernel),
                    request_id,
                    response: Box::new(None),
                    error: Some(failure.transport_error()),
                });
                return;
            }
        };
        let crate::runtime::kernel_gateway::ForwardedSubscription {
            ack, mut events, ..
        } = subscription;
        if !send(KernelOutgoingFrame::Response {
            kernel: Some(kernel.clone()),
            request_id: request_id.clone(),
            response: Box::new(Some(ack)),
            error: None,
        }) {
            return;
        }
        let mut last_event_id = None;
        // The kernel sends a heartbeat every 5 s; silence for six of them means it is gone
        // (the relay does not say so when a target dies with a subscription open).
        let mut quiet_since = tokio::time::Instant::now();
        let probed_kernel = kernel.clone();
        let unaccepted = gateway.until_unaccepted(&probed_kernel, &origin);
        tokio::pin!(unaccepted);
        loop {
            let (event_id, body) = tokio::select! {
                next = events.recv() => match next {
                    Some(event) => event,
                    None => break,
                },
                _ = tokio::time::sleep_until(quiet_since + SUBSCRIPTION_SILENCE) => break,
                error = &mut unaccepted => {
                    send(KernelOutgoingFrame::Response {
                        kernel: Some(kernel),
                        request_id,
                        response: Box::new(None),
                        error: Some(error.transport_error()),
                    });
                    return;
                }
            };
            quiet_since = tokio::time::Instant::now();
            let Ok(event) = serde_json::from_slice::<KernelEvent>(&body) else {
                continue;
            };
            last_event_id = Some(event_id);
            // An event an external peer may not see is skipped, not offered to the writer:
            // a denied frame stops it for good.
            if !super::outbound::is_public_event(&event) {
                continue;
            }
            if !send(KernelOutgoingFrame::Event {
                event_id,
                kernel: Some(kernel.clone()),
                event: Box::new(event),
            }) {
                return;
            }
        }
        // A cut link is also how a revoked gateway finds out: say so instead of "resubscribe".
        let revoked = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            gateway.revoked_after_loss(&kernel),
        )
        .await
        .ok()
        .flatten();
        let ended = revoked.unwrap_or_else(|| {
            crate::runtime::kernel_gateway::GatewayError::SubscriptionEnded {
                name: kernel.clone(),
                last_event_id,
            }
        });
        send(KernelOutgoingFrame::Response {
            kernel: Some(kernel),
            request_id,
            response: Box::new(None),
            error: Some(ended.transport_error()),
        });
    });
    let mut state = connection_state.lock().await;
    if let Some(previous) = state.watch_task.replace(task) {
        previous.abort();
    }
}

/// Serves an admitted gateway request and answers on the same connection. The
/// permit bounds concurrent gateway requests like any other request.
pub(super) fn serve_gateway(
    connection: &IncomingConnection<'_>,
    caller: &KernelCaller,
    request_id: String,
    command_id: Option<String>,
    route: GatewayRoute,
) {
    let IncomingConnection {
        runtime,
        router,
        inbound_request_admission,
        connection_inbound_request_permits,
        outgoing_tx,
        close_tx,
        close_requested,
        ..
    } = *connection;
    let permit = match inbound_request_admission.try_acquire(
        connection_inbound_request_permits,
        &KernelCommandPriority::Normal,
    ) {
        Ok(permit) => permit,
        Err(error) => {
            runtime.transport_health.record_inbound_overload_rejection();
            let _ = try_send_outgoing_frame(
                outgoing_tx,
                close_tx,
                close_requested,
                &runtime.transport_health,
                KernelOutgoingFrame::Response {
                    kernel: None,
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
            return;
        }
    };
    let grant_id = caller.caller_id.clone();
    let harness = router
        .runtime_state()
        .access_grant_summary(&grant_id)
        .map(|grant| crate::runtime::kernel_gateway::harness_name(&grant))
        .unwrap_or_else(|| "unknown".into());
    let router = Arc::clone(router);
    let health = runtime.transport_health.clone();
    let (outgoing_tx, close_tx, close_requested) = (
        outgoing_tx.clone(),
        close_tx.clone(),
        Arc::clone(close_requested),
    );
    tokio::spawn(async move {
        let _permit = permit;
        let command_id = command_id.unwrap_or_else(|| request_id.clone());
        let origin = crate::runtime::kernel_gateway::Origin { grant_id, harness };
        let echoed = match &route {
            GatewayRoute::Forward { kernel, .. } => Some(kernel.clone()),
            GatewayRoute::List => None,
        };
        let started = std::time::Instant::now();
        let audited = match &route {
            GatewayRoute::Forward { kernel, request }
                if !crate::runtime::kernel_gateway::ledger::is_repeatable(request) =>
            {
                Some((
                    kernel.clone(),
                    crate::runtime::kernel_gateway::ledger::variant_name(request),
                ))
            }
            _ => None,
        };
        let outcome = match route {
            GatewayRoute::Forward { kernel, request } if crate::runtime::state::is_outside_event_request(&request)=>Err(crate::runtime::kernel_gateway::GatewayError::Remote { name:kernel,code:"events_local_only".into(),message:"event requests run on your local kernel; use chariox events subscribe --kernel NAME to follow another kernel".into(),retryable:false }),
            GatewayRoute::Forward { kernel, request } => {
                let gateway = router.kernel_gateway();
                if matches!(
                    *request,
                    crate::local::LocalDaemonRequest::RequestKernelSudo(_)
                ) {
                    gateway.watch_sudo_grant(&kernel, &origin);
                }
                let forwarded = gateway.forward(
                    &kernel,
                    &origin,
                    &command_id,
                    &request,
                    crate::runtime::kernel_gateway::ledger::deadline_for(&request),
                );
                // A wait that outlasts the owner revoking the gateway ends with the revoke.
                let mut answered = if crate::runtime::kernel_gateway::ledger::is_long_wait(&request) {
                    tokio::select! {
                        answered = forwarded => answered,
                        error = gateway.until_unaccepted(&kernel, &origin) => Err(error),
                    }
                } else {
                    forwarded.await
                };
                if let Ok(response) = &mut answered {
                    gateway.note_forwarded(&kernel, &origin, &request, response);
                    gateway.follow_forwarded(&origin, &kernel, &request, response).await;
                }
                answered
            }
            GatewayRoute::List => router.kernel_gateway().kernels().await.map(|kernels| {
                serde_json::to_value(crate::local::LocalDaemonResponse::GatewayKernelsListed {
                    kernels,
                })
                .expect("kernel list serializes")
            }),
        };
        if let Some((kernel, kind)) = audited {
            router.runtime_state().audit_gateway_forward(
                &origin.grant_id,
                &kernel,
                &kind,
                outcome
                    .as_ref()
                    .map_or_else(|failure| failure.code(), |_| "ok"),
                u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            );
        }
        let (response, error) = match outcome {
            Ok(value) => (Some(value), None),
            Err(failure) => (None, Some(failure.transport_error())),
        };
        let _ = try_send_outgoing_frame(
            &outgoing_tx,
            &close_tx,
            &close_requested,
            &health,
            KernelOutgoingFrame::Response {
                kernel: echoed,
                request_id,
                response: Box::new(response),
                error,
            },
            None,
            None,
        );
    });
}
