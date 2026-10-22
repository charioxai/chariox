//! MP-08 / MP-10 / MP-11 A07 (protocol 477): protected owner hand-off.
//!
//! An agent that must not or cannot perform one browser step (a model
//! refusal, a site that disallows automation, a human verification, an owner
//! authorization) asks the owner for exactly one bound action. The kernel
//! validates the observed target, registers a `hand_off` obligation and one
//! kernel-owned interaction on every terminal of the session. Only the owner's
//! Chariox terminal answers it; the first answer is claimed before any I/O.
//! Typed values go from that answer straight to the bound field under
//! observation protection. The agent learns only the safe outcome, through
//! its durable inbox, and must re-read the page to verify the result.
use super::kernel_browser_runtime::host_error;
use super::*;
use crate::local::{HandoffOutcome, HandoffResponseAction, HandoffStatus, RespondToHandoffRequest};
use crate::runtime::browser_controller_action::BrowserLocatorAction;
use crate::runtime::browser_controller_snapshot::BrowserControllerStructuredSnapshot;
use crate::runtime::kernel_browser_host::{KernelBrowserAdmission, KernelBrowserHost};
use crate::session::{
    bounded_display_text, HandoffChangeLine, HandoffKind, HandoffReason, HandoffTarget,
    RuntimeHandoff, RuntimeInteraction, RuntimeInteractionChoice,
};
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};
use serde_json::json;

pub(super) const HANDOFF: &str = "chariox.handoff.request";
pub(crate) const HANDOFF_OBLIGATION_KIND: &str = "hand_off";
const DEFAULT_TIMEOUT_SEC: u64 = 900;

pub(crate) fn is_handoff_interaction_id(id: &str) -> bool {
    id.starts_with("handoff-")
}

mod contract;
use contract::*;

mod browser;
mod claim;
mod lifecycle;

impl KernelRuntimeState {
    /// MP-11 A07: a pending or in-flight owner step reserves its tab against
    /// every model mutation. Observation and authenticated owner input stay live.
    pub(super) fn handoff_model_write_blocked(&self, user: &str, tab: Option<&str>) -> bool {
        let matches = |h: &RuntimeHandoff| tab.is_none_or(|tab| h.target.tab_id == tab);
        if self
            .owned
            .session_store
            .list_all_sessions()
            .iter()
            .any(|session| {
                self.provider_account_authority_owner_user_id(session.owner_user_id()) == user
                    && session
                        .active_interactions()
                        .iter()
                        .filter_map(RuntimeInteraction::handoff)
                        .any(&matches)
            })
        {
            return true;
        }
        let claims = self
            .owned
            .handoff_claims
            .lock()
            .map(|claims| claims.clone());
        let Ok(claims) = claims else {
            return true;
        };
        claims.iter().any(|id| {
            self.owned
                .durable_state_store
                .load_subject_events_by_kind(id, "handoff.claimed", 1)
                .ok()
                .and_then(|events| events.last().cloned())
                .is_none_or(|event| {
                    let Some(room) = event.payload["room"].as_str() else {
                        return true;
                    };
                    let Ok(session) = self.owned.session_store.get_session(room) else {
                        return true;
                    };
                    self.provider_account_authority_owner_user_id(session.owner_user_id()) == user
                        && serde_json::from_value::<RuntimeHandoff>(
                            event.payload["handoff"].clone(),
                        )
                        .map(|handoff| matches(&handoff))
                        .unwrap_or(true)
                })
        })
    }

    pub(super) fn owner_handoff_tool_spec(&self) -> Option<RuntimeToolSpec> {
        self.room_agent_tools_enabled().then(handoff_spec)
    }

