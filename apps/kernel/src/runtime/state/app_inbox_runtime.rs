//! Protocol 353 installation inbox. An owner routes one external event type to
//! an App's declared incoming event. Accepted occurrences are durable before
//! the source is acknowledged, then delivered to the App's handler at least
//! once, starting a stopped worker on demand, and waiting through updates.
use super::app_wake_pump_runtime::{after_delivery, Settle, DELIVERY_TIMEOUT, PAGE, START_WAIT_MS};
use super::KernelRuntimeState;
use crate::durable_state::app_active_release::ActiveReleaseError;
use crate::durable_state::app_inbox::{AppInboxOperation, AppInboxOutcome};
use crate::local::{
    AppInboxRouteSummary, AppRequestErrorCode, LocalDaemonRequest, LocalDaemonResponse,
};
use chariox_app_runtime::app_inbox::{
    Accepted, InboxCounts, InboxError, InboxItem, InboxRoute, IncomingCatalog,
};
use serde_json::Value;

impl KernelRuntimeState {
    pub(super) async fn app_inbox_request(
        &self,
        owner: String,
        installation: String,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, AppRequestErrorCode> {
        match request {
            LocalDaemonRequest::CreateAppInboxRoute(request) => {
                let (_, catalog) = self.incoming_catalog(&owner, &installation).await?;
                if catalog.version(&request.event_name).is_none() {
                    return Err(AppRequestErrorCode::InvalidRequest);
                }
                let route = InboxRoute {
                    route_id: request.route_id,
                    owner_id: owner.clone(),
                    installation_id: installation.clone(),
                    event_name: request.event_name,
                    source_event_type: request.source_event_type,
                    source_event_version: request.source_event_version,
                    active: true,
                };
                let now_ms = crate::session::unix_epoch_ms();
                self.inbox(AppInboxOperation::CreateRoute { route, now_ms })
                    .await?;
            }
            LocalDaemonRequest::RemoveAppInboxRoute(request) => {
                self.inbox(AppInboxOperation::RemoveRoute {
                    owner: owner.clone(),
                    installation: installation.clone(),
                    route_id: request.route_id,
                })
                .await?;
            }
            LocalDaemonRequest::ListAppInboxRoutes(_) => {}
            LocalDaemonRequest::TestAppInboxRoute(request) => {
                let duplicate = self
                    .accept_app_inbox_occurrence(
                        &owner,
                        &installation,
                        &request.route_id,
                        &request.occurrence_id,
                        request.payload,
                    )
                    .await?;
                return Ok(LocalDaemonResponse::AppInboxOccurrenceAccepted {
                    installation_id: installation,
                    route_id: request.route_id,
                    occurrence_id: request.occurrence_id,
                    duplicate,
                });
            }
            _ => return Err(AppRequestErrorCode::InvalidRequest),
        }
        let AppInboxOutcome::Routes(routes) = self
            .inbox(AppInboxOperation::Routes {
                owner,
                installation: installation.clone(),
            })
            .await?
        else {
            return Err(AppRequestErrorCode::StorageUnavailable);
        };
        Ok(LocalDaemonResponse::AppInboxRoutes {
            installation_id: installation,
            routes: routes.into_iter().map(summary).collect(),
        })
    }

    /// Records one source occurrence after validating it against the active
    /// release's signed incoming schema. The source may acknowledge once this
    /// returns; delivery to the App happens later. Returns whether the same
    /// occurrence was already accepted.
    pub(crate) async fn accept_app_inbox_occurrence(
        &self,
        owner: &str,
        installation: &str,
        route_id: &str,
        occurrence_id: &str,
        payload: Value,
    ) -> Result<bool, AppRequestErrorCode> {
        let AppInboxOutcome::Routes(routes) = self
            .inbox(AppInboxOperation::Routes {
                owner: owner.to_owned(),
                installation: installation.to_owned(),
            })
            .await?
        else {
            return Err(AppRequestErrorCode::StorageUnavailable);
        };
        let (route, _) = routes
            .into_iter()
            .find(|(route, _)| route.route_id == route_id)
            .ok_or(AppRequestErrorCode::NotFound)?;
        let (generation, catalog) = self.incoming_catalog(owner, installation).await?;
        catalog
            .validate(&route.event_name, &payload)
            .map_err(|_| AppRequestErrorCode::InvalidRequest)?;
        let accepted = self
            .inbox(AppInboxOperation::Accept {
                owner: owner.to_owned(),
                installation: installation.to_owned(),
                route_id: route_id.to_owned(),
                occurrence_id: occurrence_id.to_owned(),
                payload,
                generation,
                now_ms: crate::session::unix_epoch_ms(),
            })
            .await?;
        self.schedule_app_wake_pump();
        Ok(matches!(
            accepted,
            AppInboxOutcome::Accepted(Accepted::Duplicate(_))
        ))
    }

    /// One bounded delivery pass, run with the wake pass.
    pub(super) async fn app_inbox_pass(&self, now_ms: u64) {
        let store = self.owned.durable_state_store.clone();
        let due = tokio::task::spawn_blocking(move || {
            store.app_inbox(AppInboxOperation::Due {
                now_ms,
                limit: PAGE,
            })
        })
        .await;
        let Ok(Ok(AppInboxOutcome::Due(due))) = due else {
            return;
        };
        if due.is_empty() {
            return;
        }
        let (deliver, planned) = self
            .plan_app_delivery(due, now_ms, |item: &InboxItem| {
                (item.owner_id.clone(), item.installation_id.clone())
            })
            .await;
        let mut records: Vec<_> = planned
            .into_iter()
            .map(|(item, settle)| record(item.sequence, settle, 0, now_ms))
            .collect();
        let control = self.app_control().clone();
        for item in deliver {
            // The worker left since planning: wait for the next one.
            let Some(lease) = control.active_app_lease(&item.owner_id, &item.installation_id)
            else {
                records.push(AppInboxOperation::Postponed {
                    sequence: item.sequence,
                    until_ms: now_ms.saturating_add(START_WAIT_MS),
                });
                continue;
            };
            let delivered = lease.deliver_event(&item, DELIVERY_TIMEOUT).await.is_ok();
            let update_pending = !delivered
                && self
                    .app_update_pending(&item.owner_id, &item.installation_id)
                    .await;
            let settle = after_delivery(delivered, update_pending, now_ms);
            records.push(record(
                item.sequence,
                settle,
                lease.catalog().generation(),
                now_ms,
            ));
        }
        let store = self.owned.durable_state_store.clone();
        let _ = tokio::task::spawn_blocking(move || {
            for record in records {
                let _ = store.app_inbox(record);
            }
        })
        .await;
    }

    async fn inbox(
        &self,
        operation: AppInboxOperation,
    ) -> Result<AppInboxOutcome, AppRequestErrorCode> {
        let store = self.owned.durable_state_store.clone();
        let permit = self.app_control().try_admit()?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            store.app_inbox(operation)
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
        .map_err(inbox_error)
    }

    async fn incoming_catalog(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<(u64, IncomingCatalog), AppRequestErrorCode> {
        let store = self.owned.durable_state_store.clone();
        let (owner, installation) = (owner.to_owned(), installation.to_owned());
        let permit = self.app_control().try_admit()?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            store.active_app_incoming_catalog(&owner, &installation)
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
        .map_err(|error| match error {
            ActiveReleaseError::NotActive => AppRequestErrorCode::NotFound,
            ActiveReleaseError::Untrusted | ActiveReleaseError::Invalid => {
                AppRequestErrorCode::Conflict
            }
            ActiveReleaseError::Unavailable | ActiveReleaseError::Storage => {
                AppRequestErrorCode::StorageUnavailable
            }
        })
    }
}

fn record(sequence: i64, settle: Settle, generation: u64, now_ms: u64) -> AppInboxOperation {
    match settle {
        Settle::Delivered => AppInboxOperation::Delivered {
            sequence,
            generation,
        },
        Settle::Postponed(until_ms) => AppInboxOperation::Postponed { sequence, until_ms },
        Settle::Failed => AppInboxOperation::Failed { sequence, now_ms },
    }
}

fn summary((route, counts): (InboxRoute, InboxCounts)) -> AppInboxRouteSummary {
    AppInboxRouteSummary {
        route_id: route.route_id,
        event_name: route.event_name,
        source_event_type: route.source_event_type,
        source_event_version: route.source_event_version,
        active: route.active,
        pending: counts.pending,
        delivered: counts.delivered,
        failed: counts.failed,
        expired: counts.expired,
    }
}

fn inbox_error(error: InboxError) -> AppRequestErrorCode {
    match error {
        InboxError::NotFound => AppRequestErrorCode::NotFound,
        InboxError::Conflict => AppRequestErrorCode::Conflict,
        InboxError::Limit => AppRequestErrorCode::LimitExceeded,
        InboxError::Invalid | InboxError::Schema => AppRequestErrorCode::InvalidRequest,
        InboxError::Database(_) | InboxError::Corrupt => AppRequestErrorCode::StorageUnavailable,
    }
}
