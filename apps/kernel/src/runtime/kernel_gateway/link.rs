//! One relay client connection to one target kernel, shared by every forwarded
//! request: end-to-end encrypted with the gateway key, like a remote terminal.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chariox_relay::protocol::{ClientTarget, RelayEnvelope, RelayError};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

use crate::transport::relay_crypto;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const IDLE_CLOSE: Duration = Duration::from_secs(5 * 60);
/// Hosted client tokens last five minutes; a link is replaced well before.
pub(super) const MAX_LINK_AGE: Duration = Duration::from_secs(4 * 60);

pub(super) enum LinkFailure {
    /// The relay or the target refused or failed the request.
    Relay(RelayError),
    /// The connection ended with the request in flight.
    Closed,
    TimedOut,
    /// The reply did not come from the pinned target key.
    IdentityMismatch,
}

type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Result<Vec<u8>, LinkFailure>>>>>;

/// One event of a subscription on the target: its id on the target's stream and
/// the decrypted event body. The channel closes when the link does.
pub(super) type EventChannel = mpsc::UnboundedSender<(u64, Vec<u8>)>;
type Subscriptions = Arc<Mutex<HashMap<String, EventChannel>>>;

/// What the target needs to start a subscription for this link's key.
pub(super) struct SubscribeParams {
    pub(super) session_id: String,
    pub(super) attachment_id: String,
    pub(super) scope: Option<String>,
    pub(super) resume_from_event_id: Option<u64>,
}

pub(super) struct LinkParams {
    pub(super) relay_url: String,
    pub(super) auth_token: String,
    pub(super) target_kernel_id: String,
    /// The key the target must announce; None only while pairing, when the
    /// relay's announcement is learned and then pinned by the caller.
    pub(super) target_public_key: Option<String>,
    pub(super) client_private_key: String,
}

pub(super) struct Link {
    outgoing: mpsc::UnboundedSender<Message>,
    pending: Pending,
    subscriptions: Subscriptions,
    alive: Arc<AtomicBool>,
    opened_at: Instant,
    /// When the relay token this connection runs on was issued; renewal restarts it.
    token_issued_at: Mutex<Instant>,
    target: ClientTarget,
    target_public_key: String,
    client_private_key: String,
    client_public_key: String,
}

