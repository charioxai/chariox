//! App views: the installation's signed UI runs as a managed Tab in the
//! session's Room browser, so people and agents share one DOM and profile.
//! `window.chariox.call(tool, input)` runs the App's own tool as the human
//! owner through the same catalog, validation and durable path as agent calls.
use super::KernelRuntimeState;
use crate::{
    local::{AppRequestErrorCode, LocalDaemonRequest, LocalDaemonResponse},
    runtime::{
        app_operation_budget::AppOperationBudget,
        app_views::AppViewBinding,
        browser_controller_app_view::{
            BrowserAppViewAsset, BrowserAppViewCall, BrowserAppViewCalls, BrowserAppViewError,
            BrowserAppViewOpened, BrowserAppViewRequest,
        },
        command::KernelCommand,
    },
    transport::room_browser_controller::{
        RoomBrowserControllerCommand as Command, RoomBrowserControllerResult as Response,
    },
};
use base64::Engine;
use chariox_app_runtime::app_catalog::{Actor, CallerContext};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_millis(250);
/// Consecutive failed polls before the session's views are dropped (for
/// example after the Room environment went away).
const MAX_POLL_FAILURES: u32 = 20;

impl KernelRuntimeState {
    pub(crate) async fn execute_app_view_request(
        &self,
        command: &KernelCommand,
        request: &LocalDaemonRequest,
    ) -> Option<LocalDaemonResponse> {
        let LocalDaemonRequest::OpenAppView(request) = request else {
            return None;
        };
        let owner = match crate::runtime::app_control::owner(command) {
            Ok(owner) => owner,
            Err(code) => return Some(failed(code)),
        };
        Some(
            self.open_app_view(owner, &request.session_id, &request.installation_id)
                .await
                .unwrap_or_else(failed),
        )
    }

    async fn open_app_view(
        &self,
        owner: String,
        session_id: &str,
        installation: &str,
    ) -> Result<LocalDaemonResponse, AppRequestErrorCode> {
        let store = self.owned.durable_state_store.clone();
        let (view_owner, view_installation) = (owner.clone(), installation.to_owned());
        let view = tokio::task::spawn_blocking(move || {
            store.app_view_assets(&view_owner, &view_installation)
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
        .map_err(|code| {
            tracing::warn!(code, "App view assets unavailable");
            match code {
                "app_view_too_large" => AppRequestErrorCode::LimitExceeded,
                _ => AppRequestErrorCode::NotFound,
            }
        })?;
        let engine = base64::engine::general_purpose::STANDARD;
        let request = BrowserAppViewRequest::Open {
            origin_label: origin_label(&owner, installation),
            installation_id: installation.to_owned(),
            entry: view.entry,
            assets: view
                .assets
                .into_iter()
                .map(|asset| BrowserAppViewAsset {
                    path: asset.path,
                    content_type: asset.content_type.to_owned(),
                    body_base64: engine.encode(asset.bytes),
                })
                .collect(),
        };
        let opened: BrowserAppViewOpened = self
            .app_view_command(session_id, request)
            .await
            .ok_or(AppRequestErrorCode::Conflict)?;
        let views = self.app_control().views().clone();
        if views.register(
            session_id,
            &opened.target_id,
            AppViewBinding {
                owner,
                installation: installation.to_owned(),
            },
        ) {
            let state = self.clone();
            let session = session_id.to_owned();
            tokio::spawn(async move { state.pump_app_view_calls(session).await });
        }
        // Project the new Tab into the Room so viewers and agents see it.
        let _ = self
            .reconcile_browser_controller_environment(session_id)
            .await;
        Ok(LocalDaemonResponse::AppViewOpened {
            installation_id: installation.to_owned(),
            target_id: opened.target_id,
            origin: opened.origin,
        })
    }

    async fn app_view_command<T: serde::de::DeserializeOwned>(
        &self,
        session_id: &str,
        request: BrowserAppViewRequest,
    ) -> Option<T> {
        match self
            .room_browser_controller_command(session_id, Command::AppView { request })
            .await
        {
            Ok(Response::AppView {
                result: Some(value),
            }) => serde_json::from_value(value).ok(),
            Ok(_) => None,
            Err(error) => {
                tracing::debug!(%error, "App view controller command failed");
                None
            }
        }
    }

    async fn pump_app_view_calls(self, session_id: String) {
        let views = self.app_control().views().clone();
        let mut failures = 0;
        while views.keep_pumping(&session_id) {
            tokio::time::sleep(POLL_INTERVAL).await;
            let polled_up_to = views.registrations(&session_id);
            let Some(batch) = self
                .app_view_command::<BrowserAppViewCalls>(&session_id, BrowserAppViewRequest::Calls)
                .await
            else {
                failures += 1;
                if failures >= MAX_POLL_FAILURES {
                    views.forget_session(&session_id);
                }
                continue;
            };
            failures = 0;
            if let Some(open) = &batch.open_targets {
                views.retain_open(&session_id, open, polled_up_to);
            }
            for call in batch.calls {
                let state = self.clone();
                let session = session_id.clone();
                tokio::spawn(async move { state.answer_app_view_call(session, call).await });
            }
        }
    }

    async fn answer_app_view_call(self, session_id: String, call: BrowserAppViewCall) {
        let views = self.app_control().views().clone();
        let outcome = match views.binding(&session_id, &call.target_id) {
            Some(binding) if binding.installation == call.installation_id => {
                self.invoke_app_view_tool(&session_id, &binding, &call.method, call.params)
                    .await
            }
            _ => Err(view_error(
                "APP_VIEW_UNBOUND",
                "This view is not bound to an App",
            )),
        };
        let (result, error) = match outcome {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error)),
        };
        let delivered = self
            .app_view_command::<Value>(
                &session_id,
                BrowserAppViewRequest::Respond {
                    target_id: call.target_id.clone(),
                    call_id: call.call_id,
                    result,
                    error,
                },
            )
            .await;
        if delivered.is_none() {
            // The Tab closed or navigated away from the controller's App tabs.
            views.remove(&session_id, &call.target_id);
        }
    }

