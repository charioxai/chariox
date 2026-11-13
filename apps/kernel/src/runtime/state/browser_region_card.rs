//! MP-08 / MP-09 / MP-10 / MP-11: the kernel owns a card's live browser crop.
use super::*;
use crate::session::{RuntimeInteractionBrowserRegion, RuntimeInteractionView};
use crate::transport::runtime_tools::RequestPopupBrowserRegionArgs;

#[derive(serde::Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum RegionObservation {
    Present {
        region: Box<RuntimeInteractionBrowserRegion>,
    },
    ConditionMet {
        reason: String,
    },
    Unavailable {
        reason: String,
    },
}

fn region_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "runtime_tool_request_popup",
        message: message.into(),
    }
}

struct RegionWatch {
    watching: Arc<std::sync::Mutex<BTreeSet<String>>>,
    id: String,
}

#[derive(Default)]
struct ObservationFailures {
    since: Option<tokio::time::Instant>,
}

impl ObservationFailures {
    fn observed(&mut self) {
        self.since = None;
    }

    fn exhausted(&mut self) -> bool {
        let now = tokio::time::Instant::now();
        now.duration_since(*self.since.get_or_insert(now)) >= Duration::from_secs(15)
    }
}

impl Drop for RegionWatch {
    fn drop(&mut self) {
        self.watching
            .lock()
            .expect("browser card watches poisoned")
            .remove(&self.id);
    }
}