    /// The agent side: validate the observed target and create the obligation
    /// plus one owner interaction. Returns only identifiers and expiry.
    pub(super) async fn kernel_browser_handoff(
        &self,
        agent: &crate::agent::AgentInstance,
        arguments: serde_json::Value,
        admission: KernelBrowserAdmission,
    ) -> Result<RuntimeToolResult, DaemonError> {
        if !self.room_agent_tools_enabled() {
            return Err(handoff_error("hand-off needs the room agent tools"));
        }
        let args: HandoffArgs = serde_json::from_value(arguments)
            .map_err(|_| handoff_error("invalid hand-off arguments"))?;
        let timeout_sec = args.timeout_sec.unwrap_or(DEFAULT_TIMEOUT_SEC);
        if !(crate::session::HANDOFF_MIN_TIMEOUT_SEC..=crate::session::HANDOFF_MAX_TIMEOUT_SEC)
            .contains(&timeout_sec)
        {
            return Err(handoff_error("hand-off timeout is out of range"));
        }
        if args.offer_vault_save && args.kind != HandoffKind::Secret {
            return Err(handoff_error("only a secret hand-off can offer Vault save"));
        }
        let explanation = bounded_display_text(
            &args.explanation,
            crate::session::HANDOFF_EXPLANATION_MAX_CHARS,
            true,
        );
        if explanation.is_empty() {
            return Err(handoff_error("hand-off explanation is required"));
        }
        let change = args
            .change
            .into_iter()
            .take(crate::session::HANDOFF_CHANGE_MAX_LINES)
            .map(|line| HandoffChangeLine {
                op: line.op,
                text: bounded_display_text(
                    &line.text,
                    crate::session::HANDOFF_CHANGE_LINE_MAX_CHARS,
                    false,
                ),
            })
            .collect::<Vec<_>>();
        let user = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        // MP-11 A07: bind the pending request to the original grant identity.
        // A later grant must not resurrect a request made before source loss.
        let source_grant_identity = self
            .owned
            .kernel_browser_host
            .grant_identity(&admission)
            .map_err(host_error)?;
        let result = self
            .kernel_browser_operation_admitted(
                &user,
                Some(admission),
                "host.browser",
                json!({"op":"snapshot","tab_id":args.tab_id,"generation":args.generation}),
            )
            .await?;
        let snapshot: BrowserControllerStructuredSnapshot =
            serde_json::from_value(result["snapshot"].clone())
                .map_err(|_| handoff_error("invalid browser snapshot"))?;
        snapshot
            .validate(&snapshot.target_id, &args.document_id)
            .map_err(|_| handoff_error("stale_reference: take a fresh snapshot"))?;
        let node = snapshot
            .dom_nodes
            .iter()
            .find(|node| node.node_ref == args.node_ref)
            .ok_or_else(|| handoff_error("stale_reference: rediscover the target element"))?;
        if !node_accepts(args.kind, node) {
            return Err(handoff_error(match args.kind {
                HandoffKind::Click => "click hand-off needs an enabled element",
                HandoffKind::Code => "code hand-off needs an editable text field",
                HandoffKind::Secret => "secret hand-off needs an editable password field",
            }));
        }
        let (origin, path) = origin_and_path(
            snapshot
                .document_url_for_node(&args.node_ref)
                .map_err(host_error)?,
        )?;
        let label = node_label(&snapshot, &args.node_ref);
        let session = self.owned.session_store.get_session(agent.session_id())?;
        let owner = session.owner_user_id().to_owned();
        let obligation = self
            .register_room_dispatch_obligation(agent, HANDOFF_OBLIGATION_KIND, None)?
            .ok_or_else(|| handoff_error("hand-off needs the room agent tools"))?;
        let task_id = self
            .owned
            .durable_state_store
            .agent_tasks(Some(agent.session_id()), Some(agent.id()))?
            .into_iter()
            .find(|t| t.obligations.iter().any(|o| o.id == obligation))
            .map(|t| t.task_id)
            .ok_or_else(|| handoff_error("hand-off task unavailable"))?;
        let id = RuntimeHandoff::interaction_id(&obligation);
        let handoff = RuntimeHandoff {
            kind: args.kind,
            reason: args.reason,
            agent_id: agent.id().to_owned(),
            task_id,
            obligation_id: obligation.clone(),
            explanation,
            target: HandoffTarget {
                tab_id: args.tab_id,
                generation: args.generation,
                document_id: args.document_id,
                node_ref: args.node_ref,
                origin,
                path,
                label,
            },
            change,
            expires_at_ms: crate::session::unix_epoch_ms() + timeout_sec * 1000,
            save_to_vault_offered: args.offer_vault_save,
        };
        // Retain the full-URL fence privately. URLs may contain sensitive
        // query/fragment values, so only safe origin/path enter the projection.
        let binding = self.owned.durable_state_store.append_event("handoff.binding", Some(id.clone()),
            json!({"room":agent.session_id(), "source_grant_identity":source_grant_identity,
                "document_url_digest":document_url_binding(snapshot.document_url_for_node(&handoff.target.node_ref).map_err(host_error)?)}));
        crate::runtime::room_dispatch_registration::reject_if_failed(
            &self.owned.durable_state_store,
            Some(&obligation),
            binding,
        )?;
        let registered = self
            .register_handoff_interaction(agent.session_id(), &owner, handoff.clone(), timeout_sec)
            .await;
        crate::runtime::room_dispatch_registration::reject_if_failed(
            &self.owned.durable_state_store,
            Some(&obligation),
            registered,
        )?;
        self.finish_room_dispatch(Some(&obligation), Some(&id), || {})?;
        Ok(RuntimeToolResult {
            ok: true,
            payload: json!({
                "handoff_id": id,
                "obligation_id": obligation,
                "registration_id": format!("completion-{obligation}"),
                "status": "pending",
                "expires_at_ms": handoff.expires_at_ms,
                "next": "Call chariox.events.yield with this registration_id and a deadline after expires_at_ms, then end the turn. After waking, re-read the page and verify the result.",
            }),
        })
    }

