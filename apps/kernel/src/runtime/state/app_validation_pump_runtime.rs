//! Shows each pending App validation as a trusted kernel approval (outside App
//! content and the agent-operated Environment) and records the human decision.
//! An App can only request; neither App code, an App view click nor an agent
//! reply can answer a kernel-operation approval.
use super::KernelRuntimeState;
use crate::durable_state::app_validations::{ValidationCommand, ValidationOperation};
use crate::session::{RuntimeInteraction, RuntimeInteractionChoice};

/// Installations considered per pass; each shows its oldest pending operation.
const PAGE: usize = 64;

impl KernelRuntimeState {
    pub(crate) fn schedule_app_validation_pump(&self) {
        let now_ms = crate::session::unix_epoch_ms();
        if self
            .owned
            .durable_state_store
            .require_writer_healthy()
            .is_err()
        {
            return;
        }
        let Some(pass) = self.app_control().validation_pump().try_begin(now_ms) else {
            return;
        };
        let runtime = self.clone();
        tokio::spawn(async move {
            let _pass = pass;
            runtime.app_validation_pass(now_ms).await;
            runtime.app_file_pick_pass(now_ms).await;
            runtime.app_file_export_pass(now_ms).await;
        });
    }

    async fn app_validation_pass(&self, now_ms: u64) {
        let store = self.owned.durable_state_store.clone();
        let Ok(Ok(pending)) = tokio::task::spawn_blocking(move || {
            let _ = store.app_validation(ValidationCommand::Expire { now_ms });
            store.pending_app_validations(PAGE)
        })
        .await
        else {
            return;
        };
        for operation in pending {
            if !self
                .app_control()
                .begin_validation_prompt(&operation.operation_id, &operation.owner)
            {
                continue;
            }
            let Some(session) = self.validation_session(&operation.owner) else {
                // No session to show it in yet; it stays pending until one exists.
                self.app_control()
                    .end_validation_prompt(&operation.operation_id);
                continue;
            };
            // The prompt closes no later than the operation expires.
            let remaining_sec = operation.expires_ms.saturating_sub(now_ms) / 1000;
            let requested_by = requester(&operation.callers, &operation.owner, |agent_id| {
                let agent = self.owned.agent_store.get_agent(agent_id).ok()?;
                Some(agent.alias().unwrap_or(agent.agent_ref()).to_owned())
            });
            let interaction = validation_interaction(&operation, &requested_by)
                .with_timeout_sec(remaining_sec.clamp(1, 300));
            let receiver = match self
                .create_kernel_operation_interaction(&session, &operation.owner, interaction)
                .await
            {
                Ok(receiver) => receiver,
                Err(_) => {
                    self.app_control()
                        .end_validation_prompt(&operation.operation_id);
                    continue;
                }
            };
            let runtime = self.clone();
            tokio::spawn(async move {
                let decision = receiver.await.ok().and_then(|resolution| {
                    match resolution.choice_id.as_deref() {
                        Some("approve") if resolution.status == "answered" => Some(true),
                        Some("deny") if resolution.status == "answered" => Some(false),
                        _ => None,
                    }
                });
                if let Some(approved) = decision {
                    let store = runtime.owned.durable_state_store.clone();
                    let operation_id = operation.operation_id.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        store.app_validation(ValidationCommand::Decide {
                            operation_id,
                            approved,
                            now_ms: crate::session::unix_epoch_ms(),
                        })
                    })
                    .await;
                }
                // Undecided (timed out or abandoned): shown again on a later pass
                // while the operation is still pending.
                runtime
                    .app_control()
                    .end_validation_prompt(&operation.operation_id);
            });
        }
    }

    /// The owner's most recently used session hosts the approval, so it
    /// appears on the terminals the person is using: preferably one the owner
    /// hosts. Collaborators in that session see it; only the owner answers.
    pub(super) fn validation_session(&self, owner: &str) -> Option<String> {
        self.owned
            .session_store
            .list_sessions()
            .into_iter()
            .filter(|session| session.has_member(owner))
            .max_by_key(|session| {
                (
                    session.owner_user_id() == owner,
                    session
                        .last_prompt_sent_at_ms()
                        .unwrap_or(session.created_at_ms()),
                )
            })
            .map(|session| session.id().to_owned())
    }
}

