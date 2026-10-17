//! Publishing from the durable inbox, its retry worker and the redelivery sweep.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use chariox_event_protocol::{PublishEventRequest, PublishEventResponse};

use super::AegsServer;
use crate::{metadata_matches_filter, now_ms, NormalizedEvent};

const RETRY_WORKER_INTERVAL: Duration = Duration::from_secs(2);
const RETRY_BATCH: usize = 16;
const SWEEP_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// How far back the sweep looks for deliveries this AEGS never stored.
const SWEEP_WINDOW_MS: u64 = 6 * 60 * 60_000;
/// A delivery younger than this may still be on its way through the webhook handler.
const SWEEP_GRACE_MS: u64 = 2 * 60_000;
const SWEEP_REDELIVERIES_PER_RUN: usize = 50;
const REDELIVERY_REQUESTS_PER_DELIVERY: u32 = 3;

pub(super) enum InboxAttempt {
    Published(Vec<PublishEventResponse>),
    /// Publication failed; the retry worker will try again.
    Retrying,
    /// Another attempt owns the entry, or it was published already.
    NotRun,
}

impl AegsServer {
    /// Publishes `normalized` once per matching trigger interest. AEDS dedupes on the occurrence
    /// id, so repeating a partly published event is safe.
    pub(super) async fn publish_to_matching(
        &self,
        normalized: &NormalizedEvent,
        connection_id: Option<&str>,
    ) -> Result<Vec<PublishEventResponse>, String> {
        let subscriptions = self
            .store
            .matching_for_delivery(
                &self.producer_id,
                &normalized.event_type,
                &normalized.connection_scope,
            )
            .await?;
        let mut interest_keys = HashSet::new();
        let mut responses = Vec::new();
        for (subscription, generation) in subscriptions {
            if connection_id.is_some_and(|value| subscription.connection_id != value) {
                continue;
            }
            if !metadata_matches_filter(&normalized.metadata, &subscription.filter) {
                continue;
            }
            if !interest_keys.insert(subscription.event_interest_key.clone()) {
                continue;
            }
            let _delivery_guard = match self.store.delivery_guard(&subscription, generation).await {
                Ok(guard) => guard,
                Err(_) => {
                    interest_keys.remove(&subscription.event_interest_key);
                    continue;
                }
            };
            let accepted_connection_id = subscription.connection_id.clone();
            let response = self
                .publisher
                .publish(PublishEventRequest {
                    producer_id: self.producer_id.clone(),
                    event_interest_key: subscription.event_interest_key,
                    occurrence_id: normalized.occurrence_id.clone(),
                    event_type: normalized.event_type.clone(),
                    event_type_version: subscription.event_type_version,
                    occurred_at: normalized.occurred_at.clone(),
                    prompt: normalized.prompt.clone(),
                    artifacts: Vec::new(),
                    metadata: normalized.metadata.clone(),
                    reply_context: normalized.reply_context.clone(),
                    ttl_seconds: chariox_event_protocol::DEFAULT_EVENT_DELIVERY_TTL_SECONDS,
                })
                .await?;
            self.store
                .mark_connection_event_accepted(&accepted_connection_id, now_ms())?;
            responses.push(response);
        }
        Ok(responses)
    }

    /// One publication attempt for a stored event. `Err` means the store failed; an AEDS
    /// failure is recorded on the entry and reported as `Retrying`.
    pub(super) async fn attempt_inbox_delivery(
        &self,
        delivery_id: &str,
        claimed_at_ms: u64,
    ) -> Result<InboxAttempt, String> {
        let Some(entry) = self.store.claim_inbox_attempt(delivery_id, claimed_at_ms)? else {
            return Ok(InboxAttempt::NotRun);
        };
        match self
            .publish_to_matching(&entry.event, entry.route_connection_id.as_deref())
            .await
        {
            Ok(responses) => {
                self.store
                    .finish_inbox_publication(delivery_id, responses.len(), now_ms())?;
                Ok(InboxAttempt::Published(responses))
            }
            Err(message) => {
                let status = self
                    .store
                    .fail_inbox_publication(delivery_id, &message, now_ms())?;
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "component": "chariox-aegs",
                        "event": "inbox_publication_failed",
                        "delivery_id": delivery_id,
                        "attempt": entry.attempts,
                        "status": status,
                        "error": message,
                    })
                );
                Ok(InboxAttempt::Retrying)
            }
        }
    }

    /// Runs every stored event whose next attempt is due.
    pub(super) async fn deliver_due_inbox(&self, due_at_ms: u64) -> Result<(), String> {
        for delivery_id in self.store.due_inbox_deliveries(due_at_ms, RETRY_BATCH)? {
            self.attempt_inbox_delivery(&delivery_id, due_at_ms).await?;
        }
        Ok(())
    }

    /// Asks the provider to resend recent deliveries that were never stored here. The resent
    /// webhook takes the normal path, so signature checks and deduplication still apply.
    /// `requested` counts redelivery requests per delivery for the life of the sweeper.
    pub(super) async fn sweep_missed_deliveries(
        &self,
        swept_at_ms: u64,
        requested: &mut HashMap<String, u32>,
    ) -> Result<usize, String> {
        let provider = Arc::clone(&self.provider);
        let since_ms = swept_at_ms.saturating_sub(SWEEP_WINDOW_MS);
        let deliveries = tokio::task::spawn_blocking(move || provider.recent_deliveries(since_ms))
            .await
            .map_err(|error| error.to_string())??;
        requested.retain(|id, _| {
            deliveries
                .iter()
                .any(|delivery| &delivery.delivery_id == id)
        });
        let mut redelivered = 0;
        for delivery in deliveries {
            if redelivered == SWEEP_REDELIVERIES_PER_RUN {
                break;
            }
            let count = requested.entry(delivery.delivery_id.clone()).or_default();
            if delivery.delivered_at_ms < since_ms
                || delivery.delivered_at_ms + SWEEP_GRACE_MS > swept_at_ms
                || *count >= REDELIVERY_REQUESTS_PER_DELIVERY
                || self.store.has_inbox_event(&delivery.delivery_id)?
            {
                continue;
            }
            *count += 1;
            let provider = Arc::clone(&self.provider);
            tokio::task::spawn_blocking(move || provider.redeliver(&delivery))
                .await
                .map_err(|error| error.to_string())??;
            redelivered += 1;
        }
        Ok(redelivered)
    }
}

pub(super) fn spawn_inbox_delivery(server: AegsServer) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(RETRY_WORKER_INTERVAL);
        loop {
            interval.tick().await;
            let now = now_ms();
            let outcome = match server.deliver_due_inbox(now).await {
                Ok(()) => server.store.prune_inbox(now).map(|_| ()),
                Err(message) => Err(message),
            };
            if let Err(message) = outcome {
                log_failure("inbox_worker_failed", &message);
            }
        }
    });
}

pub(super) fn spawn_delivery_sweep(server: AegsServer) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(SWEEP_INTERVAL);
        let mut requested = HashMap::new();
        loop {
            interval.tick().await;
            if let Err(message) = server
                .sweep_missed_deliveries(now_ms(), &mut requested)
                .await
            {
                log_failure("delivery_sweep_failed", &message);
            }
        }
    });
}

fn log_failure(event: &str, error: &str) {
    eprintln!(
        "{}",
        serde_json::json!({"component": "chariox-aegs", "event": event, "error": error})
    );
}