impl Link {
    pub(super) async fn open(params: LinkParams) -> Result<Self, LinkFailure> {
        let (mut socket, _) = tokio::time::timeout(
            CONNECT_TIMEOUT,
            crate::transport::relay_socket_connect::connect(&params.relay_url),
        )
        .await
        .map_err(|_| LinkFailure::TimedOut)?
        .map_err(|error| relay_failure("connect_failed", error.to_string()))?;
        let target = ClientTarget {
            daemon_id: Some(params.target_kernel_id.clone()),
            daemon_alias: None,
        };
        let connect = RelayEnvelope::ClientConnect {
            auth_token: params.auth_token,
            target: target.clone(),
        };
        socket
            .send(Message::Text(
                serde_json::to_string(&connect)
                    .expect("envelope serializes")
                    .into(),
            ))
            .await
            .map_err(|error| relay_failure("connect_failed", error.to_string()))?;
        let mut frames = chariox_relay::frame_transport::FrameReceiver::default();
        let connected = tokio::time::timeout(CONNECT_TIMEOUT, async {
            loop {
                match chariox_relay::frame_transport::read_message(&mut socket, &mut frames).await {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<RelayEnvelope>(&text) {
                            Ok(RelayEnvelope::ClientConnected {
                                daemon_public_key, ..
                            }) => return Ok(daemon_public_key),
                            Ok(RelayEnvelope::Close { reason }) => {
                                return Err(relay_failure("connect_refused", reason))
                            }
                            Ok(RelayEnvelope::ClientResponse {
                                error: Some(error), ..
                            }) => return Err(LinkFailure::Relay(error)),
                            _ => {}
                        }
                    }
                    Some(Ok(Message::Close(frame))) => {
                        let reason = frame.map(|f| f.reason.to_string()).unwrap_or_default();
                        return Err(relay_failure("connect_refused", reason));
                    }
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        return Err(relay_failure("connect_failed", error.to_string()))
                    }
                    None => return Err(LinkFailure::Closed),
                }
            }
        })
        .await
        .map_err(|_| LinkFailure::TimedOut)??;
        if params
            .target_public_key
            .as_ref()
            .is_some_and(|pinned| *pinned != connected)
        {
            return Err(LinkFailure::IdentityMismatch);
        }

        let (mut sink, mut stream) = socket.split();
        let (outgoing, mut outgoing_rx) = mpsc::unbounded_channel::<Message>();
        let pending: Pending = Arc::default();
        let subscriptions: Subscriptions = Arc::default();
        let alive = Arc::new(AtomicBool::new(true));
        tokio::spawn({
            let alive = Arc::clone(&alive);
            async move {
                while let Some(message) = outgoing_rx.recv().await {
                    let closing = matches!(message, Message::Close(_));
                    if sink.send(message).await.is_err() || closing {
                        break;
                    }
                }
                alive.store(false, Ordering::SeqCst);
                let _ = sink.close().await;
            }
        });
        tokio::spawn({
            let alive = Arc::clone(&alive);
            let pending = Arc::clone(&pending);
            let subscriptions = Arc::clone(&subscriptions);
            let outgoing = outgoing.clone();
            let client_private_key = params.client_private_key.clone();
            let target_public_key = connected.clone();
            async move {
                let mut last_activity = Instant::now();
                loop {
                    let next = tokio::time::timeout(Duration::from_secs(30), stream.next()).await;
                    let message = match next {
                        Err(_) => {
                            let idle = pending.lock().expect("pending poisoned").is_empty()
                                && subscriptions
                                    .lock()
                                    .expect("subscriptions poisoned")
                                    .is_empty()
                                && last_activity.elapsed() >= IDLE_CLOSE;
                            if idle {
                                let _ = outgoing.send(Message::Close(None));
                                break;
                            }
                            continue;
                        }
                        Ok(None) | Ok(Some(Err(_))) | Ok(Some(Ok(Message::Close(_)))) => break,
                        Ok(Some(Ok(message))) => message,
                    };
                    match message {
                        Message::Text(text) => match frames.receive(&text) {
                            Ok((text, receipt)) => {
                                if let Some(receipt) = receipt {
                                    let _ = outgoing.send(Message::Text(receipt.into()));
                                }
                                if let Some(text) = text {
                                    last_activity = Instant::now();
                                    route(
                                        &pending,
                                        &subscriptions,
                                        &client_private_key,
                                        &target_public_key,
                                        &text,
                                    );
                                }
                            }
                            Err(_) => break,
                        },
                        Message::Ping(payload) => {
                            let _ = outgoing.send(Message::Pong(payload));
                        }
                        _ => {}
                    }
                }
                alive.store(false, Ordering::SeqCst);
                subscriptions
                    .lock()
                    .expect("subscriptions poisoned")
                    .clear();
                let senders: Vec<_> = pending.lock().expect("pending poisoned").drain().collect();
                for (_, sender) in senders {
                    let _ = sender.send(Err(LinkFailure::Closed));
                }
            }
        });
        let client_public_key =
            relay_crypto::public_key_from_private_key_base64(&params.client_private_key)
                .map_err(|error| relay_failure("encrypt_failed", error.to_string()))?;
        Ok(Self {
            outgoing,
            pending,
            subscriptions,
            alive,
            opened_at: Instant::now(),
            token_issued_at: Mutex::new(Instant::now()),
            target,
            target_public_key: connected,
            client_private_key: params.client_private_key,
            client_public_key,
        })
    }

    pub(super) fn usable(&self) -> bool {
        self.alive.load(Ordering::SeqCst) && self.opened_at.elapsed() < MAX_LINK_AGE
    }

    /// Sends one encrypted request and waits for its decrypted reply bytes.
    pub(super) async fn request(
        &self,
        request_id: String,
        plaintext: &[u8],
        deadline: Duration,
    ) -> Result<Vec<u8>, LinkFailure> {
        let encrypted_request = relay_crypto::encrypt_payload_for_peer(
            &self.client_private_key,
            &self.target_public_key,
            plaintext,
        )
        .map_err(|error| relay_failure("encrypt_failed", error.to_string()))?;
        let (reply, answer) = oneshot::channel();
        self.pending
            .lock()
            .expect("pending poisoned")
            .insert(request_id.clone(), reply);
        let envelope = RelayEnvelope::ClientRequest {
            request_id: request_id.clone(),
            target: self.target.clone(),
            encrypted_request,
        };
        let text = serde_json::to_string(&envelope).expect("envelope serializes");
        if self.outgoing.send(Message::Text(text.into())).is_err() {
            self.pending
                .lock()
                .expect("pending poisoned")
                .remove(&request_id);
            return Err(LinkFailure::Closed);
        }
        match tokio::time::timeout(deadline, answer).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(LinkFailure::Closed),
            Err(_) => {
                self.pending
                    .lock()
                    .expect("pending poisoned")
                    .remove(&request_id);
                Err(LinkFailure::TimedOut)
            }
        }
    }
}

