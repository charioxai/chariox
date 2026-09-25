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
const COMMAND_RETRY: Duration = Duration::from_millis(100);
/// An Open waits for a running Room command (bounded by the controller's
/// 15 s command timeout); an answer is retried a few times.
const OPEN_WAIT: Duration = Duration::from_secs(16);
const RESPOND_ATTEMPTS: u32 = 5;
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
                owner: owner.clone(),
                installation: installation.to_owned(),
                generation: view.generation,
            },
        ) {
            let state = self.clone();
            let session = session_id.to_owned();
            tokio::spawn(async move { state.pump_app_view_calls(session).await });
        }
        // An uninstall commits, then unbinds the installation's views. Checking
        // only after registering means an Open racing it ends unbound either way.
        let store = self.owned.durable_state_store.clone();
        let (check_owner, check_installation) = (owner.clone(), installation.to_owned());
        let active = tokio::task::spawn_blocking(move || {
            store
                .active_app_release(&check_owner, &check_installation)
                .is_ok()
        })
        .await
        .unwrap_or(false);
        if !active {
            views.forget_installation(&owner, installation);
            return Err(AppRequestErrorCode::NotFound);
        }
        // Project the new Tab into the Room so viewers and agents see it.
        let _ = self
            .reconcile_browser_controller_environment(session_id)
            .await;
        let bound_agent_id = self.foreground_app(session_id, &owner, installation).await;
        Ok(LocalDaemonResponse::AppViewOpened {
            installation_id: installation.to_owned(),
            target_id: opened.target_id,
            origin: opened.origin,
            bound_agent_id,
        })
    }

    /// A view command either runs or is final, with two exceptions. An Open
    /// takes the slice's operation slot, so it waits while another Room
    /// command holds it (that rejection means it never ran; an Open is not
    /// idempotent). An answer is idempotent, so any failure is retried.
    async fn app_view_command<T: serde::de::DeserializeOwned>(
        &self,
        session_id: &str,
        request: BrowserAppViewRequest,
    ) -> Option<T> {
        let open = matches!(request, BrowserAppViewRequest::Open { .. });
        let respond = matches!(request, BrowserAppViewRequest::Respond { .. });
        let deadline = tokio::time::Instant::now() + OPEN_WAIT;
        let mut attempt = 0;
        loop {
            attempt += 1;
            match self
                .room_browser_controller_command(
                    session_id,
                    Command::AppView {
                        request: request.clone(),
                    },
                )
                .await
            {
                Ok(Response::AppView {
                    result: Some(value),
                }) => return serde_json::from_value(value).ok(),
                Ok(_) => return None,
                Err(error)
                    if (open
                        && tokio::time::Instant::now() < deadline
                        && error.to_string().contains("already has an active"))
                        || (respond && attempt < RESPOND_ATTEMPTS) =>
                {
                    tokio::time::sleep(COMMAND_RETRY).await;
                }
                Err(error) => {
                    tracing::debug!(%error, "App view controller command failed");
                    return None;
                }
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
                views.set_open_tabs(&session_id, open.len());
            }
            if let Some(panels) = &batch.panels {
                self.publish_app_tabs(&session_id, &views, panels).await;
            }
            for call in batch.calls {
                let state = self.clone();
                let session = session_id.clone();
                tokio::spawn(async move { state.answer_app_view_call(session, call).await });
            }
        }
    }

    /// Marks the Room's App view Tabs and their reserved panels. A panel is in
    /// desktop pixels (the App's window is fullscreen) and names the focus
    /// agent whose conversation the trusted terminal draws there.
    async fn publish_app_tabs(
        &self,
        session_id: &str,
        views: &crate::runtime::app_views::AppViews,
        panels: &[crate::runtime::browser_controller_app_view::BrowserAppViewPanel],
    ) {
        let (agent_id, scale) = if panels.is_empty() {
            (None, 1)
        } else {
            (
                self.focused_agent_id(session_id).await.ok().flatten(),
                self.room_environment_snapshot(session_id)
                    .map_or(1, |environment| environment.viewport.device_scale_factor),
            )
        };
        let apps = views
            .installations(session_id)
            .into_iter()
            .map(|(target, installation_id)| {
                let panel = panels
                    .iter()
                    .find(|panel| panel.target_id == target)
                    .map(|panel| crate::session::EnvironmentAppPanel {
                        x: panel.x.saturating_mul(scale),
                        y: panel.y.saturating_mul(scale),
                        width: panel.width.saturating_mul(scale),
                        height: panel.height.saturating_mul(scale),
                        agent_id: agent_id.clone(),
                    });
                let app = crate::session::EnvironmentTabApp {
                    installation_id,
                    panel,
                };
                (target, app)
            })
            .collect();
        if views.publish(session_id, &apps) {
            let _ = self.set_room_environment_app_tabs(session_id, &apps);
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
        // An undelivered answer leaves the binding alone: the next poll's
        // `open_targets` drops Tabs that really closed.
        let _ = self
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
        if lease.catalog().generation() != binding.generation {
            return Err(view_error(
                "APP_VIEW_STALE",
                "The App was updated; reopen its view",
            ));
        }
        let tool = view_tool(lease.catalog().app_catalog(), tool)
            .ok_or_else(|| view_error("UNKNOWN_TOOL", "The App declares no such tool"))?;
        let slot = lease
            .reserve_call(Duration::from_secs(30))
            .map_err(|error| view_error("APP_BUSY", &error.to_string()))?;
        slot.validate_input(&tool, &input)
            .map_err(|error| view_error("INVALID_INPUT", &error.to_string()))?;
        let permit = self.app_control().try_admit().map_err(|_| unavailable())?;
        let store = self.owned.durable_state_store.clone();
        let caller = view_caller(binding, session_id);
        let tool_name = tool;
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
        .map_err(app_error)
    }
}

