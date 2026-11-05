//! App occurrences retain App catalog, signer, schema, and receipt admission.
//! Only their final target differs from workflow automation delivery.
use super::agent_trigger_notification::ResolvedSource;
use super::*;
use crate::durable_state::app_automations::{AppAutomationMutation, WorkflowAutomationTarget};
use crate::runtime::app_operation_budget::AppOperationBudget;
use crate::session::{AgentTrigger, AgentTriggerNotificationSource, AgentTriggerSource};
use chariox_app_runtime::app_outbox::{AutomationStatus, AutomationTargetKind};

fn app_error(error: impl std::fmt::Display) -> DaemonError {
    crate::durable_state::workflow_notifications::error(error.to_string())
}
fn budget() -> AppOperationBudget {
    AppOperationBudget::from_supervisor(|| false)
}

impl KernelRuntimeState {
    /// A Room's installed Apps are its agents' existing App bindings. Listing
    /// events grants no App capability or connection access.
    pub(super) fn room_notification_apps(&self, room: &str) -> BTreeSet<String> {
        self.session_agents(room)
            .iter()
            .flat_map(|agent| {
                agent
                    .extension_grants()
                    .iter()
                    .filter(|grant| grant.kind == crate::extension::ExtensionKind::App)
                    .map(|grant| grant.name.clone())
            })
            .collect()
    }

