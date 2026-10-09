//! An admitted direct session: relay client frames in, relay client frames out.

use tokio::task::JoinSet;

use super::admission::LocalBrowserSocket;
use super::leases::LocalBrowserLeaseState;
use super::*;
use crate::runtime_transport::CONNECTION_INBOUND_REQUEST_LIMIT;

const SESSION_QUEUE_LIMIT: usize = 1024;
const PING_INTERVAL: Duration = Duration::from_secs(10);
const IDLE_TIMEOUT: Duration = Duration::from_secs(35);

pub(super) async fn run(
    direct: Arc<LocalBrowserDirect>,
    socket: LocalBrowserSocket,
    grant: LocalBrowserGrant,
) {
    let mut identity = grant.identity.clone();
    let lease_id = grant.lease_id.clone();
    let mut lease = direct.activate_lease(&lease_id, &grant);
    let (outgoing_tx, mut priority_rx, mut event_rx) =
        RelayOutgoingSender::channel(SESSION_QUEUE_LIMIT);
    let subscription_tasks: RelaySubscriptionTasks = Arc::new(Mutex::new(BTreeMap::new()));
    let mut requests = JoinSet::new();
    let (mut writer, mut reader) = socket.split();
    let mut shutdown = direct.shutdown.clone();
    let mut authority = direct.authority.subscribe();
    let expiry_ms = grant
        .expires_at_ms
        .saturating_sub(crate::session::unix_epoch_ms());
    let expiry = sleep(Duration::from_millis(expiry_ms));
    tokio::pin!(expiry);
    let mut ping = tokio::time::interval(PING_INTERVAL);
    ping.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut last_read = Instant::now();
    crate::logging::info_with_fields(
        "daemon.local_browser",
        "local browser session admitted",
        serde_json::json!({"subject": identity.subject}),
    );
    let close_reason = loop {
        tokio::select! {
            _ = shutdown.changed() => break "daemon shutting down",
            _ = &mut expiry => {
                if lease.borrow().expires_at_ms > crate::session::unix_epoch_ms() {
                    expiry.as_mut().reset(tokio::time::Instant::now() + lease_remaining(&lease));
                    continue;
                }
                break if identity.expires_at_ms <= crate::session::unix_epoch_ms() { "relay token expired" } else { "local browser lease expired" };
            },
            changed = lease.changed() => {
                if changed.is_err() { break "local browser lease retired"; }
                let renewed = lease.borrow().clone();
                identity = renewed.identity;
                let remaining_ms = renewed.expires_at_ms.saturating_sub(crate::session::unix_epoch_ms());
                expiry.as_mut().reset(tokio::time::Instant::now() + Duration::from_millis(remaining_ms));
            },
            changed = authority.changed() => {
                if changed.is_err() || authority.borrow().as_ref() != Some(&grant.authority) {
                    break "local browser authority revoked";
                }
            }
            _ = ping.tick() => {
                if last_read.elapsed() >= IDLE_TIMEOUT {
                    break "local browser connection idle timeout";
                }
                if guarded_io(&lease, &mut authority, &mut shutdown, writer.send(Message::Ping(Vec::new().into()))).await.is_none_or(|result| result.is_err()) {
                    break "local browser connection closed";
                }
            }
            Some(envelope) = priority_rx.recv() => {
                if !guarded_io(&lease, &mut authority, &mut shutdown, write_client_frame(&mut writer, envelope)).await.unwrap_or(false) {
                    break "local browser connection closed";
                }
            }
            Some(envelope) = event_rx.recv() => {
                if !guarded_io(&lease, &mut authority, &mut shutdown, write_client_frame(&mut writer, envelope)).await.unwrap_or(false) {
                    break "local browser connection closed";
                }
            }
            Some(_) = requests.join_next(), if !requests.is_empty() => {}
            incoming = reader.next() => {
                last_read = Instant::now();
                let payload = match incoming {
                    Some(Ok(Message::Text(payload))) => payload,
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                    _ => break "local browser connection closed",
                };
                let still_admitted = direct.authority.borrow().as_ref() == Some(&grant.authority)
                    && lease.borrow().expires_at_ms > crate::session::unix_epoch_ms();
                if !still_admitted {
                    break "local browser authority revoked";
                }
                identity = lease.borrow().identity.clone();
                match guarded_io(&lease, &mut authority, &mut shutdown, handle_client_frame(
                    &direct,
                    &identity,
                    &outgoing_tx,
                    &subscription_tasks,
                    &mut requests,
                    &payload,
                )).await
                {
                    Some(Ok(())) => {},
                    Some(Err(reason)) => break reason,
                    None => break "local browser authority revoked",
                }
            }
        }
    };
    direct.retire_lease(&lease_id);
    // Accepted commands own their dispatch and cache settlement independently
    // of this socket. A relay retry must observe the original result.
    requests.detach_all();
    abort_subscription_tasks(&direct.router, &subscription_tasks).await;
    let close = serde_json::json!({"kind": "close", "reason": close_reason}).to_string();
    let _ = timeout(
        Duration::from_secs(1),
        writer.send(Message::Text(close.into())),
    )
    .await;
    let _ = timeout(Duration::from_secs(1), writer.close()).await;
    crate::logging::info_with_fields(
        "daemon.local_browser",
        "local browser session closed",
        serde_json::json!({"subject": identity.subject, "reason": close_reason}),
    );
}