    /// One kernel-operation interaction carrying the hand-off; a waiter
    /// settles it as expired when nobody answered in time.
    async fn register_handoff_interaction(
        &self,
        session_id: &str,
        owner: &str,
        handoff: RuntimeHandoff,
        timeout_sec: u64,
    ) -> Result<(), DaemonError> {
        let id = RuntimeHandoff::interaction_id(&handoff.obligation_id);
        let action = match handoff.kind {
            HandoffKind::Click => format!("Click \u{201c}{}\u{201d}", handoff.target.label),
            HandoffKind::Code => {
                format!("Enter the code in \u{201c}{}\u{201d}", handoff.target.label)
            }
            HandoffKind::Secret => format!(
                "Enter the secret in \u{201c}{}\u{201d}",
                handoff.target.label
            ),
        };
        let mut message = format!(
            "{action} on {}{}.\n{}",
            handoff.target.origin, handoff.target.path, handoff.explanation
        );
        if !handoff.change.is_empty() {
            message.push_str("\nIntended change:\n");
            message.push_str(&change_text(&handoff.change));
        }
        message.push_str(if handoff.kind == HandoffKind::Click {
            "\nReview the scoped view and authorize the click, or use the live browser, then Done."
        } else {
            "\nUse Chariox protected entry to enter the value into this field."
        });
        let interaction = RuntimeInteraction::for_kernel_operation(
            &id,
            &id,
            "Agent hand-off",
            message,
            if handoff.kind == HandoffKind::Click {
                vec![
                    RuntimeInteractionChoice::new("done", "Done in browser", "done", None),
                    RuntimeInteractionChoice::new("cancel", "Cancel", "cancel", None),
                ]
            } else {
                vec![RuntimeInteractionChoice::new(
                    "cancel", "Cancel", "cancel", None,
                )]
            },
        )
        .with_timeout_sec(timeout_sec)
        .with_handoff(handoff.clone());
        let receiver = self
            .create_kernel_operation_interaction(session_id, owner, interaction)
            .await?;
        let state = self.clone();
        let room = session_id.to_owned();
        tokio::spawn(async move {
            // A claimed answer drops the responder; only a kernel timeout or
            // withdrawal resolves it.
            if let Ok(resolution) = receiver.await {
                let reason = (resolution.status == "timed_out").then_some("timeout");
                let result = outcome(
                    &RuntimeHandoff::interaction_id(&handoff.obligation_id),
                    HandoffStatus::Expired,
                    "timeout",
                    reason,
                );
                state.finish_handoff(&room, &handoff, &result, "kernel");
            }
        });
        Ok(())
    }