    async fn invoke_app_view_tool(
        &self,
        session_id: &str,
        binding: &AppViewBinding,
        tool: &str,
        input: Value,
    ) -> Result<Value, BrowserAppViewError> {
        let unavailable = || view_error("APP_UNAVAILABLE", "The App is not running");
        let lease = self
            .app_lease_on_demand(&binding.owner, &binding.installation)
            .await
            .map_err(|_| unavailable())?;
        let slot = lease
            .reserve_call(Duration::from_secs(30))
            .map_err(|error| view_error("APP_BUSY", &error.to_string()))?;
        slot.validate_input(tool, &input)
            .map_err(|error| view_error("INVALID_INPUT", &error.to_string()))?;
        let permit = self.app_control().try_admit().map_err(|_| unavailable())?;
        let store = self.owned.durable_state_store.clone();
        let caller = view_caller(binding, session_id);
        let tool_name = tool.to_owned();
        let response = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            store.enqueue_app_tool(slot, &tool_name, input, caller, budget())
        })
        .await
        .map_err(|_| unavailable())?
        .map_err(|error| view_error("APP_ERROR", &error.to_string()))?;
        let reply = response
            .receive()
            .await
            .map_err(|error| view_error("APP_ERROR", &error.to_string()))?;
        let permit = self.app_control().try_admit().map_err(|_| unavailable())?;
        let store = self.owned.durable_state_store.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            store.accept_app_tool_reply(reply)
        })
        .await
        .map_err(|_| unavailable())?
        .map_err(|error| view_error("APP_ERROR", &error.to_string()))
    }
}

/// A view call runs as the view's owner, whoever drives the Tab: the view is
/// the owner's human surface, and a person or an agent operating the shared
/// Room browser acts through it with the owner's App authority (V-SDK-04: no
/// separate view privilege). Critical effects still need the kernel's human
/// validation, which a view click cannot provide.
fn view_caller(binding: &AppViewBinding, session_id: &str) -> CallerContext {
    CallerContext {
        actor: Actor::Human(binding.owner.clone()),
        room_id: session_id.into(),
        operation_id: format!("app-operation-{:016x}", rand::random::<u64>()),
        task_id: None,
        turn_id: None,
    }
}

/// One origin per owner and installation keeps each App's web storage apart.
fn origin_label(owner: &str, installation: &str) -> String {
    let digest = Sha256::digest(format!("{owner}\0{installation}"));
    format!("a{}", &hex_prefix(&digest)[..24])
}

fn hex_prefix(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn view_error(code: &str, message: &str) -> BrowserAppViewError {
    BrowserAppViewError {
        code: code.to_owned(),
        message: message.to_owned(),
    }
}

fn budget() -> AppOperationBudget {
    AppOperationBudget::from_supervisor(|| false)
}

fn failed(code: AppRequestErrorCode) -> LocalDaemonResponse {
    crate::runtime::app_control::failed(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_calls_run_as_the_views_owner_in_its_room() {
        let binding = AppViewBinding {
            owner: "alice".into(),
            installation: "todo".into(),
        };
        let caller = view_caller(&binding, "session-1");
        assert!(matches!(&caller.actor, Actor::Human(owner) if owner == "alice"));
        assert_eq!(caller.room_id, "session-1");
        assert!(caller.task_id.is_none() && caller.turn_id.is_none());
    }

    #[test]
    fn origin_labels_are_stable_dns_labels_per_owner_and_installation() {
        let label = origin_label("user", "app-1");
        assert_eq!(label.len(), 25);
        assert!(label.starts_with('a'));
        assert!(label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()));
        assert_eq!(label, origin_label("user", "app-1"));
        assert_ne!(label, origin_label("other", "app-1"));
        assert_ne!(label, origin_label("user", "app-2"));
    }
}
