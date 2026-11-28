//! An event subscription on another kernel, held for an agent.
//!
//! The target streams the events to this kernel exactly as it would to a remote
//! terminal (its own heartbeats keep the attachment alive); this kernel relays
//! them to the agent's connection. The relay token of the connection is renewed
//! in place, as a remote terminal does, so a subscription outlives the token.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::link::{Link, SubscribeParams};
use super::{map_failure, GatewayError, KernelGateway};

const SUBSCRIBE_DEADLINE: Duration = Duration::from_secs(30);
/// Hosted relay tokens last five minutes.
pub(super) const RENEW_EVERY: Duration = Duration::from_secs(3 * 60 + 30);
const RENEW_RETRIES: u32 = 2;
pub(super) const RENEW_RETRY_AFTER: Duration = Duration::from_secs(15);
static SUBSCRIPTION_COUNTER: AtomicU64 = AtomicU64::new(1);

/// What the agent asked to watch on the kernel.
pub(crate) struct SubscribeRequest {
    pub(crate) grant_id: String,
    pub(crate) session_id: String,
    pub(crate) attachment_id: String,
    pub(crate) scope: Option<String>,
    pub(crate) resume_from_event_id: Option<u64>,
}

pub(crate) struct ForwardedSubscription {
    /// The target's acknowledgement, passed to the agent unchanged.
    pub(crate) ack: Value,
    /// (event id on the target's stream, event body); closed when the link ends.
    pub(crate) events: mpsc::UnboundedReceiver<(u64, Vec<u8>)>,
    _hold: Hold,
}

/// Stops the subscription on the target and the token renewal when dropped.
struct Hold {
    link: Arc<Link>,
    subscription_id: String,
    renewal: JoinHandle<()>,
    forwarding: JoinHandle<()>,
}

impl Drop for Hold {
    fn drop(&mut self) {
        self.renewal.abort();
        self.forwarding.abort();
        self.link.unsubscribe(&self.subscription_id);
    }
}

impl KernelGateway {
    pub(crate) async fn subscribe(
        self: &Arc<Self>,
        target: &str,
        request: SubscribeRequest,
    ) -> Result<ForwardedSubscription, GatewayError> {
        let origin = self
            .runtime_state
            .gateway_origin(&request.grant_id)
            .ok_or_else(|| GatewayError::AccessEnded {
                name: target.into(),
            })?;
        self.ensure_access_live(&origin, target)?;
        let kernel = self.resolve(target).await?;
        self.ensure_access_live(&origin, target)?;
        self.register_for(&request.grant_id, target).await?;
        self.ensure_access_live(&origin, target)?;
        // Its own connection: the relay token it renews is the one it was opened with,
        // not one shared with requests that may be minutes older.
        let link = self.open_link(target, &kernel).await?;
        self.ensure_access_live(&origin, target)?;
        let subscription_id = format!(
            "gw-sub-{}-{}",
            crate::session::unix_epoch_ms(),
            SUBSCRIPTION_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let (ack, mut remote_events) = link
            .subscribe(
                subscription_id.clone(),
                SubscribeParams {
                    session_id: request.session_id,
                    attachment_id: request.attachment_id,
                    scope: request.scope,
                    resume_from_event_id: request.resume_from_event_id,
                },
                SUBSCRIBE_DEADLINE,
            )
            .await
            .map_err(|failure| map_failure(target, failure))?;
        if let Err(error) = self.ensure_access_live(&origin, target) {
            link.unsubscribe(&subscription_id);
            return Err(error);
        }
        let ack = serde_json::from_slice(&ack).map_err(|error| GatewayError::Unreachable {
            name: target.into(),
            detail: format!("unreadable acknowledgement: {error}"),
        })?;
        let renewal = tokio::spawn(Arc::clone(self).renew_tokens(
            target.to_string(),
            kernel,
            Arc::clone(&link),
        ));
        let (sender, events) = mpsc::unbounded_channel();
        let gateway = Arc::clone(self);
        let stream_target = target.to_owned();
        let stream_link = Arc::clone(&link);
        let stream_id = subscription_id.clone();
        let forwarding = tokio::spawn(async move {
            while let Some(event) = remote_events.recv().await {
                if gateway.ensure_access_live(&origin, &stream_target).is_err()
                    || sender.send(event).is_err()
                {
                    break;
                }
            }
            stream_link.unsubscribe(&stream_id);
        });
        Ok(ForwardedSubscription {
            ack,
            events,
            _hold: Hold {
                link,
                subscription_id,
                renewal,
                forwarding,
            },
        })
    }

    /// Gets a fresh relay token and puts it on the open connection.
    pub(super) async fn renew_once(
        &self,
        target: &str,
        kernel: &chariox_relay::protocol::RelayKernelPresence,
        link: &Link,
    ) -> bool {
        let renewed = match self.identity() {
            Ok(identity) => self.connection(target, kernel, &identity).await,
            Err(error) => Err(error),
        };
        renewed.is_ok_and(|connection| link.renew(connection.relay_token))
    }

    async fn renew_tokens(
        self: Arc<Self>,
        target: String,
        kernel: chariox_relay::protocol::RelayKernelPresence,
        link: Arc<Link>,
    ) {
        loop {
            tokio::time::sleep_until(link.token_renewal_due(RENEW_EVERY).into()).await;
            let mut attempts = 0;
            loop {
                if !link.is_alive() {
                    return;
                }
                if self.renew_once(&target, &kernel, &link).await {
                    break;
                }
                // Retried within the token's remaining minute; if it still fails the
                // relay closes the connection at expiry and the agent hears
                // `kernel_connection_lost`, then subscribes again.
                if attempts >= RENEW_RETRIES {
                    return;
                }
                attempts += 1;
                tokio::time::sleep(RENEW_RETRY_AFTER).await;
            }
        }
    }
}