async fn guarded_io<F: std::future::Future>(
    lease: &watch::Receiver<LocalBrowserLeaseState>,
    authority: &mut watch::Receiver<Option<LocalBrowserAuthority>>,
    shutdown: &mut watch::Receiver<bool>,
    io: F,
) -> Option<F::Output> {
    let mut live_lease = lease.clone();
    tokio::pin!(io);
    loop {
        let remaining = Duration::from_millis(
            live_lease
                .borrow_and_update()
                .expires_at_ms
                .saturating_sub(crate::session::unix_epoch_ms()),
        );
        if remaining.is_zero() {
            return None;
        }
        tokio::select! {
            biased;
            _ = shutdown.changed() => return None,
            _ = authority.changed() => return None,
            changed = live_lease.changed() => {
                if changed.is_err() { return None; }
            }
            _ = sleep(remaining) => {},
            result = &mut io => return Some(result),
        }
    }
}

fn lease_remaining(lease: &watch::Receiver<LocalBrowserLeaseState>) -> Duration {
    Duration::from_millis(
        lease
            .borrow()
            .expires_at_ms
            .saturating_sub(crate::session::unix_epoch_ms()),
    )
}

async fn handle_client_frame(
    direct: &Arc<LocalBrowserDirect>,
    identity: &RelayCallerIdentity,
    outgoing_tx: &RelayOutgoingSender,
    subscription_tasks: &RelaySubscriptionTasks,
    requests: &mut JoinSet<()>,
    payload: &str,
) -> Result<(), &'static str> {
    let envelope = serde_json::from_str::<RelayEnvelope>(payload)
        .map_err(|_| "local browser sent an invalid frame")?;
    match envelope {
        RelayEnvelope::ClientRequest {
            request_id,
            target,
            encrypted_request,
        } => {
            if let Some(error) = invalid_request(direct, &request_id, &target, requests.len()) {
                return respond(outgoing_tx, request_id, error);
            }
            let direct = Arc::clone(direct);
            let identity = identity.clone();
            let outgoing_tx = outgoing_tx.clone();
            requests.spawn(async move {
                let outcome = handle_daemon_request(
                    &direct.router,
                    &direct.command_sequence,
                    Some(identity),
                    encrypted_request,
                    &direct.command_result_cache,
                    None,
                )
                .await;
                let _ = send_outgoing_envelope(
                    &outgoing_tx,
                    RelayEnvelope::ClientResponse {
                        request_id,
                        encrypted_response: outcome.encrypted_response,
                        error: outcome.error,
                    },
                );
            });
            Ok(())
        }
        RelayEnvelope::ClientSubscribe {
            request_id,
            subscription_id,
            target,
            session_id,
            attachment_id,
            client_public_key,
            subscription_scope,
            resume_from_event_id,
        } => {
            if let Some(error) = invalid_request(direct, &request_id, &target, 0)
                .or_else(|| empty_identifier("subscription_id", &subscription_id))
                .or_else(|| empty_identifier("session_id", &session_id))
                .or_else(|| empty_identifier("attachment_id", &attachment_id))
                .or_else(|| empty_identifier("client_public_key", &client_public_key))
            {
                return respond(outgoing_tx, request_id, error);
            }
            handle_relay_subscribe(
                &direct.router,
                outgoing_tx,
                subscription_tasks,
                &direct.event_runtime,
                request_id,
                subscription_id,
                session_id,
                attachment_id,
                Some(identity.clone()),
                client_public_key,
                subscription_scope,
                resume_from_event_id,
            )
            .await
            .map_err(|_| "local browser outgoing queue overloaded")
        }
        RelayEnvelope::ClientUnsubscribe {
            request_id,
            subscription_id,
            client_public_key,
        } => handle_relay_unsubscribe(
            &direct.router,
            outgoing_tx,
            subscription_tasks,
            request_id,
            subscription_id,
            client_public_key,
        )
        .await
        .map_err(|_| "local browser outgoing queue overloaded"),
        _ => Err("local browser sent a frame that is not a client request"),
    }
}

