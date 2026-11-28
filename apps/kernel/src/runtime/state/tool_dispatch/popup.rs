//! MP-08 / MP-09 / MP-10 / MP-11: both question modes share validation and cards.
use super::*;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use chariox_relay::protocol::ClientTarget;

/// The provider cuts a blocking tool call at 300 s and tells neither the kernel nor the card, so a blocking
/// card ends itself just before that instead of staying open for a call that is gone.
const BLOCKING_POPUP_MAX_SECONDS: u64 = 280;

pub(in crate::runtime::state) fn blocking_popup_timeout_seconds(requested: Option<u64>) -> u64 {
    requested
        .unwrap_or(BLOCKING_POPUP_MAX_SECONDS)
        .min(BLOCKING_POPUP_MAX_SECONDS)
}

/// A later prompt can continue the same task, so scope retries to its asking turn.
fn keyed_interaction_id(session_id: &str, agent_id: &str, prompt_id: &str, key: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(
        serde_json::to_vec(&(session_id, agent_id, prompt_id, key)).expect("identities serialize"),
    );
    let mut id = [0u8; 16];
    id.copy_from_slice(&digest[..16]);
    format!("interaction-{:032x}", u128::from_be_bytes(id))
}

fn popup_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "runtime_tool_request_popup",
        message: message.into(),
    }
}

/// The choice every agent question carries so the person can always decline.
pub(in crate::runtime::state) const CANCEL_CHOICE_ID: &str = "cancel";

/// Rows 574 / 655: the person's own word that a plain browser card's step is done (never a proof: the agent checks the page).
pub(in crate::runtime::state) const DONE_CHOICE_ID: &str = "done";

/// Row 655: what the agent is told when its card's element has no visible area when the card opens.
pub(in crate::runtime::state) const HIDDEN_REGION_NOTE: &str = "The element is not visible on the page now (hidden, not opened yet, or off screen). The card shows the page, or the part of it the person can act on now (for example the checkbox), so the person can take the step that reveals it, and it moves to the element when it appears. Do not tell the person that the element or challenge is open.";

/// Rows 574 / 655 (D107: a person card ends by the person's Done or Cancel, the agent, or its condition): a plain
/// browser card has no condition that ends it, so the person says when the step is done. A card that waits for page
/// text or a verification closes by itself and carries no Done; an agent's own Done is kept, never doubled.
pub(in crate::runtime::state) fn add_person_done(
    view: Option<&crate::transport::runtime_tools::RequestPopupViewArgs>,
    choices: &mut Vec<crate::session::RuntimeInteractionChoice>,
) {
    let plain = view.is_some_and(|view| {
        view.browser_region.done_text.is_none() && view.browser_region.verification.is_none()
    });
    if plain
        && !choices.iter().any(|choice| {
            choice.id() == DONE_CHOICE_ID || choice.label().trim().eq_ignore_ascii_case("done")
        })
    {
        choices.push(crate::session::RuntimeInteractionChoice::new(
            DONE_CHOICE_ID,
            "Done",
            DONE_CHOICE_ID,
            Some(crate::session::RuntimeInteractionChoiceStyle::Primary),
        ));
    }
}

/// Row 655: the note for a card whose element has no visible area (the documented empty crop).
pub(in crate::runtime::state) fn hidden_region_note(
    view: Option<&crate::session::RuntimeInteractionView>,
) -> Option<&'static str> {
    view.filter(|view| view.browser_region.rect.width == 0 || view.browser_region.rect.height == 0)
        .map(|_| HIDDEN_REGION_NOTE)
}

/// Row 655: an agent choice labelled like the built-in Cancel with another id would show a second Cancel.
fn duplicates_cancel(id: &str, label: &str) -> bool {
    id != CANCEL_CHOICE_ID && label.trim().eq_ignore_ascii_case("cancel")
}

/// What the agent reads as the outcome: declining is a definitive result of its
/// own, never a timeout or a missing answer.
pub(in crate::runtime::state) fn popup_status<'a>(
    status: &'a str,
    choice_id: Option<&str>,
) -> &'a str {
    if status == "answered" && choice_id == Some(CANCEL_CHOICE_ID) {
        "cancelled"
    } else {
        status
    }
}