impl Link {
    /// Starts a subscription on the target and returns its acknowledgement
    /// with the channel its events arrive on.
    pub(super) async fn subscribe(
        &self,
        subscription_id: String,
        params: SubscribeParams,
        deadline: Duration,
    ) -> Result<(Vec<u8>, mpsc::UnboundedReceiver<(u64, Vec<u8>)>), LinkFailure> {
        let (events, received) = mpsc::unbounded_channel();
        self.subscriptions
            .lock()
            .expect("subscriptions poisoned")
            .insert(subscription_id.clone(), events);
        let request_id = format!("{subscription_id}-subscribe");
        let (reply, answer) = oneshot::channel();
        self.pending
            .lock()
            .expect("pending poisoned")
            .insert(request_id.clone(), reply);
        // Dropped before the ack (cancelled, failed or late): the target may already
        // stream to us, so it is told to stop and nothing stays registered here.
        let mut abandon = Abandon {
            link: self,
            subscription_id: &subscription_id,
            request_id: &request_id,
            armed: true,
        };
        let envelope = RelayEnvelope::ClientSubscribe {
            request_id: request_id.clone(),
            subscription_id: subscription_id.clone(),
            target: self.target.clone(),
            session_id: params.session_id,
            attachment_id: params.attachment_id,
            client_public_key: self.client_public_key.clone(),
            subscription_scope: params.scope,
            resume_from_event_id: params.resume_from_event_id,
            heartbeat_interval_ms: None,
        };
        let text = serde_json::to_string(&envelope).expect("envelope serializes");
        if self.outgoing.send(Message::Text(text.into())).is_err() {
            return Err(LinkFailure::Closed);
        }
        let ack = match tokio::time::timeout(deadline, answer).await {
            Ok(Ok(result)) => result?,
            Ok(Err(_)) => return Err(LinkFailure::Closed),
            Err(_) => return Err(LinkFailure::TimedOut),
        };
        abandon.armed = false;
        Ok((ack, received))
    }

    /// Tells the target to stop a subscription; the link stays open.
    pub(super) fn unsubscribe(&self, subscription_id: &str) {
        self.subscriptions
            .lock()
            .expect("subscriptions poisoned")
            .remove(subscription_id);
        let envelope = RelayEnvelope::ClientUnsubscribe {
            request_id: format!("{subscription_id}-unsubscribe"),
            subscription_id: subscription_id.to_string(),
            client_public_key: self.client_public_key.clone(),
        };
        let text = serde_json::to_string(&envelope).expect("envelope serializes");
        let _ = self.outgoing.send(Message::Text(text.into()));
    }

    /// Replaces the relay token of an open connection before the old one
    /// expires; routes and subscriptions stay.
    pub(super) fn renew(&self, auth_token: String) -> bool {
        let envelope = RelayEnvelope::ClientConnect {
            auth_token,
            target: self.target.clone(),
        };
        let text = serde_json::to_string(&envelope).expect("envelope serializes");
        let sent = self.outgoing.send(Message::Text(text.into())).is_ok();
        if sent {
            *self.token_issued_at.lock().expect("token time poisoned") = Instant::now();
        }
        sent
    }

    /// When the token this connection runs on is due for renewal. A cached link is reused for
    /// four minutes, so a request starting on it may find the token already most of the way
    /// to its five-minute expiry.
    pub(super) fn token_renewal_due(&self, renew_every: Duration) -> Instant {
        *self.token_issued_at.lock().expect("token time poisoned") + renew_every
    }

    /// The key the target announced (and, when pinned, proved).
    pub(super) fn target_public_key(&self) -> &str {
        &self.target_public_key
    }

    pub(super) fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
}

/// Undoes a subscription that did not complete; see `Link::subscribe`.
struct Abandon<'a> {
    link: &'a Link,
    subscription_id: &'a str,
    request_id: &'a str,
    armed: bool,
}

impl Drop for Abandon<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.link
                .pending
                .lock()
                .expect("pending poisoned")
                .remove(self.request_id);
            self.link.unsubscribe(self.subscription_id);
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        let _ = self.outgoing.send(Message::Close(None));
    }
}

