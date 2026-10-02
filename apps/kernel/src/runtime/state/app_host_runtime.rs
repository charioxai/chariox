//! Human-only clipboard/link offers projected through the file-export prompt
//! slots. Taking an offer only returns its payload to the accepting terminal;
//! the kernel never touches an OS clipboard or opens a URL.
use super::KernelRuntimeState;
use crate::{
    durable_state::app_host_actions::{HostActionCommand, HostOffer},
    local::{AcceptAppHostActionRequest, AppHostAction, AppRequestErrorCode, LocalDaemonResponse},
    session::{RuntimeInteraction, RuntimeInteractionChoice},
};

impl KernelRuntimeState {
    pub(in crate::runtime) async fn app_host_action_pass(&self, now_ms: u64) {
        let store = self.owned.durable_state_store.clone();
        let Ok(Ok(pending)) = tokio::task::spawn_blocking(move || {
            store.app_host_action(HostActionCommand::Expire { now_ms })?;
            store.pending_app_host_actions(64)
        })
        .await
        else {
            return;
        };
        for offer in pending {
            if !self
                .app_control()
                .begin_validation_prompt(&offer.operation_id, &offer.owner)
            {
                continue;
            }
            let Some(session) = self.validation_session(&offer.owner) else {
                self.app_control()
                    .end_validation_prompt(&offer.operation_id);
                continue;
            };
            self.app_control()
                .show_validation_prompt(&offer.operation_id, &session);
            let interaction = host_interaction(&offer)
                .with_timeout_sec((offer.expires_ms.saturating_sub(now_ms) / 1000).clamp(1, 300));
            let receiver = match self
                .create_kernel_operation_interaction(&session, &offer.owner, interaction)
                .await
            {
                Ok(receiver) => receiver,
                Err(_) => {
                    self.app_control()
                        .end_validation_prompt(&offer.operation_id);
                    continue;
                }
            };
            let runtime = self.clone();
            tokio::spawn(async move {
                let resolution = receiver.await.ok();
                if resolution.as_ref().is_some_and(|reply| {
                    reply.status == "answered"
                        && reply.choice_id.as_deref() == Some("accept_host_action")
                }) {
                    // The take path keeps the prompt slot until its durable
                    // transaction finishes, preventing a duplicate projection.
                    return;
                }
                if resolution.is_some_and(|reply| {
                    reply.status == "answered" && reply.choice_id.as_deref() == Some("decline")
                }) {
                    let store = runtime.owned.durable_state_store.clone();
                    let operation_id = offer.operation_id.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        store.app_host_action(HostActionCommand::Decline {
                            owner: offer.owner,
                            operation_id,
                            now_ms: crate::session::unix_epoch_ms(),
                        })
                    })
                    .await;
                }
                runtime
                    .app_control()
                    .end_validation_prompt(&offer.operation_id);
            });
        }
    }

    pub(crate) async fn accept_app_host_action(
        &self,
        owner: String,
        request: AcceptAppHostActionRequest,
    ) -> LocalDaemonResponse {
        let failed = |code| LocalDaemonResponse::AppRequestFailed { code };
        if request.operation_id.len() > 128 || request.session_id.len() > 128 {
            return failed(AppRequestErrorCode::InvalidRequest);
        }
        let Ok(permit) = self.app_control().try_admit() else {
            return failed(AppRequestErrorCode::Busy);
        };
        // This arbitrates acceptance against decline/expiry, with owner and
        // session validation, before any payload is released to a client.
        if self
            .owned
            .take_app_host_interaction(&request.session_id, &request.operation_id, &owner)
            .is_err()
        {
            return failed(AppRequestErrorCode::NotFound);
        }
        let store = self.owned.durable_state_store.clone();
        let operation_id = request.operation_id.clone();
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            store.app_host_action(HostActionCommand::Accept {
                owner,
                operation_id,
                now_ms: crate::session::unix_epoch_ms(),
            })
        })
        .await;
        self.app_control()
            .end_validation_prompt(&request.operation_id);
        match result {
            Ok(Ok(Some(offer))) => LocalDaemonResponse::AppHostActionAccepted {
                operation_id: offer.operation_id,
                action: offer.action,
            },
            Ok(Err("NOT_FOUND")) => failed(AppRequestErrorCode::NotFound),
            _ => failed(AppRequestErrorCode::StorageUnavailable),
        }
    }
}

fn host_interaction(offer: &HostOffer) -> RuntimeInteraction {
    let (title, detail) = match &offer.action {
        // Quoting and escaping keep control/bidi/format characters visible and prevent
        // terminal escape injection while retaining the exact offered text.
        AppHostAction::ClipboardWrite { text } => (
            "Copy text from an App",
            format!("Text ({} UTF-8 bytes): {}", text.len(), visible_text(text)),
        ),
        AppHostAction::OpenLink { url } => ("Open a link from an App", format!("Exact URL: {url}")),
    };
    RuntimeInteraction::for_kernel_operation(
        format!("app_host_{}", offer.operation_id), format!("host_action:{}", offer.operation_id), title,
        format!("An App asks you to accept this action in your terminal.\n\nInstallation: {}\n{detail}\n\nOnly {} (the App's owner) can answer. Accept in this terminal, or decline. In a text terminal: /app host accept {}\nNo clipboard contents are read. The action is taken once.", offer.installation, offer.owner, offer.operation_id),
        vec![RuntimeInteractionChoice::new("decline", "Decline", "deny", None)],
    )
}

fn visible_text(text: &str) -> String {
    // escape_debug also renders invisible Unicode format characters as escapes.
    format!("\"{}\"", text.escape_debug())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_prompt_shows_exact_url_and_safe_text_with_only_decline() {
        let mut offer = HostOffer {
            operation_id: "op".into(),
            owner: "alice".into(),
            installation: "app".into(),
            generation: 1,
            action: AppHostAction::OpenLink {
                url: "https://example.org/a?x=%20#y".into(),
            },
            expires_ms: 1,
        };
        let wire = serde_json::to_value(host_interaction(&offer)).unwrap();
        assert_eq!(wire["kernel_operation_id"], "host_action:op");
        assert!(wire["message"]
            .as_str()
            .unwrap()
            .contains("Exact URL: https://example.org/a?x=%20#y"));
        assert!(wire["message"]
            .as_str()
            .unwrap()
            .contains("/app host accept op"));
        assert_eq!(wire["choices"].as_array().unwrap().len(), 1);
        assert!(wire.get("default_on_timeout").is_none());
        offer.action = AppHostAction::ClipboardWrite {
            text: "\u{1b}]52;c;evil\n\u{202e}".into(),
        };
        let prompt = host_interaction(&offer);
        assert!(!prompt.message().contains('\u{1b}'));
        assert!(!prompt.message().contains('\u{202e}'));
        assert!(prompt.message().contains("\\n"));
    }
}