impl KernelRuntimeState {
    /// Row 655 (one card per Tab, Mac 11-26): the person sees one card for a page. When the agent raises a plain
    /// browser card on a Tab where its own plain card is open, that older card ends `unavailable` ("Replaced by the
    /// agent's newer card on the same page") before the new one registers, so the card and its durable wake stay one
    /// record. Hand-offs, App confirmations, verification, page-text and condition-bound (`until`) cards on either side
    /// keep their own lifecycle and are never replaced. Returns the replaced card's id.
    pub(in crate::runtime::state) fn one_card_per_tab(
        &self,
        session_id: &str,
        agent_id: &str,
        request: &crate::session::RuntimeInteraction,
    ) -> Result<Option<String>, DaemonError> {
        let plain = |card: &crate::session::RuntimeInteraction| {
            card.closes_with_wake().is_none()
                && card.handoff().is_none()
                && card.app_confirmation().is_none()
                && card.view().is_some_and(|view| {
                    view.browser_region.verification.is_none() && view.browser_region.done_text.is_none()
                })
        };
        if !plain(request) {
            return Ok(None);
        }
        let tab = &request.view().expect("plain card has a view").browser_region.tab_id;
        let session = self.owned.session_store.get_session(session_id)?;
        let Some(existing) = session
            .active_interactions()
            .iter()
            .find(|card| {
                card.id() != request.id()
                    && card.agent_id() == Some(agent_id)
                    && plain(card)
                    && card.view().is_some_and(|open| open.browser_region.tab_id == *tab)
            })
            .map(|card| card.id().to_owned())
        else {
            return Ok(None);
        };
        self.close_unavailable_browser_region_card(
            session_id,
            &existing,
            "Replaced by the agent's newer card on the same page".into(),
        )?;
        Ok(Some(existing))
    }

    /// The wake a card follows must be this agent's own live source, so the card
    /// can never be closed by someone else's event or wait for one that is over.
    fn card_condition_wake(
        &self,
        session_id: &str,
        agent_id: &str,
        wake_id: &str,
    ) -> Result<String, DaemonError> {
        let wake = self
            .owned
            .durable_state_store
            .agent_wake(wake_id)?
            .filter(|wake| wake.room_id == session_id && wake.agent_id == agent_id)
            .ok_or_else(|| {
                popup_error(&format!(
                    "until `{wake_id}` is not a wake source you armed in this room"
                ))
            })?;
        if wake.kind == super::super::interaction_wake::KIND {
            return Err(popup_error("a card cannot follow another card"));
        }
        if !matches!(wake.state.as_str(), "scheduled" | "starting" | "running") {
            return Err(popup_error(&format!(
                "until `{wake_id}` already ended ({}); check it with events.wakes instead of asking",
                wake.state
            )));
        }
        Ok(wake.id)
    }

    pub(super) async fn dispatch_popup_tool(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let args = serde_json::from_value::<crate::transport::runtime_tools::RequestPopupArgs>(
            arguments.clone(),
        )
        .map_err(|error| DaemonError::LocalTransport {
            operation: "runtime_tool_request_popup",
            message: format!("invalid tool arguments: {error}"),
        })?;
        if args.wait == Some(false) {
            let current = self
                .owned
                .provider_store
                .get_tool_run_for_agent(
                    provider_run.session_id(),
                    provider_run
                        .agent_instance_id()
                        .ok_or_else(|| popup_error("provider run has no agent"))?,
                )
                .ok_or_else(|| popup_error("asking provider run is unavailable"))?;
            if current.id() != provider_run.id() {
                return Err(popup_error("stale provider tool authority"));
            }
            if let Some(result) = self
                .try_dispatch_remote_meta_runtime_tool_call(
                    provider_run,
                    crate::transport::runtime_tools::REQUEST_POPUP_TOOL,
                    arguments.clone(),
                )
                .await?
            {
                return Ok(result);
            }
        }
        self.dispatch_popup_for_agent(
            provider_run.session_id(),
            provider_run
                .agent_instance_id()
                .ok_or_else(|| popup_error("provider run is not bound to an agent"))?,
            Some(provider_run),
            args,
        )
        .await
    }