fn route(
    pending: &Pending,
    subscriptions: &Subscriptions,
    client_private_key: &str,
    target_public_key: &str,
    text: &str,
) {
    match serde_json::from_str::<RelayEnvelope>(text) {
        Ok(RelayEnvelope::ClientEvent {
            subscription_id,
            event_id,
            encrypted_event,
        }) => {
            let Ok(decrypted) =
                relay_crypto::decrypt_payload_for_private_key(client_private_key, &encrypted_event)
            else {
                return;
            };
            if decrypted.sender_public_key != target_public_key {
                return;
            }
            let events = subscriptions.lock().expect("subscriptions poisoned");
            if let Some(channel) = events.get(&subscription_id) {
                let _ = channel.send((event_id, decrypted.plaintext.to_vec()));
            }
        }
        Ok(RelayEnvelope::ClientResponse {
            request_id,
            encrypted_response,
            error,
        }) => settle(
            pending,
            client_private_key,
            target_public_key,
            request_id,
            encrypted_response,
            error,
        ),
        _ => {}
    }
}

fn settle(
    pending: &Pending,
    client_private_key: &str,
    target_public_key: &str,
    request_id: String,
    encrypted_response: Option<chariox_relay::protocol::EncryptedRelayPayload>,
    error: Option<RelayError>,
) {
    let Some(reply) = pending
        .lock()
        .expect("pending poisoned")
        .remove(&request_id)
    else {
        return;
    };
    let result = match (error, encrypted_response) {
        (Some(error), _) => Err(LinkFailure::Relay(error)),
        (None, Some(payload)) => {
            match relay_crypto::decrypt_payload_for_private_key(client_private_key, &payload) {
                Ok(decrypted) if decrypted.sender_public_key == target_public_key => {
                    Ok(decrypted.plaintext.to_vec())
                }
                Ok(_) => Err(LinkFailure::IdentityMismatch),
                Err(error) => Err(relay_failure("decrypt_failed", error.to_string())),
            }
        }
        (None, None) => Err(relay_failure(
            "empty_response",
            "the kernel returned no payload".into(),
        )),
    };
    let _ = reply.send(result);
}