    pub(super) async fn list_room_notification_apps(
        &self,
        request: &crate::local::ListAppInstallationsRequest,
        caller: &str,
    ) -> Result<crate::local::LocalDaemonResponse, crate::local::AppRequestErrorCode> {
        use crate::local::AppRequestErrorCode as Code;
        let room_id = request.session_id.as_deref().ok_or(Code::InvalidRequest)?;
        let room = self
            .owned
            .session_store
            .get_session(room_id)
            .map_err(|_| Code::Unauthorized)?;
        if !room.has_member(caller) || room.status() == crate::session::SessionStatus::Ended {
            return Err(Code::Unauthorized);
        }
        if let Some((agent_id, _)) = &self.room_provider_origin {
            if !self
                .owned
                .agent_store
                .get_agent(agent_id)
                .is_ok_and(|agent| agent.session_id() == room_id)
            {
                return Err(Code::Unauthorized);
            }
        }
        let limit = request.limit.unwrap_or(50) as usize;
        if !(1..=100).contains(&limit)
            || request.after.as_deref().is_some_and(|id| {
                id.is_empty() || id.len() > 128 || id.chars().any(char::is_control)
            })
        {
            return Err(Code::InvalidRequest);
        }
        let ids = self
            .room_notification_apps(room_id)
            .into_iter()
            .filter(|id| request.after.as_ref().is_none_or(|after| id > after))
            .take(limit + 1)
            .collect::<Vec<_>>();
        let next_cursor = (ids.len() > limit).then(|| ids[limit - 1].clone());
        let store = self.owned.durable_state_store.clone();
        let owner = room.owner_user_id().to_owned();
        let permit = self.app_control().try_admit()?;
        let installations = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            ids.into_iter()
                .take(limit)
                .map(|id| {
                    let installation = store
                        .get_app_installation(&owner, &id)
                        .map_err(|_| Code::NotFound)?;
                    Ok(
                        crate::runtime::app_control::projection::installation_with_events(
                            &store,
                            &owner,
                            installation,
                        ),
                    )
                })
                .collect::<Result<Vec<_>, Code>>()
        })
        .await
        .map_err(|_| Code::StorageUnavailable)??;
        self.authorize_current_external_response()
            .map_err(|_| Code::Unauthorized)?;
        if !self
            .owned
            .session_store
            .get_session(room_id)
            .is_ok_and(|room| {
                room.has_member(caller) && room.status() != crate::session::SessionStatus::Ended
            })
        {
            return Err(Code::Unauthorized);
        }
        Ok(crate::local::LocalDaemonResponse::AppInstallationsListed {
            installations,
            next_cursor,
        })
    }

    pub(super) async fn resolve_app_notification_source(
        &self,
        owner: &str,
        caller: &str,
        room: &str,
        installation: &str,
        event: &str,
    ) -> Result<ResolvedSource, DaemonError> {
        if (caller != owner || self.room_provider_origin.is_some())
            && !self.room_notification_apps(room).contains(installation)
        {
            return Err(app_error("App is not installed in this Room"));
        }
        let catalog = self
            .app_catalog(owner, installation)
            .await
            .map_err(|code| app_error(format!("App event unavailable: {code:?}")))?;
        let label = catalog
            .outgoing_events()
            .find(|(name, _)| *name == event)
            .map(|(name, label)| label.unwrap_or(name).to_owned())
            .ok_or_else(|| app_error("App does not declare this outgoing event"))?;
        let installed = self
            .owned
            .durable_state_store
            .get_app_installation(owner, installation)
            .map_err(app_error)?;
        Ok(ResolvedSource {
            kernel_id: self.owned.config_projection.snapshot().daemon_id,
            display_name: format!("{} · {label}", installed.app_id),
        })
    }

    pub(super) async fn subscribe_app_agent_notification(
        &self,
        owner: &str,
        room: &str,
        agent: &str,
        trigger: &AgentTrigger,
    ) -> Result<(), DaemonError> {
        let AgentTriggerSource::Notification {
            source:
                AgentTriggerNotificationSource::App {
                    installation_id,
                    event_name,
                },
            subscription_id,
            ..
        } = trigger.source()
        else {
            return Err(app_error("not an App notification trigger"));
        };
        let catalog = self
            .app_catalog(owner, installation_id)
            .await
            .map_err(|code| app_error(format!("App event unavailable: {code:?}")))?;
        let owned = self.owned.clone();
        let (owner, room, agent, trigger_id, automation_id, event_name) = (
            owner.to_owned(),
            room.to_owned(),
            agent.to_owned(),
            trigger.id().to_owned(),
            subscription_id.clone(),
            event_name.clone(),
        );
        let permit = self
            .app_control()
            .try_admit()
            .map_err(|code| app_error(format!("App admission unavailable: {code:?}")))?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let sessions = owned.session_store.read();
            let target = WorkflowAutomationTarget::resolve_agent(
                &sessions,
                &owner,
                &room,
                &agent,
                &trigger_id,
            )
            .map_err(app_error)?;
            owned
                .durable_state_store
                .mutate_app_automation(
                    &owner,
                    catalog,
                    AppAutomationMutation::Configure {
                        automation_id,
                        expected_revision: 0,
                        event_name,
                        target,
                        scheduled: false,
                    },
                    budget(),
                )
                .map_err(app_error)?;
            drop(sessions);
            Ok(())
        })
        .await
        .map_err(app_error)?
    }

    pub(super) async fn detach_app_agent_notification(&self, trigger: &AgentTrigger) {
        let AgentTriggerSource::Notification {
            source:
                AgentTriggerNotificationSource::App {
                    installation_id, ..
                },
            subscription_id,
            ..
        } = trigger.source()
        else {
            return;
        };
        let Some(owner) = trigger.created_by_user_id() else {
            return;
        };
        // The installation belongs to the Room owner, including guest-created triggers.
        let room_owner = self
            .owned
            .agent_store
            .get_agent(trigger.agent_id())
            .ok()
            .and_then(|agent| {
                self.owned
                    .session_store
                    .get_session(agent.session_id())
                    .ok()
            })
            .map(|room| room.owner_user_id().to_owned());
        let owner = room_owner.as_deref().unwrap_or(owner);
        if let Err(error) = self
            .disable_app_notification(owner, installation_id, subscription_id)
            .await
        {
            tracing::warn!(%error, "App notification detach will retry in the pump");
        }
    }

    async fn disable_app_notification(
        &self,
        owner: &str,
        installation: &str,
        automation_id: &str,
    ) -> Result<(), DaemonError> {
        let catalog = self
            .app_catalog(owner, installation)
            .await
            .map_err(|code| app_error(format!("App unavailable: {code:?}")))?;
        let configurations = self
            .owned
            .list_app_automations(owner, catalog.clone(), budget())
            .map_err(app_error)?;
        if let Some(config) = configurations
            .into_iter()
            .find(|c| c.automation_id == automation_id && c.status != AutomationStatus::Disabled)
        {
            self.owned
                .deactivate_app_automation(
                    owner,
                    catalog,
                    config.automation_id,
                    config.revision,
                    AutomationStatus::Disabled,
                    budget(),
                )
                .map_err(app_error)?;
        }
        Ok(())
    }

    pub(super) async fn route_app_agent_notifications(
        &self,
        leases: &[crate::runtime::app_worker::AppWorkerLease],
    ) {
        if !self.owned.publication_activation.is_active() {
            return;
        }
        for lease in leases {
            if lease.is_stopped() {
                continue;
            }
            let catalog = lease.catalog();
            let result = self
                .route_app_agent_installation(lease.owner(), catalog.clone())
                .await;
            if let Err(error) = result {
                tracing::warn!(%error, "App agent notification remains pending");
            }
        }
    }

    pub(super) async fn route_app_agent_installation(
        &self,
        owner: &str,
        catalog: Arc<chariox_app_runtime::app_outbox::EventCatalog>,
    ) -> Result<(), DaemonError> {
        // Reconcile even without receipts: archive, removal, and Room end detach.
        for config in self
            .owned
            .list_app_automations(owner, catalog.clone(), budget())
            .map_err(app_error)?
            .into_iter()
            .filter(|c| {
                c.target.kind == AutomationTargetKind::Agent
                    && c.status != AutomationStatus::Disabled
            })
        {
            if !self.app_agent_notification_live(owner, &config) {
                self.owned
                    .deactivate_app_automation(
                        owner,
                        catalog.clone(),
                        config.automation_id,
                        config.revision,
                        AutomationStatus::Disabled,
                        budget(),
                    )
                    .map_err(app_error)?;
            }
        }
        let now = crate::session::unix_epoch_ms();
        let pending = self
            .owned
            .durable_state_store
            .pending_app_events_for_target(owner, &catalog, now, 8, AutomationTargetKind::Agent)
            .map_err(app_error)?;
        for receipt in pending {
            if now >= receipt.expires_at_ms {
                continue;
            }
            let candidate = self
                .owned
                .durable_state_store
                .app_event_candidate(owner, catalog.clone(), &receipt.receipt_id)
                .map_err(app_error)?;
            let config = candidate.configuration();
            if !self.app_agent_notification_live(owner, config)
                || config.status != AutomationStatus::Active
                || config.revision != receipt.automation_revision
            {
                continue;
            }
            let payload: serde_json::Value = serde_json::from_str(
                receipt
                    .payload
                    .as_deref()
                    .ok_or_else(|| app_error("App notification payload missing"))?,
            )
            .map_err(app_error)?;
            let input = serde_json::json!({"installation_id":catalog.installation_id(),"event_name":config.event_name,"occurrence_id":receipt.occurrence_id,"occurred_at_ms":receipt.occurred_at_ms,"payload":payload});
            if let Some(prompt_id) = self
                .queue_agent_notification_input(
                    &config.target.session_id,
                    &config.target.publication_id,
                    input,
                )
                .await?
            {
                self.owned
                    .durable_state_store
                    .mutate_app_automation(
                        owner,
                        catalog.clone(),
                        AppAutomationMutation::AgentQueued { receipt, prompt_id },
                        budget(),
                    )
                    .map_err(app_error)?;
            }
        }
        Ok(())
    }

    fn app_agent_notification_live(
        &self,
        owner: &str,
        config: &chariox_app_runtime::app_outbox::AutomationConfiguration,
    ) -> bool {
        let target = &config.target;
        if !self
            .owned
            .agent_store
            .get_agent(&target.endpoint_id)
            .is_ok_and(|agent| !agent.archived() && agent.session_id() == target.session_id)
        {
            return false;
        }
        let Ok(room) = self.owned.session_store.get_session(&target.session_id) else {
            return false;
        };
        room.owner_user_id() == owner && room.status() != crate::session::SessionStatus::Ended
            && room.agent_triggers().iter().any(|trigger| {
                trigger.id() == target.publication_id && trigger.agent_id() == target.endpoint_id
                    && trigger.created_by_user_id().is_some_and(|creator| room.has_member(creator))
                    && matches!(trigger.source(), AgentTriggerSource::Notification { source: AgentTriggerNotificationSource::App { .. }, subscription_id, .. } if *subscription_id == config.automation_id)
            })
    }
}