impl KernelRuntimeState {
    async fn observe_card_region(
        &self,
        session_id: &str,
        agent_id: &str,
        card_id: &str,
        target: &RequestPopupBrowserRegionArgs,
        previous: Option<&RuntimeInteractionBrowserRegion>,
        app_confirmation: bool,
    ) -> Result<RegionObservation, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(region_error(
                "browser region agent does not belong to this room",
            ));
        }
        let user = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        let mut params = serde_json::json!({"op":"popup_region", "card_id":card_id, "tab_id": target.tab_id, "ref":target.reference});
        if let Some(verification) = &target.verification {
            params["verification"] = serde_json::to_value(verification)
                .map_err(|_| region_error("invalid verification condition"))?;
        }
        if let Some(previous) = previous {
            params["generation"] = previous.generation.into();
            params["document_id"] = previous.document_id.clone().into();
            params["after"] = serde_json::json!({
                "rect":previous.rect,
                "viewport_width":previous.viewport_width,
                "viewport_height":previous.viewport_height,
            });
        }
        // The kernel shows its own App's preview: it needs no browser grant of the asking agent.
        let admitted = (!app_confirmation).then_some(agent_id);
        let result = self
            .kernel_browser_operation(&user, admitted, "host.browser", params)
            .await?;
        serde_json::from_value(result)
            .map_err(|_| region_error("browser region observation is invalid"))
    }

    /// The App's confirmation preview, found by the browser host in the App's own Tab; the
    /// agent and the App name no element.
    pub(super) async fn resolve_app_confirmation_view(
        &self,
        session_id: &str,
        agent_id: &str,
        card_id: &str,
        tab_id: &str,
    ) -> Result<RuntimeInteractionView, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(region_error(
                "browser region agent does not belong to this room",
            ));
        }
        let user = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        let params = serde_json::json!({"op":"popup_region", "card_id":card_id, "tab_id":tab_id, "app_confirmation":true});
        let result = self
            .kernel_browser_operation(&user, None, "host.browser", params)
            .await?;
        match serde_json::from_value(result)
            .map_err(|_| region_error("browser region observation is invalid"))?
        {
            RegionObservation::Present { region } => Ok(RuntimeInteractionView {
                browser_region: *region,
            }),
            RegionObservation::ConditionMet { reason }
            | RegionObservation::Unavailable { reason } => Err(region_error(&reason)),
        }
    }

    pub(super) async fn release_app_confirmation_view(
        &self,
        agent_id: &str,
        card_id: &str,
        tab_id: &str,
    ) {
        self.release_browser_region_card(agent_id, card_id, tab_id, true)
            .await;
    }

    pub(super) async fn resolve_popup_view(
        &self,
        session_id: &str,
        agent_id: &str,
        card_id: &str,
        target: &RequestPopupBrowserRegionArgs,
    ) -> Result<RuntimeInteractionView, DaemonError> {
        match self
            .observe_card_region(session_id, agent_id, card_id, target, None, false)
            .await?
        {
            RegionObservation::Present { region } => Ok(RuntimeInteractionView {
                browser_region: *region,
            }),
            RegionObservation::ConditionMet { .. } => Err(region_error(
                "browser element is no longer current; take a fresh snapshot",
            )),
            RegionObservation::Unavailable { reason } => Err(region_error(&reason)),
        }
    }

    /// Recovery and registration share the same guard, so a recovered card
    /// cannot acquire a second watcher. Each read waits for changed geometry.
    pub(super) fn resume_browser_region_cards(&self) {
        let rooms: BTreeSet<String> = self
            .owned
            .pending_interactions
            .write()
            .values()
            .filter(|p| p.belongs_to(&self.owned.session_store))
            .map(|p| p.session_id.clone())
            .collect();
        for room in rooms {
            let Ok(session) = self.owned.session_store.get_session(&room) else {
                continue;
            };
            for interaction in session
                .active_interactions()
                .iter()
                .filter(|i| i.view().is_some())
            {
                let Some(agent) = interaction.agent_id() else {
                    continue;
                };
                let agent = agent.to_owned();
                let tab = interaction
                    .view()
                    .expect("filtered browser view")
                    .browser_region
                    .tab_id
                    .clone();
                let id = interaction.id().to_owned();
                let app_confirmation = interaction.app_confirmation().is_some();
                if !self
                    .owned
                    .browser_region_cards
                    .lock()
                    .expect("browser card watches poisoned")
                    .insert(id.clone())
                {
                    continue;
                }
                let watch = RegionWatch {
                    watching: self.owned.browser_region_cards.clone(),
                    id,
                };
                let state = self.clone();
                let room = room.clone();
                tokio::spawn(async move {
                    state.follow_browser_region_card(&room, &watch.id).await;
                    state
                        .release_browser_region_card(&agent, &watch.id, &tab, app_confirmation)
                        .await;
                    drop(watch);
                });
            }
        }
    }

    async fn follow_browser_region_card(&self, room: &str, id: &str) {
        let mut failures = ObservationFailures::default();
        loop {
            let Ok(session) = self.owned.session_store.get_session(room) else {
                return;
            };
            let Some(card) = session.active_interactions().iter().find(|i| i.id() == id) else {
                return;
            };
            let (Some(agent), Some(view)) = (card.agent_id(), card.view()) else {
                return;
            };
            let app_confirmation = card.app_confirmation().is_some();
            let previous = &view.browser_region;
            let target = RequestPopupBrowserRegionArgs {
                tab_id: previous.tab_id.clone(),
                reference: previous.reference.clone(),
                verification: previous.verification.clone(),
            };
            let result = self
                .observe_card_region(room, agent, id, &target, Some(previous), app_confirmation)
                .await;
            match result {
                Ok(RegionObservation::Present { region }) => {
                    failures.observed();
                    if let Err(error) = self
                        .owned
                        .update_browser_region_card(room, id, previous, *region)
                    {
                        tracing::warn!(interaction_id=%id, error=%crate::secret_redaction::redact_secrets(&error.to_string()), "browser card projection retained for retry");
                        tokio::time::sleep(Duration::from_millis(250)).await;
                    }
                }
                Ok(RegionObservation::ConditionMet { reason }) => {
                    if self.close_browser_region_card(room, id, reason).is_ok() {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                Ok(RegionObservation::Unavailable { reason }) => {
                    if self
                        .close_unavailable_browser_region_card(room, id, reason)
                        .is_ok()
                    {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                Err(DaemonError::UserDomainRefused {
                    reason: crate::error::UserDomainRefusalReason::StaleReference,
                }) => {
                    if self
                        .close_unavailable_browser_region_card(
                            room,
                            id,
                            "Browser element or its document is no longer available".into(),
                        )
                        .is_ok()
                    {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                Err(error) => {
                    tracing::debug!(interaction_id=%id, error=%crate::secret_redaction::redact_secrets(&error.to_string()), "browser card observation will retry");
                    let access_revoked = matches!(
                        error,
                        DaemonError::UserDomainRefused {
                            reason: crate::error::UserDomainRefusalReason::NotGranted
                                | crate::error::UserDomainRefusalReason::ForeignOwner
                                | crate::error::UserDomainRefusalReason::NotFocusedAgent
                                | crate::error::UserDomainRefusalReason::StaleEpoch
                        }
                    );
                    if (access_revoked || failures.exhausted()) && self.close_unavailable_browser_region_card(room, id, if access_revoked { "Browser access was revoked" } else { "Browser region observation remained unavailable for 15 seconds; take a fresh snapshot and ask again" }.into()).is_ok() {
                        return;
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }

    pub(super) fn close_browser_region_card(
        &self,
        room: &str,
        id: &str,
        reason: String,
    ) -> Result<(), DaemonError> {
        let session = self.owned.session_store.get_session(room)?;
        if let Some(region) = session
            .active_interactions()
            .iter()
            .find(|card| card.id() == id)
            .and_then(|card| card.view())
            .map(|view| &view.browser_region)
        {
            if let Some(condition) = &region.verification {
                let proof: serde_json::Value = serde_json::from_str(&reason)
                    .map_err(|_| region_error("Verification has no positive response"))?;
                let receipt: crate::session::RuntimeInteractionBrowserVerificationResult =
                    serde_json::from_value(proof["verification_result"].clone())
                        .map_err(|_| region_error("Verification response is invalid"))?;
                if proof["card_id"] != id
                    || proof["tab_id"] != region.tab_id
                    || proof["document_id"] != region.document_id
                    || proof["generation"] != region.generation
                    || receipt.challenge_id != condition.challenge_id
                    || receipt.request_id.is_empty()
                    || !receipt.input_delivered
                    || !receipt
                        .http_status
                        .is_some_and(|status| (200..300).contains(&status))
                    || receipt.result
                        != crate::session::RuntimeInteractionBrowserVerificationOutcome::Accepted
                {
                    return Err(region_error(
                        "Verification has not positively completed for this card",
                    ));
                }
            }
        }
        self.owned.settle_card_condition_met(room, id, reason)?;
        self.schedule_question_answer_delivery(id)
    }

    async fn release_browser_region_card(
        &self,
        agent: &str,
        card: &str,
        tab: &str,
        app_confirmation: bool,
    ) {
        let Ok(agent_record) = self.owned.agent_store.get_agent(agent) else {
            return;
        };
        let user = self.provider_account_authority_owner_user_id(agent_record.owner_user_id());
        // Revocation can prevent cleanup admission. The bounded controller
        // lease then expires without another watcher renewal.
        let admitted = (!app_confirmation).then_some(agent);
        let _ = self.kernel_browser_operation(&user, admitted, "host.browser", serde_json::json!({"op":"popup_region", "card_id":card, "tab_id":tab, "release":true})).await;
    }

    pub(super) fn close_unavailable_browser_region_card(
        &self,
        room: &str,
        id: &str,
        reason: String,
    ) -> Result<(), DaemonError> {
        self.owned.settle_card_unavailable(room, id, reason)?;
        self.schedule_question_answer_delivery(id)
    }
}

impl KernelRuntimeOwnedState {
    pub(super) fn update_browser_region_card(
        &self,
        room: &str,
        id: &str,
        expected: &RuntimeInteractionBrowserRegion,
        region: RuntimeInteractionBrowserRegion,
    ) -> Result<(), DaemonError> {
        let _mutation = self
            .pending_interactions
            .mutation
            .lock()
            .map_err(|_| region_error("interaction store unavailable"))?;
        let activity = self.begin_managed_activity_mutation();
        let mut sessions = self.session_store.write();
        let mut session = sessions.get_session(room)?;
        let Some(card) = session.active_interactions().iter().find(|i| i.id() == id) else {
            return Ok(());
        };
        if card.view().is_none_or(|v| v.browser_region != *expected) || expected == &region {
            return Ok(());
        }
        if region.tab_id != expected.tab_id
            || region.reference != expected.reference
            || region.generation != expected.generation
            || region.document_id != expected.document_id
            || region.verification != expected.verification
        {
            return Err(region_error(
                "browser card observation changed its document binding",
            ));
        }
        session.update_active_interaction_view(
            id,
            RuntimeInteractionView {
                browser_region: region,
            },
        );
        sessions.restore_session(session);
        drop(sessions);
        activity.record();
        self.session_snapshot(room)?;
        self.terminal_stream.notify_terminal_projection_change(room);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn browser_region_card_source_failure_budget_resets_only_on_observation() {
        let mut failures = ObservationFailures::default();
        assert!(!failures.exhausted());
        tokio::time::advance(Duration::from_secs(14)).await;
        assert!(!failures.exhausted());
        failures.observed();
        assert!(!failures.exhausted());
        tokio::time::advance(Duration::from_secs(14)).await;
        assert!(!failures.exhausted());
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(failures.exhausted());
    }
}