    pub(in crate::runtime::state) async fn dispatch_popup_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run: Option<&crate::provider::RuntimeProviderRun>,
        args: crate::transport::runtime_tools::RequestPopupArgs,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        if args.timeout_sec == Some(0) {
            return Err(popup_error("timeout_sec must be at least 1"));
        }
        if let Some(key) = args.idempotency_key.as_deref() {
            if key.is_empty() {
                return Err(popup_error("idempotency_key must not be empty"));
            }
            if args.wait != Some(false) {
                return Err(popup_error("idempotency_key needs wait:false"));
            }
        }
        let asking_task = (args.wait == Some(false))
            .then(|| self.asking_task_for_turn(session_id, agent_id))
            .transpose()?;
        let keyed_id = if let (Some(task), Some(key)) =
            (asking_task.as_ref(), args.idempotency_key.as_deref())
        {
            let id = keyed_interaction_id(session_id, agent_id, &task.prompt_id, key);
            if let Some(wake) = self
                .owned
                .durable_state_store
                .agent_wake(&id)?
                .filter(|wake| {
                    wake.kind == super::super::interaction_wake::KIND
                        && wake.task_id == task.task_id
                        && wake.room_id == session_id
                        && wake.agent_id == agent_id
                })
            {
                return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "interaction_id": id,
                        "registration_id": wake.registration_id,
                        "deduplicated": true,
                    }),
                });
            }
            Some(id)
        } else {
            None
        };
        if args.action.is_some() {
            return Err(popup_error(
                "an action card must be raised by a local agent's own provider run",
            ));
        }
        if args.choices.is_empty() && args.view.is_some() {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_request_popup",
                message: "a projected browser card requires at least one choice".to_string(),
            });
        }
        if args
            .view
            .as_ref()
            .is_some_and(|view| view.browser_region.verification.is_some())
            && (args.wait != Some(false)
                || args.until.is_some()
                || args.custom_choice.is_some()
                || args.default_on_timeout.is_some()
                || args.choices.iter().any(|choice| {
                    choice.id != CANCEL_CHOICE_ID && !duplicates_cancel(&choice.id, &choice.label)
                }))
        {
            return Err(popup_error("verification cards require wait:false and only Cancel; no until, custom reply or default"));
        }
        let free_text_only = args.view.is_none() && args.choices.is_empty();
        let folded_cancel: Vec<String> = args
            .choices
            .iter()
            .filter(|choice| duplicates_cancel(&choice.id, &choice.label))
            .map(|choice| choice.id.clone())
            .collect();
        let mut choices = args
            .choices
            .into_iter()
            // Row 655: the card always carries its own Cancel; an agent choice that only repeats it is the same choice.
            .filter(|choice| !duplicates_cancel(&choice.id, &choice.label))
            .map(|choice| {
                crate::session::RuntimeInteractionChoice::new(
                    choice.id,
                    choice.label,
                    choice.reply,
                    choice.style,
                )
            })
            .collect::<Vec<_>>();
        add_person_done(args.view.as_ref(), &mut choices);
        if !free_text_only && choices.iter().all(|choice| choice.id() != CANCEL_CHOICE_ID) {
            choices.push(crate::session::RuntimeInteractionChoice::new(
                CANCEL_CHOICE_ID,
                "Cancel",
                CANCEL_CHOICE_ID,
                Some(crate::session::RuntimeInteractionChoiceStyle::Secondary),
            ));
        }
        let custom_choice = args
            .custom_choice
            .map(|choice| {
                crate::session::RuntimeInteractionCustomChoice::new(
                    choice.id,
                    choice.label,
                    choice.placeholder,
                    choice.min_length,
                    choice.max_length,
                )
            })
            .or_else(|| {
                args.view.is_none().then(|| {
                    let mut reply_id = "reply".to_owned();
                    while choices.iter().any(|choice| choice.id() == reply_id) {
                        reply_id.push('_');
                    }
                    crate::session::RuntimeInteractionCustomChoice::new(
                        reply_id,
                        "",
                        None,
                        Some(1),
                        None,
                    )
                })
            });
        if let Some(custom_choice) = custom_choice.as_ref() {
            if choices
                .iter()
                .any(|choice| choice.id() == custom_choice.id())
            {
                return Err(DaemonError::LocalTransport {
                    operation: "runtime_tool_request_popup",
                    message: format!(
                        "custom_choice id `{}` duplicates a fixed choice",
                        custom_choice.id()
                    ),
                });
            }
            if let Some(max_length) = custom_choice.max_length() {
                if max_length < custom_choice.min_length() {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_request_popup",
                        message:
                            "custom_choice max_length must be greater than or equal to min_length"
                                .to_string(),
                    });
                }
            }
        }
        let default_choice_id = args.default_on_timeout.clone().map(|id| {
            if folded_cancel.contains(&id) {
                CANCEL_CHOICE_ID.to_string()
            } else {
                id
            }
        });
        if let Some(default_choice_id) = default_choice_id.as_deref() {
            if !choices
                .iter()
                .any(|choice| choice.id() == default_choice_id)
            {
                return Err(DaemonError::LocalTransport {
                    operation: "runtime_tool_request_popup",
                    message: format!(
                        "default_on_timeout choice `{default_choice_id}` is not defined"
                    ),
                });
            }
        }
        let closing_wake = match args.until {
            Some(until) if args.wait != Some(false) => {
                return Err(popup_error(&format!(
                    "until `{}` needs wait:false: a blocking card cannot also follow a wake source",
                    until.wake_id
                )));
            }
            Some(until) => Some(self.card_condition_wake(session_id, agent_id, &until.wake_id)?),
            None => None,
        };
        let interaction_id =
            keyed_id.unwrap_or_else(|| format!("interaction-{:032x}", rand::random::<u128>()));
        let (view, anchor_hidden) = if let Some(view) = args.view.as_ref() {
            if let Some(run) = provider_run {
                self.user_domain_agent_authority(run)?;
            }
            let (view, hidden) = self
                .resolve_popup_view_seen(session_id, agent_id, &interaction_id, &view.browser_region)
                .await?;
            (Some(view), hidden)
        } else {
            (None, false)
        };
        // Row 655: the agent is told the truth when the element it named cannot be seen yet (a challenge frame
        // that opens only after a checkbox): it must not tell the person the step is open.
        let page_note = if anchor_hidden {
            Some(HIDDEN_REGION_NOTE)
        } else {
            hidden_region_note(view.as_ref())
        };
        let mut interaction = crate::session::RuntimeInteraction::new(
            interaction_id,
            agent_id,
            crate::session::RuntimeInteractionKind::Choice,
            args.level
                .unwrap_or(crate::session::RuntimeInteractionLevel::Info),
            args.title,
            args.message,
            choices,
            custom_choice,
            if args.wait == Some(false) {
                args.timeout_sec
            } else {
                Some(blocking_popup_timeout_seconds(args.timeout_sec))
            },
            default_choice_id.clone(),
        );
        if let Some(wake_id) = closing_wake {
            interaction = interaction.closing_with_wake(wake_id);
        }
        if let Some(view) = view {
            interaction = interaction.with_view(view);
        }
        // Row 655 (one card per Tab): a waiting card and a wait:false card alike replace the agent's open plain card on
        // their Tab (popupcard 11-26: a wait:false card followed by a waiting one left both open).
        let replaced = self.one_card_per_tab(session_id, agent_id, &interaction)?;
        if let Some(task) = asking_task {
            let interaction_id = interaction.id().to_owned();
            let mut result = self.arm_interaction_wake_for_task(&task, interaction)?;
            if let Some(replaced) = replaced {
                result.payload["replaced"] = replaced.into();
            }
            if let Some(note) = page_note {
                result.payload["page_note"] = note.into();
            }
            self.resume_browser_region_cards();
            // Its holder is told as for a question that blocks the turn.
            self.announce_child_decision(session_id, &interaction_id)
                .await?;
            if let Ok(session) = self.owned.session_store.get_session(session_id) {
                if let Some(ask) = session
                    .active_interactions()
                    .iter()
                    .find(|ask| ask.id() == interaction_id)
                {
                    self.owned.alert_owner_of_ask(session_id, ask);
                }
            }
            return Ok(result);
        }
        let provider_run =
            provider_run.ok_or_else(|| popup_error("forwarded popup must use wait:false"))?;
        let interaction = interaction.with_native_origin(
            self.owned
                .capture_native_interaction_origin(session_id, agent_id, provider_run.id()),
        );
        let interaction_id = interaction.id().to_string();
        let session_id = provider_run.session_id().to_string();
        let timeout_sec = interaction.timeout_sec();
        let remote_target = if let Some(origin) = interaction.native_origin() {
            self.remote_native_interaction_context(
                provider_run.session_id(),
                provider_run.agent_instance_id().unwrap_or(""),
                origin,
            )
            .await?
        } else {
            None
        };
        let interaction = if interaction.view().is_some() {
            interaction.with_card_type("popup_browser_region")
        } else {
            interaction.with_card_type("Question")
        };
        if let Some((config, target_daemon_id, context)) = remote_target {
            // Home owns the popup deadline and its resolution. Leave time
            // for that outcome to return; an indefinite card has no relay cap.
            let response_timeout = timeout_sec.map(|seconds| {
                        std::time::Duration::from_secs(seconds)
                            .saturating_add(crate::runtime::native_interaction_bridge::REMOTE_NATIVE_INTERACTION_RESPONSE_BUFFER)
                    });
            let response = crate::transport::relay_client::send_peer_request_via_temporary_connection_with_optional_timeout(
                        &config,
                        ClientTarget { daemon_id: Some(target_daemon_id), daemon_alias: None },
                        RelayPeerRequest::ForwardNativeTurnInteraction { context, interaction },
                        response_timeout,
                    ).await?;
            let resolution = match response {
                RelayPeerResponse::NativeInteractionResolved { resolution } => resolution,
                other => {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_request_popup",
                        message: format!(
                            "unexpected relay response for remote popup interaction: {other:?}"
                        ),
                    });
                }
            };
            return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "interaction_id": interaction_id,
                    "status": popup_status(&resolution.status, resolution.choice_id.as_deref()),
                    "choice_id": resolution.choice_id,
                    "reply": resolution.reply,
                }),
            });
        }
        let resolution_rx = self
            .create_runtime_interaction(
                if interaction.view().is_some() {
                    crate::runtime::state::InteractionSurface::named("popup_browser_region")
                } else {
                    crate::runtime::state::InteractionSurface::named("Question")
                },
                &session_id,
                interaction,
            )
            .await?;
        // The card is only answerable while this tool call waits: when the
        // provider gives up on the call, withdraw it instead of leaving a dead card.
        let _close = super::super::runtime_vault_unlock_state::CloseInteractionOnDrop {
            state: self.clone(),
            session_id: session_id.clone(),
            interaction_id: interaction_id.clone(),
        };
        self.resume_browser_region_cards();
        if let Some(timeout_sec) = timeout_sec {
            let state = self.clone();
            let timeout_session_id = session_id.clone();
            let timeout_interaction_id = interaction_id.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(timeout_sec)).await;
                let _ = state
                    .timeout_runtime_interaction(&timeout_session_id, &timeout_interaction_id)
                    .await;
            });
        }
        let resolution = resolution_rx
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "runtime_tool_request_popup",
                message: format!("popup interaction dropped before resolution: {error}"),
            })?;
        let mut payload = serde_json::json!({
            "interaction_id": interaction_id,
            "status": popup_status(resolution.status, resolution.choice_id.as_deref()),
            "choice_id": resolution.choice_id,
            "reply": resolution.reply,
        });
        if let Some(replaced) = replaced {
            payload["replaced"] = replaced.into();
        }
        Ok(crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload })
    }
}
