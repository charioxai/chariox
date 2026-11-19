//! Connection-owned payload tasks: application waits never hold relay reads or heartbeats.
use super::browser_input_order::BrowserInputOrder;
use super::incoming_envelopes::{handle_parsed_incoming_envelope, parse_relay_payload};
use super::*;

#[derive(Default)]
pub(super) struct IncomingEnvelopeTasks {
    tasks: tokio::task::JoinSet<Result<(), DaemonError>>,
    order: BrowserInputOrder,
}

impl IncomingEnvelopeTasks {
    pub(super) fn spawn(
        &mut self,
        context: IncomingEnvelopeContext<'_>,
        active_dynamic_relay: Option<(String, String)>,
        payload: String,
    ) -> Result<(), DaemonError> {
        let envelope = parse_relay_payload(&payload)?;
        // Request preparation reserves browser input order before launching its worker.
        // Subscriptions keep socket order across attachment waits, and each display
        // stream delivers its chunks before its close.
        let key = match &envelope {
            RelayEnvelope::DaemonRequest { .. }
            | RelayEnvelope::DaemonIncomingPeerRequest { .. } => Some("requests".to_string()),
            RelayEnvelope::DaemonSubscribe {
                relay_subscription_id,
                ..
            }
            | RelayEnvelope::DaemonUnsubscribe {
                relay_subscription_id,
                ..
            } => Some(format!("subscription/{relay_subscription_id}")),
            RelayEnvelope::DaemonDisplayTunnelClientChunk { chunk } => {
                Some(format!("display/{}", chunk.stream_id))
            }
            RelayEnvelope::DaemonDisplayTunnelClientClose { stream_id, .. } => {
                Some(format!("display/{stream_id}"))
            }
            _ => None,
        };
        let turn = key.map(|key| self.order.reserve_payload(key));
        let router = Arc::clone(context.router);
        let command_sequence = Arc::clone(context.command_sequence);
        let state = Arc::clone(context.state);
        let outgoing_tx = context.outgoing_tx.clone();
        let subscription_tasks = Arc::clone(context.subscription_tasks);
        let event_runtime = Arc::clone(context.event_runtime);
        let command_result_cache = Arc::clone(context.command_result_cache);
        let reconnect_gate = Arc::clone(context.reconnect_gate);
        let local_browser = Arc::clone(context.local_browser);
        self.tasks.spawn(async move {
            let mut turn = turn;
            if let Some(turn) = turn.as_mut() {
                turn.ready().await;
            }
            handle_parsed_incoming_envelope(
                IncomingEnvelopeContext {
                    router: &router,
                    command_sequence: &command_sequence,
                    state: &state,
                    outgoing_tx: &outgoing_tx,
                    subscription_tasks: &subscription_tasks,
                    event_runtime: &event_runtime,
                    command_result_cache: &command_result_cache,
                    reconnect_gate: &reconnect_gate,
                    local_browser: &local_browser,
                },
                active_dynamic_relay
                    .as_ref()
                    .map(|(url, token)| (url.as_str(), token.as_str())),
                envelope,
            )
            .await
        });
        Ok(())
    }

    pub(super) fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    pub(super) async fn next(&mut self) -> Option<Result<(), DaemonError>> {
        self.tasks.join_next().await.map(|result| {
            result.unwrap_or_else(|error| {
                Err(DaemonError::LocalTransport {
                    operation: "relay payload task",
                    message: error.to_string(),
                })
            })
        })
    }

    pub(super) async fn shutdown(&mut self) {
        self.tasks.shutdown().await;
    }
}