    /// The owner side, from an authenticated Chariox terminal.
    pub(crate) async fn respond_to_handoff(
        &self,
        command: &crate::runtime::command::KernelCommand,
        request: RespondToHandoffRequest,
    ) -> Result<HandoffOutcome, DaemonError> {
        if !command.is_terminal_caller() {
            return Err(handoff_error(
                "Only a Chariox terminal can answer a hand-off",
            ));
        }
        let caller = crate::runtime::command::command_caller_user_id(command);
        let RespondToHandoffRequest {
            session_id,
            interaction_id,
            action,
        } = request;
        let browser = match &action {
            HandoffResponseAction::Click | HandoffResponseAction::EnterValue { .. } => {
                let (user, actor) = self.kernel_browser_terminal_context(command)?;
                let admission = self
                    .owned
                    .kernel_browser_host
                    .admit_terminal(&user, command.terminal_lifetime.clone().unwrap_or_default());
                Some((user, actor, admission))
            }
            _ => None,
        };
        let session_owner = self
            .owned
            .session_store
            .get_session(&session_id)?
            .owner_user_id()
            .to_owned();
        let session_browser_user = self.provider_account_authority_owner_user_id(&session_owner);
        let (handoff, _claim) =
            self.owned
                .claim_handoff(&session_id, &interaction_id, &caller, |handoff| {
                    match &action {
                        HandoffResponseAction::Done if handoff.kind.accepts_value() => {
                            Err(handoff_error("use protected entry for codes and secrets"))
                        }
                        HandoffResponseAction::Click if handoff.kind != HandoffKind::Click => {
                            Err(handoff_error("this hand-off does not ask for a click"))
                        }
                        HandoffResponseAction::EnterValue {
                            value,
                            save_to_vault_key,
                        } => {
                            if !handoff.kind.accepts_value() {
                                return Err(handoff_error(
                                    "this hand-off does not ask for a value",
                                ));
                            }
                            if value.expose_secret().is_empty()
                                || value.expose_secret().len() > 4_096
                            {
                                return Err(handoff_error("value must be 1 to 4096 bytes"));
                            }
                            if save_to_vault_key.is_some() && !handoff.save_to_vault_offered {
                                return Err(handoff_error(
                                    "this hand-off does not offer Vault save",
                                ));
                            }
                            Ok(())
                        }
                        _ => Ok(()),
                    }?;
                    if browser
                        .as_ref()
                        .is_some_and(|(user, ..)| *user != session_browser_user)
                    {
                        return Err(handoff_error(
                            "the hand-off browser belongs to the session owner",
                        ));
                    }
                    Ok(())
                })?;
        let kind = action.kind();
        let mut result = match (&action, browser) {
            (HandoffResponseAction::Cancel, _) => outcome(
                &interaction_id,
                HandoffStatus::Cancelled,
                kind,
                Some("owner_cancelled"),
            ),
            (HandoffResponseAction::Done, _) => {
                outcome(&interaction_id, HandoffStatus::Completed, kind, None)
            }
            (_, Some((user, actor, admission))) => {
                self.handoff_browser_action(
                    &user,
                    &actor,
                    admission,
                    &handoff,
                    &interaction_id,
                    &action,
                )
                .await
            }
            _ => outcome(
                &interaction_id,
                HandoffStatus::Failed,
                kind,
                Some("browser_unavailable"),
            ),
        };
        if let HandoffResponseAction::EnterValue {
            value,
            save_to_vault_key: Some(key),
        } = &action
        {
            if result.status == HandoffStatus::Completed {
                result.saved_to_vault = self
                    .save_handoff_secret(&handoff, key, value.expose_secret())
                    .await
                    .inspect_err(|error| {
                        tracing::warn!(error=%crate::secret_redaction::redact_secrets(&error.to_string()), "MP-11 A07: hand-off Vault save failed");
                    })
                    .is_ok();
                if !result.saved_to_vault {
                    result.reason_code = Some("vault_save_failed".into());
                }
            }
        }
        self.finish_handoff(&session_id, &handoff, &result, &caller);
        Ok(result)
    }

    /// Choices `done`/`cancel` from a client without protected entry.
    pub(super) fn answer_handoff_choice(
        &self,
        session_id: &str,
        interaction_id: &str,
        choice_id: &str,
        caller_user_id: Option<&str>,
    ) -> Result<(), DaemonError> {
        let action = match choice_id {
            "done" => HandoffResponseAction::Done,
            "cancel" => HandoffResponseAction::Cancel,
            _ => {
                return Err(handoff_error(
                    "use protected hand-off entry for this action",
                ))
            }
        };
        let caller =
            caller_user_id.ok_or_else(|| handoff_error("Only the hand-off owner can answer it"))?;
        let (handoff, _claim) =
            self.owned
                .claim_handoff(session_id, interaction_id, caller, |h| {
                    if choice_id == "done" && h.kind.accepts_value() {
                        Err(handoff_error("use protected entry for codes and secrets"))
                    } else {
                        Ok(())
                    }
                })?;
        let result = match action {
            HandoffResponseAction::Done => {
                outcome(interaction_id, HandoffStatus::Completed, "done", None)
            }
            _ => outcome(
                interaction_id,
                HandoffStatus::Cancelled,
                "cancel",
                Some("owner_cancelled"),
            ),
        };
        self.finish_handoff(session_id, &handoff, &result, caller);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
