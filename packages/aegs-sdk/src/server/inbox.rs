//! Publishing from the durable inbox, its retry worker and the redelivery sweep.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use chariox_event_protocol::{
    AegsReceivedEvent, AegsReceivedEventPage, AegsReceivedEventQuery,
    AegsReceivedEventReplayRequest, AegsReceivedEventReplayResponse, AegsReceivedEventStatus,
    PublishEventRequest, PublishEventResponse,
};
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};

use super::{enforce_management_owner, error, json, read_body, AegsServer};
use crate::store::{Requeue, SelectedInterest};
use crate::{metadata_matches_filter, now_ms, InboxEntry, InboxStatus, NormalizedEvent};

const RETRY_WORKER_INTERVAL: Duration = Duration::from_secs(2);
const RETRY_BATCH: usize = 16;
const SWEEP_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// How far back the sweep looks for deliveries this AEGS never stored.
const SWEEP_WINDOW_MS: u64 = 6 * 60 * 60_000;
/// A delivery younger than this may still be on its way through the webhook handler.
const SWEEP_GRACE_MS: u64 = 2 * 60_000;
const SWEEP_REDELIVERIES_PER_RUN: usize = 50;
const REDELIVERY_REQUESTS_PER_DELIVERY: u32 = 3;

enum Replayed {
    Entry(Box<InboxEntry>),
    NotFound,
    Busy,
}

pub(super) enum InboxAttempt {
    Published(Vec<PublishEventResponse>),
    /// Publication failed; the retry worker will try again.
    Retrying,
    /// Another attempt owns the entry, or it was published already.
    NotRun,
}

impl AegsServer {
    /// Publishes `normalized` once per matching trigger interest, selected under the current
    /// authorization.
    pub(super) async fn publish_to_matching(
        &self,
        normalized: &NormalizedEvent,
        connection_id: Option<&str>,
    ) -> Result<Vec<PublishEventResponse>, String> {
        let selected = self
            .select_interests(normalized, connection_id, None)
            .await?;
        self.publish_selected(normalized, selected).await
    }

    /// `owner_id` limits the selection to that owner's connections, for a replay on their behalf.
    async fn select_interests(
        &self,
        normalized: &NormalizedEvent,
        connection_id: Option<&str>,
        owner_id: Option<&str>,
    ) -> Result<Vec<SelectedInterest>, String> {
        let mut selected = Vec::new();
        for (subscription, generation) in self
            .store
            .matching_for_delivery(
                &self.producer_id,
                &normalized.event_type,
                &normalized.connection_scope,
            )
            .await?
        {
            if connection_id.is_some_and(|value| subscription.connection_id != value)
                || !metadata_matches_filter(&normalized.metadata, &subscription.filter)
            {
                continue;
            }
            if let Some(owner_id) = owner_id {
                let owned = self
                    .store
                    .interest_owner(&subscription.connection_id, &subscription.binding_id)?
                    .is_some_and(|owner| owner == owner_id);
                if !owned {
                    continue;
                }
            }
            selected.push((subscription, generation));
        }
        Ok(selected)
    }

    /// Publishes `selected` once per interest key; an interest whose authorization changed since
    /// it was selected is skipped.
    async fn publish_selected(
        &self,
        normalized: &NormalizedEvent,
        selected: Vec<SelectedInterest>,
    ) -> Result<Vec<PublishEventResponse>, String> {
        let mut interest_keys = HashSet::new();
        let mut responses = Vec::new();
        for interest in selected {
            let key = interest.0.event_interest_key.clone();
            if !interest_keys.insert(key.clone()) {
                continue;
            }
            match self.publish_interest(normalized, interest).await? {
                Some(response) => responses.push(response),
                None => {
                    interest_keys.remove(&key);
                }
            }
        }
        Ok(responses)
    }

