//! A `person_confirmed` App action. The App's own signed component previews the exact
//! input in its view; the kernel frames that preview with the verified App and publisher
//! and asks the person on one card. Only the person's Confirm lets the action run: no
//! grant, session approval or `/sudo` stands in for it, and no agent may answer the card.

use super::*;
use crate::local::AppConfirmationFrame;
use crate::runtime::state::runtime_vault_unlock_state::CloseInteractionOnDrop;

const CONFIRMATION_TIMEOUT_SEC: u64 = 300;
const CONFIRM: &str = "confirm";
const CANCEL: &str = "cancel";

/// The App's own Tab and origin: a preview is shown from there only.
fn confirmation_tab(context: &ViewContext) -> Result<(String, u64, String), ViewError> {
    let ViewHost::User { retained, .. } = &context.host else {
        return Err(failure(
            "confirmation_unavailable",
            "This action needs the person's confirmation in an App Tab: open the App in a Tab first.",
        ));
    };
    let view = &retained.1;
    let browser = view
        .browser
        .as_ref()
        .ok_or_else(|| failure("unknown_view", "The App view closed; reopen it"))?;
    Ok((
        browser.tab_id.clone(),
        browser.generation,
        view.origin.clone(),
    ))
}

/// An abandoned call (the turn ended while the card waited) still removes its preview, in
/// the view's turn so a later preview is not cleared by mistake.
struct PreviewCleanup {
    state: KernelRuntimeState,
    context: Option<ViewContext>,
}

impl Drop for PreviewCleanup {
    fn drop(&mut self) {
        let (Some(context), Ok(handle)) =
            (self.context.take(), tokio::runtime::Handle::try_current())
        else {
            return;
        };
        let state = self.state.clone();
        handle.spawn(async move {
            if let Ok(_turn) = state.view_turn(&context).await {
                let _ = state
                    .view_call(&context, ViewAgentCallKind::ClearConfirmation, None, None)
                    .await;
            }
        });
    }
}

impl KernelRuntimeState {
    /// Returns `Ok` only after the person confirmed this invocation. The caller holds the
    /// view's turn, so nothing else runs in the page while its preview is shown.
    pub(super) async fn confirm_view_action(
        &self,
        run_id: &str,
        agent: &crate::agent::AgentInstance,
        context: &ViewContext,
        action: &UiActionDeclaration,
        input: &Value,
    ) -> Result<(), ViewError> {
        confirmation_tab(context)?;
        let current_turn = || {
            self.authorize_room_provider_epoch(Some(agent.id()), Some(run_id))
                .map_err(|_| {
                    failure(
                        "confirmation_expired",
                        "The confirmation belongs to a turn that has ended",
                    )
                })
        };
        current_turn()?;
        self.view_still_bound(context)?;
        let frame = self
            .app_confirmation_frame(&context.binding, &action.name)
            .await
            .map_err(|error| failure("confirmation_unavailable", error.to_string()))?;
        let registrations = self
            .view_call(context, ViewAgentCallKind::Registrations, None, None)
            .await?;
        declared_actions::check_registered(&context.declared, &registrations, &action.name)
            .map_err(action_error)?;
        self.view_call(
            context,
            ViewAgentCallKind::PrepareConfirmation,
            Some((&action.name, input.clone())),
            None,
        )
        .await?;
        let mut preview = PreviewCleanup {
            state: self.clone(),
            context: Some(context.clone()),
        };
        let decision = self
            .ask_person_to_confirm(run_id, agent, context, frame, input)
            .await;
        // Every way out removes the preview from the page.
        let cleared = self
            .view_call(context, ViewAgentCallKind::ClearConfirmation, None, None)
            .await;
        preview.context = None;
        decision?;
        cleared?;
        current_turn()?;
        self.ensure_context_current(context).await?;
        context.authority.check()?;
        Ok(())
    }

    pub(super) async fn ask_person_to_confirm(
        &self,
        run_id: &str,
        agent: &crate::agent::AgentInstance,
        context: &ViewContext,
        frame: AppConfirmationFrame,
        input: &Value,
    ) -> Result<(), ViewError> {
        let (tab_id, generation, origin) = confirmation_tab(context)?;
        let (tab_id, origin) = (tab_id.as_str(), origin.as_str());
        let unavailable =
            |error: DaemonError| failure("confirmation_unavailable", error.to_string());
        let session_id = agent.session_id();
        let id = format!("app-confirm-{:032x}", rand::random::<u128>());
        let view = self
            .resolve_app_confirmation_view(session_id, agent.id(), &id, tab_id, generation)
            .await
            .map_err(unavailable)?;
        if view.browser_region.origin != origin.trim_end_matches('/') {
            self.release_app_confirmation_view(agent.id(), &id, tab_id)
                .await;
            return Err(failure(
                "confirmation_unavailable",
                "The preview is not shown by the App's own page",
            ));
        }
        let action_name = frame.action_name.clone();
        // A client that cannot show the App's preview still reads the exact input, in the kernel's words.
        let shown: String = serde_json::to_string(input)
            .unwrap_or_default()
            .chars()
            .take(2000)
            .collect();
        let interaction = RuntimeInteraction::new(
            id.clone(),
            agent.id(),
            RuntimeInteractionKind::Choice,
            RuntimeInteractionLevel::Warning,
            Some(format!("Confirm {action_name}")),
            format!(
                "{} by {} asks you to confirm {action_name}.\nInput: {shown}",
                frame.app_name, frame.publisher_name
            ),
            vec![
                RuntimeInteractionChoice::new(
                    CONFIRM,
                    "Confirm",
                    CONFIRM,
                    Some(RuntimeInteractionChoiceStyle::Primary),
                ),
                RuntimeInteractionChoice::new(
                    CANCEL,
                    "Cancel",
                    CANCEL,
                    Some(RuntimeInteractionChoiceStyle::Danger),
                ),
            ],
            None,
            Some(CONFIRMATION_TIMEOUT_SEC),
            Some(CANCEL.into()),
        )
        .with_app_confirmation(frame)
        .with_view(view)
        .with_native_origin(self.owned.capture_native_interaction_origin(
            session_id,
            agent.id(),
            run_id,
        ));
        let receiver = self
            .create_runtime_interaction(session_id, interaction)
            .await
            .map_err(unavailable)?;
        // An abandoned call withdraws its card instead of leaving a dead one.
        let _close = CloseInteractionOnDrop {
            state: self.clone(),
            session_id: session_id.to_owned(),
            interaction_id: id.clone(),
        };
        self.resume_browser_region_cards();
        let state = self.clone();
        let (timeout_session, timeout_id) = (session_id.to_owned(), id);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(CONFIRMATION_TIMEOUT_SEC)).await;
            let _ = state
                .timeout_runtime_interaction(&timeout_session, &timeout_id)
                .await;
        });
        let resolution = tokio::select! {
            resolution = receiver => resolution,
            () = context.authority.cancelled() => return Err(app_view_authority::not_granted()),
        };
        match resolution {
            Ok(answer) if answer.status == "answered" && answer.choice_id.as_deref() == Some(CONFIRM) => Ok(()),
            Ok(answer) if answer.status == "answered" => Err(failure(
                "confirmation_declined",
                format!("The person did not confirm {action_name}. Nothing ran. Do not retry it unless they ask."),
            )),
            _ => Err(failure(
                "confirmation_expired",
                format!("The person did not confirm {action_name} in time. Nothing ran."),
            )),
        }
    }
}