fn relay_failure(code: &str, message: String) -> LinkFailure {
    LinkFailure::Relay(RelayError {
        code: code.to_string(),
        message,
        retryable: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    enum Behaviour {
        Echo,
        DropWithRequestInFlight,
        Refuse,
    }

    /// A relay that accepts one client, announces `announced_key` and answers like a target kernel.
    async fn fake_relay(
        target_private_key: String,
        announced_key: String,
        behaviour: Behaviour,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let Some(Ok(Message::Text(text))) = socket.next().await else {
                return;
            };
            let RelayEnvelope::ClientConnect { target, .. } = serde_json::from_str(&text).unwrap()
            else {
                return;
            };
            let connected = RelayEnvelope::ClientConnected {
                target,
                daemon_public_key: announced_key,
            };
            socket
                .send(Message::Text(
                    serde_json::to_string(&connected).unwrap().into(),
                ))
                .await
                .unwrap();
            while let Some(Ok(Message::Text(text))) = socket.next().await {
                let Ok(RelayEnvelope::ClientRequest {
                    request_id,
                    encrypted_request,
                    ..
                }) = serde_json::from_str(&text)
                else {
                    continue;
                };
                let decrypted = relay_crypto::decrypt_payload_for_private_key(
                    &target_private_key,
                    &encrypted_request,
                )
                .unwrap();
                let response = match behaviour {
                    Behaviour::DropWithRequestInFlight => return,
                    Behaviour::Refuse => RelayEnvelope::ClientResponse {
                        request_id,
                        encrypted_response: None,
                        error: Some(RelayError {
                            code: "target_not_connected".into(),
                            message: "gone".into(),
                            retryable: true,
                        }),
                    },
                    Behaviour::Echo => RelayEnvelope::ClientResponse {
                        request_id,
                        encrypted_response: Some(
                            relay_crypto::encrypt_payload_for_peer(
                                &target_private_key,
                                &decrypted.sender_public_key,
                                &decrypted.plaintext,
                            )
                            .unwrap(),
                        ),
                        error: None,
                    },
                };
                socket
                    .send(Message::Text(
                        serde_json::to_string(&response).unwrap().into(),
                    ))
                    .await
                    .unwrap();
            }
        });
        url
    }

    /// A relay that acknowledges a subscription, streams two events on it and
    /// reports each envelope kind it receives afterwards.
    async fn subscribing_relay(
        target_private_key: String,
        announced_key: String,
        acknowledge: bool,
    ) -> (String, mpsc::UnboundedReceiver<&'static str>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (seen, observed) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let Some(Ok(Message::Text(text))) = socket.next().await else {
                return;
            };
            let RelayEnvelope::ClientConnect { target, .. } = serde_json::from_str(&text).unwrap()
            else {
                return;
            };
            let connected = RelayEnvelope::ClientConnected {
                target,
                daemon_public_key: announced_key,
            };
            socket
                .send(Message::Text(
                    serde_json::to_string(&connected).unwrap().into(),
                ))
                .await
                .unwrap();
            while let Some(Ok(Message::Text(text))) = socket.next().await {
                match serde_json::from_str::<RelayEnvelope>(&text) {
                    Ok(RelayEnvelope::ClientSubscribe {
                        request_id,
                        subscription_id,
                        client_public_key,
                        ..
                    }) => {
                        if !acknowledge {
                            let _ = seen.send("subscribe");
                            continue;
                        }
                        let seal = |body: &[u8]| {
                            relay_crypto::encrypt_payload_for_peer(
                                &target_private_key,
                                &client_public_key,
                                body,
                            )
                            .unwrap()
                        };
                        let mut replies = vec![RelayEnvelope::ClientResponse {
                            request_id,
                            encrypted_response: Some(seal(br#"{"ok":true}"#)),
                            error: None,
                        }];
                        for event_id in [7, 8] {
                            replies.push(RelayEnvelope::ClientEvent {
                                subscription_id: subscription_id.clone(),
                                event_id,
                                encrypted_event: seal(br#"{"event":"heartbeat","session_id":"s"}"#),
                            });
                        }
                        for reply in replies {
                            socket
                                .send(Message::Text(serde_json::to_string(&reply).unwrap().into()))
                                .await
                                .unwrap();
                        }
                        let _ = seen.send("subscribe");
                    }
                    Ok(RelayEnvelope::ClientUnsubscribe { .. }) => {
                        let _ = seen.send("unsubscribe");
                    }
                    Ok(RelayEnvelope::ClientConnect { .. }) => {
                        let _ = seen.send("renew");
                    }
                    _ => {}
                }
            }
        });
        (url, observed)
    }

    async fn open(url: String, target_private_key: &str) -> Result<Link, LinkFailure> {
        Link::open(LinkParams {
            relay_url: url,
            auth_token: "token".into(),
            target_kernel_id: "target".into(),
            target_public_key: Some(
                relay_crypto::public_key_from_private_key_base64(target_private_key).unwrap(),
            ),
            client_private_key: relay_crypto::generate_private_key_base64(),
        })
        .await
    }

    fn keys() -> (String, String) {
        let private = relay_crypto::generate_private_key_base64();
        let public = relay_crypto::public_key_from_private_key_base64(&private).unwrap();
        (private, public)
    }

    #[tokio::test]
    async fn requests_are_encrypted_to_the_pinned_target_and_answers_return_by_id() {
        let (private, public) = keys();
        let url = fake_relay(private.clone(), public, Behaviour::Echo).await;
        let link = open(url, &private).await.ok().expect("link opens");
        let (a, b) = tokio::join!(
            link.request("one".into(), b"first", Duration::from_secs(5)),
            link.request("two".into(), b"second", Duration::from_secs(5)),
        );
        assert_eq!(a.ok().unwrap(), b"first");
        assert_eq!(b.ok().unwrap(), b"second");
        assert!(link.usable());
    }

    // Review: a cached link is reused for four minutes, so the token a request starts on can
    // already be most of the way to its five-minute expiry; renewal is due by token age.
    #[tokio::test]
    async fn token_renewal_is_due_by_the_age_of_the_token_not_of_the_request() {
        let renew_every = Duration::from_secs(210);
        let (private, public) = keys();
        let url = fake_relay(private.clone(), public, Behaviour::Echo).await;
        let link = open(url, &private).await.ok().expect("link opens");
        let fresh = link.token_renewal_due(renew_every);
        assert!(fresh > Instant::now() + Duration::from_secs(150));
        *link.token_issued_at.lock().unwrap() = Instant::now() - Duration::from_secs(230);
        assert!(link.token_renewal_due(renew_every) < Instant::now() + Duration::from_secs(1));
        assert!(link.renew("fresh-token".into()));
        assert!(link.token_renewal_due(renew_every) > Instant::now() + Duration::from_secs(150));
    }

    #[tokio::test]
    async fn a_relay_that_announces_another_key_is_refused_before_any_request() {
        let (private, _) = keys();
        let (_, other_public) = keys();
        let url = fake_relay(private.clone(), other_public, Behaviour::Echo).await;
        assert!(matches!(
            open(url, &private).await,
            Err(LinkFailure::IdentityMismatch)
        ));
    }

    #[tokio::test]
    async fn a_dropped_connection_ends_the_request_in_flight_as_closed() {
        let (private, public) = keys();
        let url = fake_relay(private.clone(), public, Behaviour::DropWithRequestInFlight).await;
        let link = open(url, &private).await.ok().unwrap();
        let outcome = link
            .request("one".into(), b"x", Duration::from_secs(5))
            .await;
        assert!(matches!(outcome, Err(LinkFailure::Closed)));
        assert!(!link.usable());
    }

    #[tokio::test]
    async fn a_relay_refusal_is_returned_as_the_relays_own_error() {
        let (private, public) = keys();
        let url = fake_relay(private.clone(), public, Behaviour::Refuse).await;
        let link = open(url, &private).await.ok().unwrap();
        let outcome = link
            .request("one".into(), b"x", Duration::from_secs(5))
            .await;
        assert!(
            matches!(outcome, Err(LinkFailure::Relay(error)) if error.code == "target_not_connected")
        );
    }

    // EA2-12: a subscription's events arrive decrypted and in order, the
    // connection can renew its token in place, and dropping the subscription
    // tells the target.
    #[tokio::test]
    async fn a_subscription_streams_events_renews_in_place_and_unsubscribes() {
        let (private, public) = keys();
        let (url, mut observed) = subscribing_relay(private.clone(), public, true).await;
        let link = open(url, &private).await.ok().expect("link opens");
        let (ack, mut events) = link
            .subscribe(
                "sub-1".into(),
                SubscribeParams {
                    session_id: "s".into(),
                    attachment_id: "a".into(),
                    scope: None,
                    resume_from_event_id: Some(6),
                },
                Duration::from_secs(5),
            )
            .await
            .ok()
            .expect("subscribed");
        assert_eq!(ack, br#"{"ok":true}"#);
        for expected in [7, 8] {
            let (event_id, body) = events.recv().await.unwrap();
            assert_eq!(event_id, expected);
            assert!(String::from_utf8(body).unwrap().contains("heartbeat"));
        }
        assert_eq!(observed.recv().await, Some("subscribe"));
        assert!(link.renew("fresh-token".into()));
        assert_eq!(observed.recv().await, Some("renew"));
        link.unsubscribe("sub-1");
        assert_eq!(observed.recv().await, Some("unsubscribe"));
        assert!(link.is_alive());
    }

    // A subscription cancelled before its acknowledgement is undone on the target too,
    // so nothing keeps streaming to a connection nobody reads.
    #[tokio::test]
    async fn a_subscription_cancelled_before_its_ack_is_withdrawn_on_the_target() {
        let (private, public) = keys();
        let (url, mut observed) = subscribing_relay(private.clone(), public, false).await;
        let link = open(url, &private).await.ok().expect("link opens");
        let mut attempt = Box::pin(link.subscribe(
            "sub-1".into(),
            SubscribeParams {
                session_id: "s".into(),
                attachment_id: "a".into(),
                scope: None,
                resume_from_event_id: None,
            },
            Duration::from_secs(60),
        ));
        tokio::select! {
            _ = &mut attempt => panic!("the target never acknowledged"),
            seen = observed.recv() => assert_eq!(seen, Some("subscribe")),
        }
        drop(attempt);
        let withdrawn = tokio::time::timeout(Duration::from_secs(5), observed.recv()).await;
        assert_eq!(withdrawn, Ok(Some("unsubscribe")));
        assert!(link.subscriptions.lock().unwrap().is_empty());
        assert!(link.pending.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_dropped_connection_closes_the_event_channel() {
        let (private, public) = keys();
        let url = fake_relay(private.clone(), public, Behaviour::DropWithRequestInFlight).await;
        let link = open(url, &private).await.ok().unwrap();
        let outcome = link
            .subscribe(
                "sub-1".into(),
                SubscribeParams {
                    session_id: "s".into(),
                    attachment_id: "a".into(),
                    scope: None,
                    resume_from_event_id: None,
                },
                Duration::from_secs(5),
            )
            .await;
        assert!(matches!(
            outcome,
            Err(LinkFailure::Closed) | Err(LinkFailure::TimedOut)
        ));
    }
}