/// Who the App was working for when it asked, from the callers the kernel
/// recorded with the request (never App-supplied). The App cannot tell which
/// of several concurrent calls asked, so all of them are named.
fn requester(callers: &str, owner: &str, agent_label: impl Fn(&str) -> Option<String>) -> String {
    // An operation recorded before callers were has none to name.
    let Ok(callers) = serde_json::from_str::<Vec<serde_json::Value>>(callers) else {
        return "Requested by: not recorded.".to_owned();
    };
    let named: Vec<String> = callers
        .iter()
        .filter_map(|actor| {
            let id = actor["id"].as_str()?;
            Some(match actor["kind"].as_str()? {
                "agent" => match agent_label(id) {
                    Some(label) => format!("agent {id} ({label}), through its App tools"),
                    None => format!("agent {id}, through its App tools"),
                },
                "human" if id == owner => format!("you ({id}), in the App's view"),
                "human" => format!("{id}, in the App's view"),
                "background" => match id.strip_prefix("inbox:") {
                    Some(route) => format!("background work: an event on inbox route {route}"),
                    None if id == "schedule" => "background work: the App's scheduled wake".into(),
                    None => format!("background work ({id})"),
                },
                _ => return None,
            })
        })
        .collect();
    let view = callers.iter().any(|actor| actor["kind"] == "human");
    let mut text = match named.as_slice() {
        [] => {
            "Requested by: the App, with no caller named (its own or background work).".to_owned()
        }
        [one] => format!("Requested by: {one}."),
        many => format!(
            "Requested while the App was handling calls from: {}.",
            many.join("; ")
        ),
    };
    if view {
        text.push_str(" An agent that operates the App's view in the Room is shown the same way.");
    }
    text
}

fn validation_interaction(
    operation: &ValidationOperation,
    requested_by: &str,
) -> RuntimeInteraction {
    RuntimeInteraction::for_kernel_operation(
        format!("app_validation_{}", operation.operation_id),
        format!("validation:{}", operation.operation_id),
        "Approve App action",
        format!(
            "An App asks to perform a protected action.\n\nInstallation: {}\nAction: {}\nParameters: {}\n{requested_by}\n\nOnly {} (the App's owner) can answer. Approve only if you expect this exact action with these exact parameters.",
            operation.installation, operation.action, operation.parameters, operation.owner
        ),
        vec![
            RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
            RuntimeInteractionChoice::new("approve", "Approve", "allow", None),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::requester;

    fn named(callers: &str) -> String {
        requester(callers, "local", |agent| {
            (agent == "agent-1").then(|| "fresh-todo".into())
        })
    }

    #[test]
    fn the_approval_names_whom_the_app_was_working_for() {
        assert_eq!(
            named(r#"[{"kind":"agent","id":"agent-1"}]"#),
            "Requested by: agent agent-1 (fresh-todo), through its App tools."
        );
        assert_eq!(
            named(r#"[{"kind":"agent","id":"agent-9"}]"#),
            "Requested by: agent agent-9, through its App tools."
        );
        assert_eq!(
            named(r#"[{"kind":"human","id":"local"}]"#),
            "Requested by: you (local), in the App's view. An agent that operates the App's view in the Room is shown the same way."
        );
        assert_eq!(
            named(r#"[{"kind":"background","id":"inbox:steps"}]"#),
            "Requested by: background work: an event on inbox route steps."
        );
        assert_eq!(
            named(r#"[{"kind":"background","id":"schedule"}]"#),
            "Requested by: background work: the App's scheduled wake."
        );
        assert_eq!(
            named(r#"[{"kind":"agent","id":"agent-1"},{"kind":"background","id":"inbox:steps"}]"#),
            "Requested while the App was handling calls from: agent agent-1 (fresh-todo), through its App tools; background work: an event on inbox route steps."
        );
        // No named call in progress, or an operation recorded before callers were.
        assert_eq!(
            named("[]"),
            "Requested by: the App, with no caller named (its own or background work)."
        );
        assert_eq!(named(""), "Requested by: not recorded.");
    }
}
