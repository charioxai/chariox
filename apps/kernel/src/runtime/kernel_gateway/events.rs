//! MP-08 / MP-09 / MP-10 / MP-11: independent, durable, source-filtered parts.
use super::*;
use crate::durable_state::agent_lifecycle::outside::{Mutation, Part};
use crate::local::*;
use std::collections::HashSet;
use std::sync::Mutex as StdMutex;
use std::time::Instant;

/// A kernel that is slow to answer is still serving. Its health turns offline only when
/// requests keep timing out for this long; every other failure is reported at once.
const BUSY_GRACE: Duration = Duration::from_secs(15);
/// What one pull request to a followed kernel may take. A busy kernel needs more than a second.
const PULL_DEADLINE: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(super) struct EventParts {
    running: StdMutex<HashSet<(String, String)>>,
    health: StdMutex<HashMap<(String, String), Result<(), GatewayError>>>,
    /// Since when each followed kernel's requests have timed out without an answer between.
    timing_out_since: StdMutex<HashMap<(String, String), Instant>>,
}
mod installing;
mod parts;
#[cfg(test)]
mod tests;

impl KernelGateway {
    async fn event_targets(
        &self,
        origin: &Origin,
        targets: &[EventTarget],
    ) -> Result<Vec<EventTarget>, crate::error::DaemonError> {
        let config = self.config.snapshot();
        let owner = self
            .runtime_state
            .event_source_owner(&origin.grant_id)
            .ok_or_else(|| {
                crate::durable_state::agent_lifecycle::outside::refused("grant revoked or expired")
            })?;
        let mut normalized = Vec::new();
        for target in targets {
            let local = target.kernel == "local" || target.kernel == config.daemon_id;
            let kernel = if local {
                config.daemon_id.clone()
            } else {
                self.resolve(&target.kernel)
                    .await
                    .map_err(event_error)?
                    .kernel_id
            };
            let target = EventTarget {
                kernel,
                session_id: target.session_id.clone(),
                agent_id: target.agent_id.clone(),
            };
            if target.kernel == config.daemon_id {
                self.runtime_state
                    .validate_event_targets(std::slice::from_ref(&target), Some(&owner))?;
            }
            if !normalized.contains(&target) {
                normalized.push(target);
            }
        }
        Ok(normalized)
    }