fn invalid_request(
    direct: &LocalBrowserDirect,
    request_id: &str,
    target: &ClientTarget,
    in_flight: usize,
) -> Option<RelayError> {
    if let Some(error) = empty_identifier("request_id", request_id) {
        return Some(error);
    }
    let config = direct.router.relay_config_snapshot();
    let addressed_here = target.daemon_id.as_deref() == Some(config.daemon_id.as_str())
        || (target.daemon_id.is_none()
            && target.daemon_alias.is_some()
            && target.daemon_alias == config.daemon_alias);
    if !addressed_here {
        return Some(relay_error(
            "target_mismatch",
            "local browser connection is bound to another kernel",
            false,
        ));
    }
    (in_flight >= CONNECTION_INBOUND_REQUEST_LIMIT).then(|| {
        relay_error(
            "target_backpressure",
            "local browser request queue is full",
            true,
        )
    })
}

fn empty_identifier(field: &str, value: &str) -> Option<RelayError> {
    value.trim().is_empty().then(|| {
        relay_error(
            "invalid_runtime_identifier",
            &format!("{field} must not be empty"),
            false,
        )
    })
}

fn respond(
    outgoing_tx: &RelayOutgoingSender,
    request_id: String,
    error: RelayError,
) -> Result<(), &'static str> {
    send_outgoing_envelope(
        outgoing_tx,
        RelayEnvelope::ClientResponse {
            request_id,
            encrypted_response: None,
            error: Some(error),
        },
    )
    .map_err(|_| "local browser outgoing queue overloaded")
}

/// The shared subscription/response code speaks daemon-side envelopes; the
/// browser expects the client-side ones the relay would have produced.
async fn write_client_frame<S>(writer: &mut S, envelope: RelayEnvelope) -> bool
where
    S: futures_util::Sink<Message> + Unpin,
{
    let envelope = match envelope {
        RelayEnvelope::DaemonResponse {
            relay_request_id,
            encrypted_response,
            error,
        } => RelayEnvelope::ClientResponse {
            request_id: relay_request_id,
            encrypted_response,
            error,
        },
        RelayEnvelope::DaemonEvent {
            subscription_id,
            event_id,
            encrypted_event,
        } => RelayEnvelope::ClientEvent {
            subscription_id,
            event_id,
            encrypted_event,
        },
        envelope @ (RelayEnvelope::ClientResponse { .. } | RelayEnvelope::Close { .. }) => envelope,
        _ => return true,
    };
    match serde_json::to_string(&envelope) {
        Ok(text) => writer.send(Message::Text(text.into())).await.is_ok(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn changing_deadline(shorten: bool) {
        let browser = super::super::tests::Browser::new();
        let state = LocalBrowserLeaseState {
            expires_at_ms: crate::session::unix_epoch_ms() + if shorten { 500 } else { 100 },
            identity: browser.identity(),
        };
        let (lease_tx, lease) = watch::channel(state.clone());
        let (_authority_tx, mut authority) = watch::channel(None);
        let (_shutdown_tx, mut shutdown) = watch::channel(false);
        let (done, io) = tokio::sync::oneshot::channel::<()>();
        let update = tokio::spawn(async move {
            sleep(Duration::from_millis(20)).await;
            let mut renewed = state;
            renewed.expires_at_ms =
                crate::session::unix_epoch_ms() + if shorten { 50 } else { 500 };
            lease_tx.send_replace(renewed);
            sleep(Duration::from_millis(150)).await;
            let _ = done.send(());
            sleep(Duration::from_millis(50)).await;
        });
        let result = guarded_io(&lease, &mut authority, &mut shutdown, io).await;
        if shorten {
            assert!(
                result.is_none(),
                "I/O completed after the shortened live deadline"
            );
        } else {
            assert!(
                result.is_some_and(|result| result.is_ok()),
                "renewal did not extend the pending I/O guard"
            );
        }
        update.await.unwrap();
    }

    #[tokio::test]
    async fn mp11_pending_io_observes_extended_lease() {
        changing_deadline(false).await;
    }

    #[tokio::test]
    async fn mp11_pending_io_observes_shortened_lease() {
        changing_deadline(true).await;
    }
}