    /// Publishes one selected interest. `None` means its authorization changed since it was
    /// selected, so the event must not reach it. AEDS dedupes on the occurrence id, so
    /// repeating a published interest is safe.
    async fn publish_interest(
        &self,
        normalized: &NormalizedEvent,
        (subscription, generation): SelectedInterest,
    ) -> Result<Option<PublishEventResponse>, String> {
        let Ok(_delivery_guard) = self.store.delivery_guard(&subscription, generation).await else {
            return Ok(None);
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
        // AEDS has the event, so a failed health stamp must not turn it into a retry.
        if let Err(error) = self
            .store
            .mark_connection_event_accepted(&accepted_connection_id, now_ms())
        {
            eprintln!(
                "{}",
                serde_json::json!({
                    "component": "chariox-aegs",
                    "event": "connection_event_stamp_failed",
                    "connection_id": accepted_connection_id,
                    "error": error,
                })
            );
        }
        Ok(Some(response))
    }

    /// One publication attempt for every due interest of a stored event (`owner_id` limits it to
    /// that owner's). Each interest succeeds, fails or is fenced on its own row, so a failure
    /// for one never holds back or erases another. `Err` means the store failed; an AEDS failure
    /// is recorded on its row and reported as `Retrying`.
    pub(super) async fn attempt_inbox_delivery(
        &self,
        delivery_id: &str,
        owner_id: Option<&str>,
        claimed_at_ms: u64,
    ) -> Result<InboxAttempt, String> {
        let Some(entry) = self.store.inbox_event(delivery_id)? else {
            return Ok(InboxAttempt::NotRun);
        };
        if !self.store.inbox_event_selected(delivery_id)? {
            let selected = self
                .select_interests(&entry.event, entry.route_connection_id.as_deref(), None)
                .await?;
            if self
                .store
                .select_inbox_interests(delivery_id, &selected, claimed_at_ms)?
                && selected.is_empty()
            {
                return Ok(InboxAttempt::Published(Vec::new()));
            }
        }
        // Each interest is claimed when its own publication starts, so its lease never runs
        // while earlier interests are still waiting on AEDS. `claimed_at_ms` is the caller's
        // clock at the start; it advances with real time.
        let started = Instant::now();
        let elapsed_ms = || started.elapsed().as_millis() as u64;
        let mut responses = Vec::new();
        let mut retrying = false;
        let mut claimed_any = false;
        // Bindings that share an interest key publish once; a later one takes over only if the
        // earlier lost its authority.
        let mut published_keys = HashSet::new();
        for binding_id in
            self.store
                .due_inbox_bindings(delivery_id, owner_id, claimed_at_ms + elapsed_ms())?
        {
            let Some(delivery) = self.store.claim_inbox_delivery(
                delivery_id,
                &binding_id,
                claimed_at_ms + elapsed_ms(),
            )?
            else {
                continue;
            };
            claimed_any = true;
            let key = delivery.interest.0.event_interest_key.clone();
            if published_keys.contains(&(delivery.interest.0.connection_id.clone(), key.clone())) {
                self.store.finish_inbox_delivery(&delivery, now_ms())?;
                continue;
            }
            match self
                .publish_interest(&entry.event, delivery.interest.clone())
                .await
            {
                Ok(Some(response)) => {
                    if self.store.finish_inbox_delivery(&delivery, now_ms())? {
                        published_keys.insert((delivery.interest.0.connection_id.clone(), key));
                        responses.push(response);
                    }
                }
                Ok(None) => {
                    self.store.fence_inbox_delivery(&delivery, now_ms())?;
                }
                Err(message) => {
                    let Some(status) =
                        self.store
                            .fail_inbox_delivery(&delivery, &message, now_ms())?
                    else {
                        continue;
                    };
                    retrying = true;
                    eprintln!(
                        "{}",
                        serde_json::json!({
                            "component": "chariox-aegs",
                            "event": "inbox_publication_failed",
                            "delivery_id": delivery_id,
                            "interest_key": key,
                            "attempt": delivery.attempts,
                            "status": status.as_str(),
                            "error": message,
                        })
                    );
                }
            }
        }
        Ok(if !claimed_any {
            InboxAttempt::NotRun
        } else if retrying {
            InboxAttempt::Retrying
        } else {
            InboxAttempt::Published(responses)
        })
    }

    /// Runs every stored event whose next attempt is due.
    pub(super) async fn deliver_due_inbox(&self, due_at_ms: u64) -> Result<(), String> {
        let started = Instant::now();
        for delivery_id in self.store.due_inbox_deliveries(due_at_ms, RETRY_BATCH)? {
            let at_ms = due_at_ms + started.elapsed().as_millis() as u64;
            self.attempt_inbox_delivery(&delivery_id, None, at_ms)
                .await?;
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

impl AegsServer {
    /// `POST /v1/events/query`: the owner's stored webhooks, newest first.
    pub(super) async fn query_received_events(
        &self,
        request: Request<Incoming>,
    ) -> Response<Full<Bytes>> {
        let authorization = match self.authorize_management(&request) {
            Ok(authorization) => authorization,
            Err(response) => return *response,
        };
        let query: AegsReceivedEventQuery = match self.read_management_json(request).await {
            Ok(value) => value,
            Err(response) => return *response,
        };
        if let Err(response) = self.check_management_target(
            &authorization,
            &query.owner_id,
            &query.generator_id,
            query.validate(),
        ) {
            return *response;
        }
        match self
            .store
            .inbox_events_for_owner(&query.owner_id, query.limit as usize)
        {
            Ok(entries) => json(
                StatusCode::OK,
                AegsReceivedEventPage {
                    events: entries
                        .into_iter()
                        .map(|entry| self.received_event(entry))
                        .collect(),
                },
            ),
            Err(message) => error(StatusCode::INTERNAL_SERVER_ERROR, "store_failed", message),
        }
    }

    /// `POST /v1/events/replay`: publishes a stored webhook again with a fresh attempt budget.
    /// AEDS dedupes on the occurrence id, so replaying an event it already holds changes nothing.
    pub(super) async fn replay_received_event(
        &self,
        request: Request<Incoming>,
    ) -> Response<Full<Bytes>> {
        let authorization = match self.authorize_management(&request) {
            Ok(authorization) => authorization,
            Err(response) => return *response,
        };
        let replay: AegsReceivedEventReplayRequest = match self.read_management_json(request).await
        {
            Ok(value) => value,
            Err(response) => return *response,
        };
        if let Err(response) = self.check_management_target(
            &authorization,
            &replay.owner_id,
            &replay.generator_id,
            replay.validate(),
        ) {
            return *response;
        }
        let replayed: Result<Replayed, String> = async {
            let Some(stored) = self
                .store
                .inbox_event_for_owner(&replay.owner_id, &replay.delivery_id)?
            else {
                return Ok(Replayed::NotFound);
            };
            // Chosen here, not by the attempt, so no attempt can publish to another owner's
            // subscriptions in this owner's name.
            let selected = self
                .select_interests(
                    &stored.event,
                    stored.route_connection_id.as_deref(),
                    Some(&replay.owner_id),
                )
                .await?;
            match self.store.requeue_inbox_event(
                &replay.delivery_id,
                &replay.owner_id,
                &selected,
                now_ms(),
            )? {
                Requeue::Queued => {}
                Requeue::Busy => return Ok(Replayed::Busy),
                Requeue::NotFound => return Ok(Replayed::NotFound),
            }
            self.attempt_inbox_delivery(&replay.delivery_id, Some(&replay.owner_id), now_ms())
                .await?;
            Ok(
                match self
                    .store
                    .inbox_event_for_owner(&replay.owner_id, &replay.delivery_id)?
                {
                    Some(entry) => Replayed::Entry(Box::new(entry)),
                    None => Replayed::NotFound,
                },
            )
        }
        .await;
        match replayed {
            Ok(Replayed::Entry(entry)) => json(
                StatusCode::OK,
                AegsReceivedEventReplayResponse {
                    event: self.received_event(*entry),
                },
            ),
            Ok(Replayed::NotFound) => error(
                StatusCode::NOT_FOUND,
                "received_event_not_found",
                "no stored webhook with that delivery id belongs to this owner",
            ),
            Ok(Replayed::Busy) => error(
                StatusCode::CONFLICT,
                "received_event_publishing",
                "this event is being published right now; replay it again once the attempt ends",
            ),
            Err(message) => error(StatusCode::INTERNAL_SERVER_ERROR, "store_failed", message),
        }
    }

    async fn read_management_json<T: serde::de::DeserializeOwned>(
        &self,
        request: Request<Incoming>,
    ) -> Result<T, Box<Response<Full<Bytes>>>> {
        let body = read_body(request).await.map_err(Box::new)?;
        serde_json::from_slice(&body).map_err(|message| {
            Box::new(error(
                StatusCode::BAD_REQUEST,
                "invalid_json",
                message.to_string(),
            ))
        })
    }

    fn check_management_target(
        &self,
        authorization: &super::ManagementAuthorization,
        owner_id: &str,
        generator_id: &str,
        validation: Result<(), String>,
    ) -> Result<(), Box<Response<Full<Bytes>>>> {
        enforce_management_owner(authorization, owner_id)?;
        if let Err(message) = validation {
            return Err(Box::new(error(
                StatusCode::BAD_REQUEST,
                "invalid_received_event_request",
                message,
            )));
        }
        if generator_id != self.producer_id {
            return Err(Box::new(error(
                StatusCode::CONFLICT,
                "generator_mismatch",
                format!("this AEGS owns {}, not {generator_id}", self.producer_id),
            )));
        }
        Ok(())
    }
}

impl AegsServer {
    fn received_event(&self, entry: InboxEntry) -> AegsReceivedEvent {
        let subject = self.provider.event_subject(&entry.event);
        received_event(entry, subject)
    }
}

fn received_event(entry: InboxEntry, subject: Option<String>) -> AegsReceivedEvent {
    AegsReceivedEvent {
        delivery_id: entry.delivery_id,
        event_type: entry.event.event_type,
        occurrence_id: entry.event.occurrence_id,
        connection_id: entry.route_connection_id,
        connection_scope: entry.event.connection_scope,
        subject,
        received_at_ms: entry.received_at_ms,
        published_at_ms: entry.published_at_ms,
        status: match entry.status {
            InboxStatus::Pending => AegsReceivedEventStatus::Pending,
            InboxStatus::Published => AegsReceivedEventStatus::Published,
            InboxStatus::Unmatched => AegsReceivedEventStatus::Unmatched,
            InboxStatus::Failed => AegsReceivedEventStatus::Failed,
        },
        attempts: entry.attempts,
        publication_count: entry.publication_count,
        last_error: entry.last_error,
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