    pub(crate) async fn event_request(
        self: &Arc<Self>,
        origin: &Origin,
        request: &LocalDaemonRequest,
        lifetime: Option<&crate::runtime::command::TerminalLifetime>,
    ) -> Result<LocalDaemonResponse, crate::error::DaemonError> {
        let id = &origin.grant_id;
        let store = self.runtime_state.outside_store();
        let owner = self.runtime_state.event_source_owner(id);
        let response = match request {
            LocalDaemonRequest::SubscribeEvents(request) => {
                let follows = request.targets.len() + request.sources.len();
                if follows == 0 || follows > 1024 {
                    return Err(crate::durable_state::agent_lifecycle::outside::refused(
                        "follow 1–1024 event targets and sources",
                    ));
                }
                let targets = self.event_targets(origin, &request.targets).await?;
                if !request.sources.is_empty() && owner.is_none() {
                    return Err(crate::durable_state::agent_lifecycle::outside::refused(
                        "grant revoked or expired",
                    ));
                }
                let previous = store
                    .outside_part(id)?
                    .map_or_else(Vec::new, |part| part.targets);
                let following: Vec<_> = self
                    .runtime_state
                    .followed_notification_sources(id)?
                    .into_iter()
                    .map(|follow| follow.source_id)
                    .collect();
                let mut added = Added {
                    targets: targets
                        .iter()
                        .filter(|target| !previous.contains(target))
                        .cloned()
                        .collect(),
                    sources: Vec::new(),
                };
                for source in &request.sources {
                    if !following.contains(source) && !added.sources.contains(source) {
                        added.sources.push(source.clone());
                    }
                }
                // The inbox admits the request before anything else commits: an ended one
                // refuses it here, so no follow is left behind that could never deliver.
                self.runtime_state
                    .subscribe_outside_part(id, true, targets.clone())?;
                if let Some(owner) = owner.as_deref().filter(|_| !request.sources.is_empty()) {
                    if let Err(error) =
                        self.runtime_state
                            .follow_notification_sources(id, owner, &request.sources)
                    {
                        self.withdraw(origin, &added)?;
                        return Err(error);
                    }
                }
                let part = store.outside_part(id)?.expect("subscribed part exists");
                let local = self.config.snapshot().daemon_id;
                let kernels: HashSet<_> = part
                    .targets
                    .iter()
                    .filter(|target| target.kernel != local)
                    .map(|target| target.kernel.clone())
                    .collect();
                let installations = futures_util::future::join_all(kernels.iter().map(|kernel| {
                    let targets = part
                        .targets
                        .iter()
                        .filter(|target| &target.kernel == kernel)
                        .cloned()
                        .collect();
                    async move {
                        (
                            kernel,
                            self.forward(
                                kernel,
                                origin,
                                &event_command_id(),
                                &LocalDaemonRequest::SubscribeEvents(SubscribeEventsRequest {
                                    targets,
                                    sources: Vec::new(),
                                }),
                                REQUEST_DEADLINE,
                            )
                            .await,
                        )
                    }
                }))
                .await;
                self.settle_installations(origin, &added, installations)?;
                self.start_event_parts(origin);
                self.runtime_state.outside_subscription_response(id)?
            }
            LocalDaemonRequest::UnsubscribeEvents(request) => {
                let named = !request.targets.is_empty() || !request.sources.is_empty();
                if request.all == named {
                    return Err(crate::durable_state::agent_lifecycle::outside::refused(
                        "unsubscribe needs targets, sources or --all",
                    ));
                }
                // Removing an ended target must not require that target to remain live.
                let targets = if request.all {
                    vec![]
                } else {
                    self.event_targets_for_removal(id, &request.targets).await?
                };
                // The inbox ends before its follows do: a follow made meanwhile is refused by
                // the ended inbox, and one made earlier is removed below.
                store.outside_mutate(Mutation::Unsubscribe {
                    id: id.clone(),
                    targets,
                    all: request.all,
                    reason: "cancelled".into(),
                })?;
                // A holder whose grant is gone follows nothing: its sources retired with it.
                if let Some(owner) = owner.as_deref() {
                    if request.all || !request.sources.is_empty() {
                        self.runtime_state.unfollow_notification_sources(
                            id,
                            owner,
                            (!request.all).then_some(request.sources.as_slice()),
                        )?;
                    }
                }
                self.start_event_parts(origin);
                self.runtime_state.outside_subscription_response(id)?
            }
            LocalDaemonRequest::WaitEvents(request) => {
                self.runtime_state
                    .outside_wait(id, request, Some(id), lifetime)
                    .await?
            }
            _ => self
                .runtime_state
                .outside_part_request(id, true, owner.as_deref(), request)?,
        };
        if let LocalDaemonResponse::EventSubscription { mut subscription } = response {
            let health = self
                .event_parts
                .health
                .lock()
                .expect("event health poisoned");
            let local = self.config.snapshot().daemon_id;
            let kernels: HashSet<_> = subscription
                .targets
                .iter()
                .map(|t| t.kernel.clone())
                .collect();
            subscription.kernels=kernels.into_iter().map(|kernel| {
                let status=health.get(&(id.clone(),kernel.clone()));
                let online=kernel==local || matches!(status,Some(Ok(())));
                let error=if online { None } else { Some(match status {
                    Some(Err(error))=>EventKernelIssue { code:error.code().into(),message:error.message() },
                    _=>EventKernelIssue { code:"events_connecting".into(),message:"event part has not connected yet; the inbox will resume when the kernel is reachable".into() },
                }) };
                EventKernelHealth { kernel,online,error }
            }).collect();
            subscription.kernels.sort_by(|a, b| a.kernel.cmp(&b.kernel));
            return Ok(LocalDaemonResponse::EventSubscription { subscription });
        }
        Ok(response)
    }
    async fn event_targets_for_removal(
        &self,
        id: &str,
        targets: &[EventTarget],
    ) -> Result<Vec<EventTarget>, crate::error::DaemonError> {
        let existing = self
            .runtime_state
            .outside_store()
            .outside_part(id)?
            .ok_or_else(|| {
                crate::durable_state::agent_lifecycle::outside::refused("no event subscription")
            })?;
        let local = self.config.snapshot().daemon_id;
        let mut normalized = Vec::new();
        for target in targets {
            let kernel = if target.kernel == "local" {
                local.clone()
            } else if existing.targets.iter().any(|t| t.kernel == target.kernel) {
                target.kernel.clone()
            } else {
                self.resolve(&target.kernel)
                    .await
                    .map_err(event_error)?
                    .kernel_id
            };
            normalized.push(EventTarget {
                kernel,
                session_id: target.session_id.clone(),
                agent_id: target.agent_id.clone(),
            });
        }
        Ok(normalized)
    }
}
fn event_command_id() -> String {
    format!("events-{}", REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed))
}
/// What one subscribe request added to the grant: the rest of the follow existed before it.
pub(crate) struct Added {
    pub(crate) targets: Vec<EventTarget>,
    pub(crate) sources: Vec<String>,
}

