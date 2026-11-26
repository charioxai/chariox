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

/// What the watcher does after it tried to bind a card to the page its Tab shows now.
enum PageChangeFlow {
    /// Observe again.
    Continue,
    /// The card is settled: stop watching.
    Done,
}

/// Rows 546 and 548 (Miguel's D3 decision, row 78): a person card survives a navigation of its own Tab and any kernel
/// restart, hot or cold, whenever its Tab is restored. It ends by the person's choice, the agent's cancel, its page-text
/// condition, or its Tab closing; and, with a plain reason, only when its Tab cannot be restored (the page cannot be shown
/// again within the observation budget). A verification's proof belongs to one document: on the new page the card stays
/// pending without it (it re-arms only if the agent raises or updates the card for that page).
fn keeps_card_pending(
    reason: &str,
    previous: &RuntimeInteractionBrowserRegion,
    app_confirmation: bool,
    waiting_for_text: bool,
) -> bool {
    if app_confirmation {
        return false;
    }
    // A card that waits for page text or a proof is observed as unavailable, a plain card as condition met.
    let waiting = previous.done_text.is_some() || previous.verification.is_some();
    match reason {
        // A restart (a hot reload is a planned one) and a navigation keep the card the same way: the person must not notice
        // either. A card waiting for text goes on waiting on the page that came back; a proof is lost with the old document.
        "Browser restarted" | "Browser document changed" => waiting == waiting_for_text,
        _ => false,
    }
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

/// What the watcher does after one attempt to start the browser it found unavailable.
#[derive(Debug, PartialEq, Eq)]
enum StartRecovery {
    /// The browser is back: observe again.
    Restarted,
    /// It could not start (or was stopped on request) yet: try again within the budget.
    Retry,
    /// The 15 second budget ran out: settle the card unavailable.
    GiveUp,
}

/// A failed start counts toward the same budget as a failed observation, so a controller that never
/// comes back, or a browser that stays stopped, ends the card instead of keeping it pending.
fn after_start_attempt(
    failures: &mut ObservationFailures,
    started: &Result<(), DaemonError>,
) -> StartRecovery {
    match started {
        Ok(()) => {
            failures.observed();
            StartRecovery::Restarted
        }
        Err(_) if failures.exhausted() => StartRecovery::GiveUp,
        Err(_) => StartRecovery::Retry,
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
        if let Some(done_text) = &target.done_text {
            params["done_text"] = done_text.clone().into();
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

    /// After a kernel restart the card's Tab is back with a new document: ask the controller to
    /// bind the card to the Tab's whole page.
    async fn rebind_card_region(
        &self,
        session_id: &str,
        agent_id: &str,
        card_id: &str,
        tab_id: &str,
        done_text: Option<&str>,
    ) -> Result<RegionObservation, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(region_error(
                "browser region agent does not belong to this room",
            ));
        }
        let user = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        let mut params = serde_json::json!({"op":"popup_region", "card_id":card_id, "tab_id":tab_id, "rebind":true});
        if let Some(text) = done_text {
            params["done_text"] = serde_json::json!(text);
        }
        let result = self
            .kernel_browser_operation(&user, Some(agent_id), "host.browser", params)
            .await?;
        serde_json::from_value(result)
            .map_err(|_| region_error("browser region observation is invalid"))
    }

    /// The kernel restarted and ended its browser: the card's page comes back when the browser
    /// starts again, the same way an agent's own read would start it.
    pub(crate) async fn start_card_browser(&self, agent_id: &str) -> Result<(), DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let user = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        self.kernel_browser_operation(
            &user,
            Some(agent_id),
            "host.browser",
            // `unless_stopped`: the host answers `browser_unavailable` both for a controller a kernel
            // restart ended and for one a `stop` ended. Only the first is the card's to bring back.
            serde_json::json!({"op":"start","unless_stopped":true}),
        )
        .await?;
        Ok(())
    }

    /// The App's confirmation preview, found by the browser host in the App's own Tab; the
    /// agent and the App name no element.
    pub(super) async fn resolve_app_confirmation_view(
        &self,
        session_id: &str,
        agent_id: &str,
        card_id: &str,
        tab_id: &str,
        generation: u64,
    ) -> Result<RuntimeInteractionView, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(region_error(
                "browser region agent does not belong to this room",
            ));
        }
        let user = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        let params = serde_json::json!({"op":"popup_region", "card_id":card_id, "tab_id":tab_id, "generation":generation, "app_confirmation":true});
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

    /// The Tab shows another page than the card's (a restart brought the browser back, or the page navigated):
    /// the card is bound again to the Tab's whole page and the person carries on there. A card that waits for
    /// page text keeps waiting for it on the new page; if it is already there the card completes.
    async fn rebind_after_page_change(
        &self,
        room: &str,
        agent: &str,
        id: &str,
        previous: &RuntimeInteractionBrowserRegion,
        reason: &str,
        failures: &mut ObservationFailures,
    ) -> PageChangeFlow {
        // The rebind itself is retried: a second navigation during it leaves the old reference stale, so going back
        // to the outer poll would settle the card unavailable while the person is still signing in.
        loop {
            match self
                .rebind_card_region(
                    room,
                    agent,
                    id,
                    &previous.tab_id,
                    previous.done_text.as_deref(),
                )
                .await
            {
                Ok(RegionObservation::Present { region }) => {
                    failures.observed();
                    if let Err(error) = self
                        .owned
                        .rebind_browser_region_card(room, id, previous, *region)
                    {
                        tracing::warn!(interaction_id=%id, error=%crate::secret_redaction::redact_secrets(&error.to_string()), "browser card rebind retained for retry");
                        tokio::time::sleep(Duration::from_millis(250)).await;
                    }
                    return PageChangeFlow::Continue;
                }
                // The text the card waits for is on the page now: the card's condition is met.
                Ok(RegionObservation::ConditionMet { reason }) if previous.done_text.is_some() => {
                    if self.close_browser_region_card(room, id, reason).is_ok() {
                        return PageChangeFlow::Done;
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    return PageChangeFlow::Continue;
                }
                Ok(_) | Err(_) => {
                    if !failures.exhausted() {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                    // The page change is not the person finishing, and the page was never shown again: the agent
                    // is told the card is unavailable, never that its step is done.
                    let message = if reason == "Browser restarted" {
                        "Browser restarted and the page could not be shown again; take a fresh snapshot and ask again"
                    } else {
                        "The page changed and could not be shown again; take a fresh snapshot and ask again"
                    };
                    if self
                        .close_unavailable_browser_region_card(room, id, message.into())
                        .is_ok()
                    {
                        return PageChangeFlow::Done;
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    return PageChangeFlow::Continue;
                }
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
                done_text: previous.done_text.clone(),
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
                    // A restart or a navigation of the card's own Tab is not the person finishing: the
                    // card keeps its Tab and is bound again to the page the Tab shows now.
                    if keeps_card_pending(&reason, previous, app_confirmation, false) {
                        match Box::pin(self.rebind_after_page_change(
                            room,
                            agent,
                            id,
                            previous,
                            &reason,
                            &mut failures,
                        ))
                        .await
                        {
                            PageChangeFlow::Continue => continue,
                            PageChangeFlow::Done => return,
                        }
                    }
                    if self.close_browser_region_card(room, id, reason).is_ok() {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                Ok(RegionObservation::Unavailable { reason }) => {
                    // A card that waits for page text is still waiting after its Tab navigated: the text can
                    // appear on the page the Tab shows now.
                    if keeps_card_pending(&reason, previous, app_confirmation, true) {
                        match Box::pin(self.rebind_after_page_change(
                            room,
                            agent,
                            id,
                            previous,
                            &reason,
                            &mut failures,
                        ))
                        .await
                        {
                            PageChangeFlow::Continue => continue,
                            PageChangeFlow::Done => return,
                        }
                    }
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
                    // The controller is stopped after a restart: start it, then observe again.
                    if !app_confirmation && error.to_string().contains("browser_unavailable") {
                        let started = self.start_card_browser(agent).await;
                        if after_start_attempt(&mut failures, &started) == StartRecovery::GiveUp
                            && self
                                .close_unavailable_browser_region_card(
                                    room,
                                    id,
                                    "Browser is stopped or could not be started for 15 seconds; take a fresh snapshot and ask again".into(),
                                )
                                .is_ok()
                        {
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        continue;
                    }
                    // A different owner is the only refusal: the agent's own access needs no grant,
                    // so a restart or a reload never closes a card that shows a live page.
                    let foreign = matches!(
                        error,
                        DaemonError::UserDomainRefused {
                            reason: crate::error::UserDomainRefusalReason::ForeignOwner
                        }
                    );
                    if (foreign || failures.exhausted()) && self.close_unavailable_browser_region_card(room, id, if foreign { "Browser is no longer available to this agent" } else { "Browser region observation remained unavailable for 15 seconds; take a fresh snapshot and ask again" }.into()).is_ok() {
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
        // A failed cleanup is not retried: the bounded controller lease then
        // expires without another watcher renewal.
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
    /// The card's Tab came back with a new document after a restart: bind the card to it. The
    /// card keeps its identity and its Tab; only a plain card (no proof, no awaited text) rebinds.
    pub(super) fn rebind_browser_region_card(
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
        if card.view().is_none_or(|v| v.browser_region != *expected) {
            return Ok(());
        }
        // A rebind never creates or keeps a proof: the proof belonged to the old document, the card goes on without it.
        if region.tab_id != expected.tab_id
            || region.verification.is_some()
            || region.done_text != expected.done_text
        {
            return Err(region_error(
                "browser card rebind changed its Tab or its proof",
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

    fn region(done_text: Option<&str>, verification: bool) -> RuntimeInteractionBrowserRegion {
        RuntimeInteractionBrowserRegion {
            tab_id: "tab-one".into(),
            reference: "backend:42".into(),
            generation: 1,
            document_id: "doc-one".into(),
            origin: "https://claude.ai".into(),
            rect: crate::session::RuntimeInteractionBrowserRect {
                x: 0,
                y: 0,
                width: 100,
                height: 40,
            },
            viewport_width: 1280,
            viewport_height: 800,
            verification: verification.then(|| {
                serde_json::from_value(serde_json::json!({
                    "challenge_id": "c",
                    "url": "https://example.com/verify",
                    "accepted_json_pointer": "/accepted"
                }))
                .expect("verification config")
            }),
            verification_result: None,
            done_text: done_text.map(str::to_owned),
        }
    }

    /// Rows 546 and 548: a navigation of the card's own Tab or a kernel restart is not the person's step ending: every person
    /// card is kept, whatever it waits for.
    #[test]
    fn a_navigation_or_a_restart_keeps_every_person_card_pending_until_its_tab_cannot_be_restored()
    {
        let plain = region(None, false);
        let waiting = region(Some("Welcome home"), false);
        let proof = region(None, true);
        // A plain card is observed as condition met, a card waiting for text as unavailable.
        assert!(keeps_card_pending(
            "Browser document changed",
            &plain,
            false,
            false
        ));
        assert!(keeps_card_pending(
            "Browser document changed",
            &waiting,
            false,
            true
        ));
        // Row 548 (D3-7): a hot reload or restart is a planned upgrade the person must not notice, so it keeps every person
        // card pending: a card waiting for text goes on waiting on the page that came back.
        assert!(keeps_card_pending(
            "Browser restarted",
            &plain,
            false,
            false
        ));
        assert!(keeps_card_pending(
            "Browser restarted",
            &waiting,
            false,
            true
        ));
        // A verification belongs to one document, an App confirmation is not a person step on a page.
        // A verification card survives the navigation as pending; its proof lapses with the old document.
        assert!(keeps_card_pending(
            "Browser document changed",
            &proof,
            false,
            true
        ));
        // Its proof is lost with the old document across a restart too; the card stays pending without it.
        assert!(keeps_card_pending("Browser restarted", &proof, false, true));
        assert!(!keeps_card_pending(
            "Browser restarted",
            &plain,
            true,
            false
        ));
        assert!(!keeps_card_pending(
            "Browser document changed",
            &plain,
            true,
            false
        ));
        // The Tab closing or its element disappearing still ends the card.
        assert!(!keeps_card_pending(
            "Browser element disappeared",
            &plain,
            false,
            false
        ));
        assert!(!keeps_card_pending(
            "Browser element disappeared",
            &waiting,
            false,
            true
        ));
    }

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

    #[tokio::test(start_paused = true)]
    async fn failed_browser_starts_count_toward_the_card_failure_budget() {
        let refused: Result<(), DaemonError> = Err(region_error("browser_unavailable: stopped"));
        let mut failures = ObservationFailures::default();
        assert_eq!(
            after_start_attempt(&mut failures, &refused),
            StartRecovery::Retry
        );
        tokio::time::advance(Duration::from_secs(14)).await;
        assert_eq!(
            after_start_attempt(&mut failures, &refused),
            StartRecovery::Retry
        );
        // A start that works resets the budget.
        assert_eq!(
            after_start_attempt(&mut failures, &Ok(())),
            StartRecovery::Restarted
        );
        tokio::time::advance(Duration::from_secs(14)).await;
        assert_eq!(
            after_start_attempt(&mut failures, &refused),
            StartRecovery::Retry
        );
        // Persistent failure ends the card with the unavailable settlement.
        tokio::time::advance(Duration::from_secs(15)).await;
        assert_eq!(
            after_start_attempt(&mut failures, &refused),
            StartRecovery::GiveUp
        );
    }
}
