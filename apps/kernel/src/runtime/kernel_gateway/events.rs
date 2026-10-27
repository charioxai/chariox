//! MP-08 / MP-09 / MP-10 / MP-11: independent, durable, source-filtered parts.
use super::*;
use crate::durable_state::agent_lifecycle::outside::{Mutation, Part};
use crate::local::*;
use std::collections::HashSet;
use std::sync::Mutex as StdMutex;

#[derive(Default)]
pub(super) struct EventParts {
    running: StdMutex<HashSet<(String, String)>>,
    health: StdMutex<HashMap<(String, String), Result<(), GatewayError>>>,
}
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
        if targets.is_empty() || targets.len() > 1024 {
            return Err(crate::durable_state::agent_lifecycle::outside::refused(
                "follow 1–1024 event targets",
            ));
        }
        let owner = self
            .runtime_state
            .access_grant_summary(&origin.grant_id)
            .ok_or_else(|| {
                crate::durable_state::agent_lifecycle::outside::refused("grant revoked or expired")
            })?
            .owner_user_id;
        let mut normalized = Vec::new();
        for target in targets {
            let local = target.kernel == "local"
                || target.kernel == config.daemon_id
                || config.daemon_alias.as_deref() == Some(&target.kernel);
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
        let owner = self
            .runtime_state
            .access_grant_summary(id)
            .map(|grant| grant.owner_user_id);
        let response = match request {
            LocalDaemonRequest::SubscribeEvents(request) => {
                let targets = self.event_targets(origin, &request.targets).await?;
                let previous = store
                    .outside_part(id)?
                    .map_or_else(Vec::new, |part| part.targets);
                let added: Vec<_> = targets
                    .iter()
                    .filter(|target| !previous.contains(target))
                    .cloned()
                    .collect();
                self.runtime_state
                    .subscribe_outside_part(id, true, targets.clone())?;
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
                                }),
                                REQUEST_DEADLINE,
                            )
                            .await,
                        )
                    }
                }))
                .await;
                for (kernel, result) in installations {
                    // Only the subscribing caller receives this refusal directly.
                    if let Err(
                        error @ GatewayError::Remote {
                            retryable: false, ..
                        },
                    ) = result
                    {
                        // Keep cleanup durable if another target already installed its part.
                        store.outside_mutate(Mutation::Unsubscribe {
                            id: id.clone(),
                            targets: added.clone(),
                            all: false,
                            reason: "target_refused".into(),
                        })?;
                        self.start_event_parts(origin);
                        return Err(event_error(error));
                    }
                    self.record_event_health(
                        origin,
                        kernel,
                        result.as_ref().map(|_| ()).map_err(Clone::clone),
                    );
                }
                self.start_event_parts(origin);
                self.runtime_state.outside_subscription_response(id)?
            }
            LocalDaemonRequest::UnsubscribeEvents(request) => {
                if request.all != request.targets.is_empty() {
                    return Err(crate::durable_state::agent_lifecycle::outside::refused(
                        "unsubscribe needs targets or --all",
                    ));
                }
                // Removing an ended target must not require that target to remain live.
                let targets = if request.all {
                    vec![]
                } else {
                    self.event_targets_for_removal(id, &request.targets).await?
                };
                store.outside_mutate(Mutation::Unsubscribe {
                    id: id.clone(),
                    targets,
                    all: request.all,
                    reason: "cancelled".into(),
                })?;
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
