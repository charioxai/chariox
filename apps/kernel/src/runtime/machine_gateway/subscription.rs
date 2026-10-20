//! An event subscription on another machine, held for an agent.
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
use super::{map_failure, GatewayError, MachineGateway};

const SUBSCRIBE_DEADLINE: Duration = Duration::from_secs(30);
/// Hosted relay tokens last five minutes.
const RENEW_EVERY: Duration = Duration::from_secs(3 * 60 + 30);
static SUBSCRIPTION_COUNTER: AtomicU64 = AtomicU64::new(1);

/// What the agent asked to watch on the machine.
pub(crate) struct SubscribeRequest {
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
}

impl Drop for Hold {
    fn drop(&mut self) {
        self.renewal.abort();
        self.link.unsubscribe(&self.subscription_id);
    }
}

impl MachineGateway {
    pub(crate) async fn subscribe(
        self: &Arc<Self>,
        machine: &str,
        request: SubscribeRequest,
    ) -> Result<ForwardedSubscription, GatewayError> {
        let kernel = self.directory.resolve(machine).await?;
        let link = self.link_for(machine, &kernel).await?;
        let subscription_id = format!(
            "gw-sub-{}-{}",
            crate::session::unix_epoch_ms(),
            SUBSCRIPTION_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let (ack, events) = link
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
            .map_err(|failure| map_failure(machine, failure))?;
        let ack = serde_json::from_slice(&ack).map_err(|error| GatewayError::Unreachable {
            name: machine.into(),
            detail: format!("unreadable acknowledgement: {error}"),
        })?;
        let renewal = tokio::spawn(Arc::clone(self).renew_tokens(
            machine.to_string(),
            kernel,
            Arc::clone(&link),
        ));
        Ok(ForwardedSubscription {
            ack,
            events,
            _hold: Hold {
                link,
                subscription_id,
                renewal,
            },
        })
    }

    async fn renew_tokens(
        self: Arc<Self>,
        machine: String,
        kernel: chariox_relay::protocol::RelayKernelPresence,
        link: Arc<Link>,
    ) {
        loop {
            tokio::time::sleep(RENEW_EVERY).await;
            if !link.is_alive() {
                return;
            }
            let renewed = match self.identity() {
                Ok(identity) => self.connection(&machine, &kernel, &identity).await,
                Err(error) => Err(error),
            };
            // The relay closes the connection when the old token expires; the
            // agent then hears `machine_connection_lost` and subscribes again.
            let Ok(connection) = renewed else { return };
            if !link.renew(connection.relay_token) {
                return;
            }
        }
    }
}