impl KernelGateway {
    /// Takes back what a refused subscribe request committed, so a refusal leaves no follow
    /// behind that still counts against the grant's limits.
    fn withdraw(
        self: &Arc<Self>,
        origin: &Origin,
        added: &Added,
    ) -> Result<(), crate::error::DaemonError> {
        let id = &origin.grant_id;
        if !added.sources.is_empty() {
            if let Some(owner) = self.runtime_state.event_source_owner(id) {
                self.runtime_state.unfollow_notification_sources(
                    id,
                    &owner,
                    Some(added.sources.as_slice()),
                )?;
            }
        }
        if !added.targets.is_empty() {
            // Keep cleanup durable if another target already installed its part.
            self.runtime_state
                .outside_store()
                .outside_mutate(Mutation::Unsubscribe {
                    id: id.clone(),
                    targets: added.targets.clone(),
                    all: false,
                    reason: "target_refused".into(),
                })?;
        }
        self.start_event_parts(origin);
        Ok(())
    }

    /// Records each followed kernel's answer. A kernel that refuses the request for good
    /// refuses the whole request: nothing it added stays followed.
    pub(crate) fn settle_installations(
        self: &Arc<Self>,
        origin: &Origin,
        added: &Added,
        installations: Vec<(&String, Result<Value, GatewayError>)>,
    ) -> Result<(), crate::error::DaemonError> {
        for (kernel, result) in installations {
            // Only the subscribing caller receives this refusal directly.
            if let Err(
                error @ GatewayError::Remote {
                    retryable: false, ..
                },
            ) = result
            {
                self.withdraw(origin, added)?;
                return Err(event_error(error));
            }
            self.record_event_health(
                origin,
                kernel,
                result.as_ref().map(|_| ()).map_err(Clone::clone),
            );
        }
        Ok(())
    }
}

fn event_error(error: GatewayError) -> crate::error::DaemonError {
    crate::error::DaemonError::MachineRequest {
        code: error.code().into(),
        message: error.message(),
    }
}
fn part_error(kernel: &str, message: &str) -> GatewayError {
    GatewayError::Remote {
        name: kernel.into(),
        code: "events_part_failed".into(),
        message: message.into(),
        retryable: true,
    }
}