/// The App's own error (e.g. CONFLICT from a stale edit) reaches its view;
/// kernel-side failures keep a generic code.
fn app_error(error: crate::durable_state::app_tools::AppToolsError) -> BrowserAppViewError {
    match error {
        crate::durable_state::app_tools::AppToolsError::Catalog(
            chariox_app_runtime::app_catalog::CatalogError::Worker(remote),
        ) => view_error(&remote.code, &remote.message),
        error => view_error("APP_ERROR", &error.to_string()),
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

/// A view calls the App's own tools by their local names; the catalog keys
/// them by the installation-namespaced runtime MCP name.
fn view_tool(
    catalog: &chariox_app_runtime::app_catalog::AppCatalog,
    local: &str,
) -> Option<String> {
    catalog
        .tools()
        .find(|tool| tool.local_name == local)
        .map(|tool| tool.name.clone())
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
            generation: 1,
        };
        let caller = view_caller(&binding, "session-1");
        assert!(matches!(&caller.actor, Actor::Human(owner) if owner == "alice"));
        assert_eq!(caller.room_id, "session-1");
        assert!(caller.task_id.is_none() && caller.turn_id.is_none());
    }

    #[test]
    fn view_calls_name_the_apps_own_tools_by_local_name() {
        let root =
            std::env::temp_dir().join(format!("chariox-view-tool-{:016x}", rand::random::<u64>()));
        std::fs::create_dir(&root).unwrap();
        let store =
            crate::durable_state::DurableKernelStateStore::open_owned(root.join("kernel.sqlite"))
                .unwrap();
        let catalog = crate::durable_state::app_state::fixture_tool_catalog(&store);
        let catalog = catalog.app_catalog();
        let namespaced = view_tool(catalog, "echo").unwrap();
        assert_ne!(namespaced, "echo");
        assert!(catalog.tool(&namespaced).is_some());
        assert_eq!(view_tool(catalog, "missing"), None);
        // A namespaced name is not a local name.
        assert_eq!(view_tool(catalog, &namespaced), None);
        drop(store);
        let _ = std::fs::remove_dir_all(root);
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
