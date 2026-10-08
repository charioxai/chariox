//! App views: the installation's signed UI runs as a managed Tab in the
//! session's Room browser, so people and agents share one DOM and profile.
//! `window.chariox.call(tool, input)` runs the App's own tool as the human
//! owner through the same catalog, validation and durable path as agent calls.
use super::KernelRuntimeState;
#[path = "app_view_recovery.rs"]
mod recovery;
use super::app_view_host::{AppViewHost, RoomAppViewHost};
use crate::{
    error::DaemonError,
    local::{AppRequestErrorCode, LocalDaemonRequest, LocalDaemonResponse},
    runtime::{
        app_call_errors,
        app_views::{AppViewBinding, ViewCall},
        app_worker::AppWorkerError,
        browser_controller_app_view::{
            AppViewPage, BrowserAppViewAsset, BrowserAppViewCall, BrowserAppViewCalls,
            BrowserAppViewError, BrowserAppViewOpened, BrowserAppViewRequest,
        },
        command::KernelCommand,
    },
};
use base64::Engine;
use chariox_app_runtime::app_catalog::{Actor, CallerContext};
use recovery::{cold_restore_error, AppViewRecovery, ColdAppRestoreError};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::Duration;

const COMMAND_RETRY: Duration = Duration::from_millis(100);
/// An Open or a Reload waits for a running Room command (bounded by the
/// controller's 15 s command timeout); an answer is retried a few times.
const OPEN_WAIT: Duration = Duration::from_secs(16);
const RESPOND_ATTEMPTS: u32 = 5;
/// A view's first call re-projects the Room, waiting this long for a busy slice.
const REPROJECT_WINDOW: Duration = Duration::from_secs(5);
/// The page's agent panel request; answered by the kernel, not the App.
const PANEL_METHOD: &str = "chariox.panel";

impl KernelRuntimeState {
    pub(crate) async fn execute_app_view_request(
        &self,
        command: &KernelCommand,
        request: &LocalDaemonRequest,
    ) -> Option<Result<LocalDaemonResponse, DaemonError>> {
        let owner = match request {
            LocalDaemonRequest::OpenAppView(_) | LocalDaemonRequest::SetAppViewPanel(_) => {
                match crate::runtime::app_control::owner(command) {
                    Ok(owner) => owner,
                    Err(code) => return Some(Ok(crate::runtime::app_control::failed(code))),
                }
            }
            _ => return None,
        };
        Some(match request {
            LocalDaemonRequest::OpenAppView(request) => {
                self.open_app_view(owner, &request.session_id, &request.installation_id)
                    .await
            }
            LocalDaemonRequest::SetAppViewPanel(request) => Ok(self
                .set_app_view_panel(request)
                .await
                .unwrap_or_else(crate::runtime::app_control::failed)),
            _ => return None,
        })
    }

    /// The user's panel choice for an App's open views: it wins over the App.
    async fn set_app_view_panel(
        &self,
        request: &crate::local::SetAppViewPanelRequest,
    ) -> Result<LocalDaemonResponse, AppRequestErrorCode> {
        let views = self.app_control().views().clone();
        let user = views
            .set_user_panel(
                &request.session_id,
                &request.installation_id,
                request.placement,
                request.minimized,
                request.reset,
            )
            .ok_or(AppRequestErrorCode::NotFound)?;
        self.publish_app_tabs(&request.session_id, &views).await;
        Ok(LocalDaemonResponse::AppViewPanelSet {
            installation_id: request.installation_id.clone(),
            placement: user.placement,
            minimized: user.minimized,
        })
    }

    async fn open_app_view(
        &self,
        owner: String,
        session_id: &str,
        installation: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let store = self.owned.durable_state_store.clone();
        let (view_owner, view_installation) = (owner.clone(), installation.to_owned());
        let view = tokio::task::spawn_blocking(move || {
            store.app_view_assets(&view_owner, &view_installation)
        })
        .await
        .map_err(|_| open_error("App storage is unavailable. Try again shortly."))?
        .map_err(|error| open_error(error.message()))?;
        let generation = view.generation;
        let panel = crate::runtime::app_views::PanelRequest::from_manifest(view.panel.as_ref());
        let views = self.app_control().views().clone();
        // The page's first layout already leaves its panel free.
        let page = self
            .room_environment_snapshot(session_id)
            .ok()
            .map(|environment| {
                let layout = views.opening_layout(session_id, installation, panel);
                environment.viewport.app_layout(layout, None).0
            });
        let (entry, assets) = view_assets(view);
        let request = BrowserAppViewRequest::Open {
            origin_label: origin_label(&owner, installation),
            installation_id: installation.to_owned(),
            instance_id: None,
            entry,
            assets,
            page: page.map(|(width, height)| AppViewPage { width, height }),
        };
        let opened: BrowserAppViewOpened = self.app_view_command(session_id, request).await?;
        if let Some(page) = page {
            views.sent_page(session_id, &opened.target_id, page);
        }
        if views.register(
            session_id,
            &opened.target_id,
            AppViewBinding {
                logical_tab: None,
                owner: owner.clone(),
                installation: installation.to_owned(),
                generation,
                panel,
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
            return Err(open_error("The App release became unavailable while opening its view. Use /app status <installation-id> to check it before retrying."));
        }
        // Project the new Tab into the Room so viewers and agents see it.
        let _ = self
            .reconcile_browser_controller_environment(session_id)
            .await;
        self.remember_app_logical_tabs(session_id, &views);
        let bound_agent_id = self.foreground_app(session_id, &opened.target_id).await;
        Ok(LocalDaemonResponse::AppViewOpened {
            installation_id: installation.to_owned(),
            target_id: opened.target_id,
            origin: opened.origin,
            bound_agent_id,
        })
    }

    /// A view command either runs or is final, with two exceptions. An Open
    /// or a Reload takes the slice's operation slot, so it waits while another
    /// Room command holds it (that rejection means it never ran; an Open is
    /// not idempotent, and a Reload refused this way would unbind a working
    /// view). An answer is idempotent, so any failure is retried.
    async fn app_view_command<T: serde::de::DeserializeOwned>(
        &self,
        session_id: &str,
        request: BrowserAppViewRequest,
    ) -> Result<T, DaemonError> {
        let open = waits_for_slot(&request);
        let respond = matches!(request, BrowserAppViewRequest::Respond { .. });
        let deadline = tokio::time::Instant::now() + OPEN_WAIT;
        let mut attempt = 0;
        loop {
            attempt += 1;
            match (RoomAppViewHost { state: self, session: session_id }).command(request.clone()).await {
                Ok(Some(value)) => return serde_json::from_value(value)
                    .map_err(|_| open_error("The Room browser returned an invalid App view response. Restart the Room Environment and try again.")),
                Ok(None) => return Err(open_error("This Room has no browser controller available. Bind an Environment with /room bind <slice> and start it with /room start, then retry /app open.")),
                Err(error)
                    if (open
                        && tokio::time::Instant::now() < deadline
                        && crate::runtime::app_views::slice_busy(&error.to_string()))
                        || (respond && attempt < RESPOND_ATTEMPTS) =>
                {
                    tokio::time::sleep(COMMAND_RETRY).await;
                }
                Err(error) => {
                    tracing::debug!(%error, "App view controller command failed");
                    return Err(open_error(&error.to_string()));
                }
            }
        }
    }

    async fn pump_app_view_calls(self, session_id: String) {
        let views = self.app_control().views().clone();
        let mut recovery = AppViewRecovery::default();
        let mut polls = super::app_view_poll::AppViewPoll::new();
        let mut identity_probe_after = tokio::time::Instant::now();
        while views.keep_pumping(&session_id) {
            polls.tick().await;
            if !views.cold_start_views(&session_id).is_empty() {
                if cold_restore_waiting_for_slice(
                    self.owned
                        .slice_store
                        .environment_slice(&session_id)
                        .map(|slice| slice.status),
                ) {
                    // An explicit stop retains intent while the worker is down.
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
                if let Some(binding) = views.next_cold_start_attempt(&session_id) {
                    let result = self.restore_cold_app_view(&session_id, &binding).await;
                    recovery.restore_finished(&views, &session_id, &binding, result);
                }
                // Poll registered targets after every attempt, even if another
                // restore failed. Their bindings already carry call authority.
            }
            let polled_up_to = views.registrations(&session_id);
            let polled = self
                .app_view_command::<BrowserAppViewCalls>(&session_id, BrowserAppViewRequest::Calls)
                .await;
            let batch = match polled {
                Ok(batch) => batch,
                // Both end fast polling; only a genuine failure spends the budget.
                Err(error) if crate::runtime::app_views::slice_busy(&error.to_string()) => {
                    polls.failed();
                    continue;
                }
                Err(error) if browser_recovery_downtime(&error.to_string()) => {
                    // Positive browser/controller loss ends old document
                    // authority but retains the user's open-view intent.
                    views.suspend_for_cold_start(&session_id);
                    polls.failed();
                    continue;
                }
                Err(error) => {
                    recovery.poll_failed(&views, &session_id, &error);
                    polls.failed();
                    continue;
                }
            };
            recovery.poll_succeeded();
            polls.observed_calls(!batch.calls.is_empty());
            retain_app_poll_targets(
                &views,
                &session_id,
                &batch,
                polled_up_to,
                &mut identity_probe_after,
                self.reconcile_browser_controller_environment(&session_id),
            )
            .await;
            self.reload_updated_app_views(&session_id, &views);
            // App Tabs are always marked; an older controller, which does not
            // lay pages out beside a panel, gets no panel.
            views.set_app_panels(&session_id, batch.app_panels);
            self.publish_app_tabs(&session_id, &views).await;
            // A view's first call means its document loaded: project the
            // Room again so the Tab shows the App's title and URL, not the
            // blank page it had when it opened.
            // Every Tab in the batch is marked; one reconcile covers them.
            let first: Vec<String> = batch
                .calls
                .iter()
                .filter(|call| views.first_call(&session_id, &call.target_id))
                .map(|call| call.target_id.clone())
                .collect();
            if !first.is_empty() {
                let state = self.clone();
                let session = session_id.clone();
                let views = views.clone();
                tokio::spawn(async move {
                    let reconcile = || async {
                        state
                            .reconcile_browser_controller_environment(&session)
                            .await
                            .map(|_| ())
                            .map_err(|error| error.to_string())
                    };
                    let projected = crate::runtime::app_views::reproject(
                        reconcile,
                        REPROJECT_WINDOW,
                        COMMAND_RETRY,
                    )
                    .await;
                    if !projected {
                        for target in first {
                            views.forget_call(&session, &target);
                        }
                    }
                });
            }
            for call in batch.calls {
                let tracked =
                    views.track_call(&session_id, &call.target_id, call.document_id.clone());
                let state = self.clone();
                let session = session_id.clone();
                tokio::spawn(
                    async move { state.answer_app_view_call(session, call, tracked).await },
                );
            }
            // A call whose Tab closed, reloaded or navigated away is
            // cancelled: the worker gets `cancel` and its slot is freed.
            let cancelled = views.cancel_gone_calls(
                &session_id,
                batch.open_targets.as_deref(),
                batch.documents.as_ref(),
            );
            if cancelled > 0 {
                tracing::debug!(
                    cancelled,
                    "App view calls cancelled: their document is gone"
                );
            }
        }
    }

    async fn restore_cold_app_view(
        &self,
        session: &str,
        binding: &AppViewBinding,
    ) -> Result<(), ColdAppRestoreError> {
        // A local Room's explicit Stop must win over pending recovery intent.
        // Slice stops retain their existing intent behind the slice status gate.
        if self.owned.slice_store.environment_slice(session).is_none()
            && self.room_environment_snapshot(session).is_ok_and(|room| {
                matches!(
                    room.lifecycle,
                    crate::session::EnvironmentLifecycle::Stopped
                        | crate::session::EnvironmentLifecycle::Stopping
                        | crate::session::EnvironmentLifecycle::Failed
                )
            })
        {
            return Err(AppRequestErrorCode::NotFound.into());
        }
        // Acquire before reading assets: a Running slice can still be held by
        // slice.start while its attached agents relaunch. Admission refusals
        // are downtime, and spend neither restore nor polling failure budgets.
        self.ensure_browser_controller_process_started(session)
            .await
            .map_err(cold_restore_error)?;
        let store = self.owned.durable_state_store.clone();
        let (owner, installation) = (binding.owner.clone(), binding.installation.clone());
        let view =
            tokio::task::spawn_blocking(move || store.app_view_assets(&owner, &installation))
                .await
                .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
                .map_err(|error| match error {
                    crate::durable_state::app_view_assets::AppViewAssetsError::TooLarge => {
                        AppRequestErrorCode::LimitExceeded
                    }
                    _ => AppRequestErrorCode::StorageUnavailable,
                })?;
        let generation = view.generation;
        let panel = crate::runtime::app_views::PanelRequest::from_manifest(view.panel.as_ref());
        let views = self.app_control().views().clone();
        // Lay the page out beside its panel from the start, as a normal Open.
        let page = self
            .room_environment_snapshot(session)
            .ok()
            .map(|environment| {
                let layout = views.opening_layout(session, &binding.installation, panel);
                environment.viewport.app_layout(layout, None).0
            });
        let (entry, assets) = view_assets(view);
        let opened: BrowserAppViewOpened = self
            .app_view_command(
                session,
                BrowserAppViewRequest::Open {
                    origin_label: origin_label(&binding.owner, &binding.installation),
                    installation_id: binding.installation.clone(),
                    instance_id: None,
                    entry,
                    assets,
                    page: page.map(|(width, height)| AppViewPage { width, height }),
                },
            )
            .await
            .map_err(cold_restore_error)?;
        if let Some(page) = page {
            views.sent_page(session, &opened.target_id, page);
        }
        // Reattach before publishing this target as a bound App view. A poll
        // must never remember a temporary restored target's newly allocated Tab.
        if let Some(tab) = &binding.logical_tab {
            self.owned
                .session_store
                .write()
                .restore_room_environment_app_tab(session, tab, &opened.target_id)
                .map_err(|_| ColdAppRestoreError::Failed(AppRequestErrorCode::Conflict))?;
        }
        views.register(
            session,
            &opened.target_id,
            AppViewBinding {
                generation,
                panel,
                ..binding.clone()
            },
        );
        // Match normal Open's uninstall race gate: an install may disappear
        // while the browser command is in flight, after its assets were read.
        let store = self.owned.durable_state_store.clone();
        let (owner, installation) = (binding.owner.clone(), binding.installation.clone());
        if !tokio::task::spawn_blocking(move || {
            store.active_app_release(&owner, &installation).is_ok()
        })
        .await
        .unwrap_or(false)
        {
            views.forget_installation(&binding.owner, &binding.installation);
            return Err(AppRequestErrorCode::NotFound.into());
        }
        let _ = self.reconcile_browser_controller_environment(session).await;
        self.publish_app_tabs(session, &views).await;
        // Publishing the verified binding restores its logical Tab and focus.
        // Bring the physical page to the same focus through the normal path.
        if let Ok(environment) = self.room_environment_snapshot(session) {
            if let Some(tab_id) = environment.focused_tab_id {
                if self
                    .room_environment_controller_tab_binding(session, &tab_id)
                    .is_ok_and(|tab| tab.runtime_target_id == opened.target_id)
                {
                    let _ = self
                        .restore_browser_environment_tab_focus(session, &tab_id)
                        .await;
                }
            }
        }
        Ok(())
    }

    fn remember_app_logical_tabs(
        &self,
        session_id: &str,
        views: &crate::runtime::app_views::AppViews,
    ) {
        // Remember the home Tab only after projection, before any recovery
        // detaches the physical target. Polls may refresh document revisions.
        if let Ok(environment) = self.room_environment_snapshot(session_id) {
            for target in views.layouts(session_id).keys() {
                if let Ok(Some(tab_id)) =
                    self.room_environment_tab_id_for_controller_target(session_id, target)
                {
                    if let Some(tab) = environment.tabs.iter().find(|tab| tab.tab_id == tab_id) {
                        views.remember_logical_tab(session_id, target, tab.clone());
                    }
                }
            }
        }
    }

    /// Marks the Room's App view Tabs. The Room draws each one's panel beside
    /// the App page, showing the session's focus agent.
    ///
    /// Each page gets its CSS size beside its panel when that changed: a
    /// placement request, the user's choice or a viewport change.
    async fn publish_app_tabs(
        &self,
        session_id: &str,
        views: &crate::runtime::app_views::AppViews,
    ) {
        let app_panels = views.app_panels(session_id);
        let Ok(pages) =
            self.set_room_environment_app_tabs(session_id, views.layouts(session_id), app_panels)
        else {
            return;
        };
        self.remember_app_logical_tabs(session_id, views);
        if !app_panels {
            return;
        }
        for (target_id, (width, height)) in views.changed_pages(session_id, &pages) {
            let request = BrowserAppViewRequest::Layout {
                target_id: target_id.clone(),
                page: AppViewPage { width, height },
            };
            if self
                .app_view_command::<Value>(session_id, request)
                .await
                .is_err()
            {
                views.forget_page(session_id, &target_id);
            }
        }
    }

    /// `chariox.panel` from an App page: where it wants its agent panel
    /// (`{placement: "right" | "bottom" | "none", size?}`), or with no
    /// placement just the current layout. The user's choice still wins.
    async fn request_app_panel(
        &self,
        session_id: &str,
        target_id: &str,
        params: Value,
    ) -> Result<Value, BrowserAppViewError> {
        let invalid = || {
            view_error(
                "INVALID_PANEL",
                "A panel request is {placement: \"right\" | \"bottom\" | \"none\", size?: 120..1200}",
            )
        };
        let views = self.app_control().views().clone();
        if let Some(placement) = params.get("placement") {
            let placement = match placement.as_str() {
                Some("right") => Some(crate::session::AppPanelPlacement::Right),
                Some("bottom") => Some(crate::session::AppPanelPlacement::Bottom),
                Some("none") => None,
                _ => return Err(invalid()),
            };
            let size = match params.get("size") {
                None | Some(Value::Null) => None,
                Some(size) => match size.as_u64() {
                    Some(size) if placement.is_some() && (120..=1200).contains(&size) => {
                        Some(size as u32)
                    }
                    _ => return Err(invalid()),
                },
            };
            let request = crate::runtime::app_views::PanelRequest { placement, size };
            if !views.request_panel(session_id, target_id, request) {
                return Err(view_error(
                    "APP_VIEW_UNBOUND",
                    "This view is not bound to an App",
                ));
            }
            self.publish_app_tabs(session_id, &views).await;
        }
        let layout = views
            .layouts(session_id)
            .remove(target_id)
            .map(|(_, layout)| layout)
            .unwrap_or_default();
        Ok(serde_json::json!({
            "placement": match layout.placement {
                Some(crate::session::AppPanelPlacement::Right) => "right",
                Some(crate::session::AppPanelPlacement::Bottom) => "bottom",
                None => "none",
            },
            "minimized": layout.minimized,
        }))
    }

    async fn answer_app_view_call(
        self,
        session_id: String,
        call: BrowserAppViewCall,
        mut tracked: ViewCall,
    ) {
        if tracked.is_cancelled() {
            return;
        }
        let views = self.app_control().views().clone();
        let unbound = || view_error("APP_VIEW_UNBOUND", "This view is not bound to an App");
        let outcome = match views.binding_state(&session_id, &call.target_id) {
            // Bound to the new generation, but still showing the old page.
            Some((binding, true)) if binding.installation == call.installation_id => {
                Err(view_reloading())
            }
            Some((binding, false))
                if binding.installation == call.installation_id && call.method == PANEL_METHOD =>
            {
                self.request_app_panel(&session_id, &call.target_id, call.params)
                    .await
            }
            Some((binding, false)) if binding.installation == call.installation_id => {
                match self
                    .invoke_app_view_tool(
                        Some(&session_id),
                        &binding,
                        &call.method,
                        call.params,
                        &mut tracked,
                    )
                    .await
                {
                    // Built for an older generation: reload it with the current one.
                    Err(error)
                        if error.code == "APP_VIEW_STALE"
                            && views.reloadable(&session_id, &call.target_id) =>
                    {
                        self.reconnect_app_view(
                            &session_id,
                            &call.target_id,
                            &binding.owner,
                            &binding.installation,
                        )
                        .await
                    }
                    outcome => outcome,
                }
            }
            // A view left open across a kernel restart. Only the session host's
            // own active installation is reconnected; an uninstalled App, or a
            // Tab bound to another installation, stays unbound.
            None => match self
                .session_host(&session_id)
                .filter(|_| views.reloadable(&session_id, &call.target_id))
            {
                Some(owner) => {
                    self.reconnect_app_view(
                        &session_id,
                        &call.target_id,
                        &owner,
                        &call.installation_id,
                    )
                    .await
                }
                None => Err(unbound()),
            },
            Some(_) => Err(unbound()),
        };
        // Nobody is left to answer; the controller would not deliver it.
        if tracked.is_cancelled() {
            return;
        }
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

    /// An update that committed while a view is open reloads the view onto
    /// the new generation now. Until then the controller keeps serving the old
    /// generation's files, so reloading the page would show the old release
    /// again; only a call from the page would find it stale.
    fn reload_updated_app_views(
        &self,
        session_id: &str,
        views: &crate::runtime::app_views::AppViews,
    ) {
        let control = self.app_control();
        let outdated = views.take_outdated(session_id, |binding| {
            control
                .active_app_lease(&binding.owner, &binding.installation)
                .map(|lease| lease.catalog().generation())
        });
        for (target, binding) in outdated {
            let state = self.clone();
            let session = session_id.to_owned();
            let views = views.clone();
            tokio::spawn(async move {
                // Its outcome is for a call; the reload is the point here.
                let _ = state
                    .reconnect_app_view(&session, &target, &binding.owner, &binding.installation)
                    .await;
                views.finish_refresh(&session, &target);
            });
        }
    }

    /// Binds the Tab to the installation's current generation, then reloads it
    /// with that generation's view; the interrupted call asks the page to retry.
    async fn reconnect_app_view(
        &self,
        session_id: &str,
        target_id: &str,
        owner: &str,
        installation: &str,
    ) -> Result<Value, BrowserAppViewError> {
        let unbound = || view_error("APP_VIEW_UNBOUND", "This view is not bound to an App");
        let store = self.owned.durable_state_store.clone();
        let (view_owner, view_installation) = (owner.to_owned(), installation.to_owned());
        let views = self.app_control().views().clone();
        let Ok(Ok(view)) = tokio::task::spawn_blocking(move || {
            store.app_view_assets(&view_owner, &view_installation)
        })
        .await
        else {
            // Not the host's active installation (or unreadable): answered
            // unbound without re-reading the release on every call.
            views.unbind(session_id, target_id);
            return Err(unbound());
        };
        // A view's concurrent calls each land here; one reload is enough.
        if !views.claim_reconnect(
            session_id,
            target_id,
            AppViewBinding {
                logical_tab: None,
                owner: owner.to_owned(),
                installation: installation.to_owned(),
                generation: view.generation,
                panel: crate::runtime::app_views::PanelRequest::from_manifest(view.panel.as_ref()),
            },
        ) {
            return Err(view_reloading());
        }
        let (entry, assets) = view_assets(view);
        let reloaded: Result<Value, DaemonError> = self
            .app_view_command(
                session_id,
                BrowserAppViewRequest::Reload {
                    target_id: target_id.to_owned(),
                    entry,
                    assets,
                },
            )
            .await;
        if reloaded.is_err() {
            // Unbound before the mark goes, so no call runs in between.
            views.unbind(session_id, target_id);
            views.finish_reconnect(session_id, target_id);
            return Err(unbound());
        }
        views.finish_reconnect(session_id, target_id);
        Err(view_reloading())
    }

    /// The session's host, who owns the Room's reconnected views.
    fn session_host(&self, session_id: &str) -> Option<String> {
        self.owned
            .session_store
            .get_session(session_id)
            .ok()
            .map(|session| session.owner_user_id().to_owned())
    }

    /// Once per session and kernel, as soon as its Room is bound to a slice,
    /// polls it so views that survived a restart reconnect instead of hanging.
    pub(crate) fn resume_app_view_pumps(&self) {
        let views = self.app_control().views().clone();
        for session_id in self.owned.slice_store.environment_sessions() {
            if views.take_resume(&session_id) && views.begin_pumping(&session_id) {
                let state = self.clone();
                tokio::spawn(async move { state.pump_app_view_calls(session_id).await });
            }
        }
    }

    pub(super) async fn invoke_app_view_tool(
        &self,
        session_id: Option<&str>,
        binding: &AppViewBinding,
        tool: &str,
        input: Value,
        tracked: &mut ViewCall,
    ) -> Result<Value, BrowserAppViewError> {
        let unavailable = || view_error("APP_UNAVAILABLE", "The App is not running");
        let leased = tokio::select! {
            result = self.app_lease_on_demand(&binding.owner, &binding.installation) => result,
            () = tracked.cancelled() => return Err(view_error("CANCELLED", "The calling view went away")),
        };
        let lease = match leased {
            Ok(lease) => lease,
            Err(_)
                if self
                    .app_updating(&binding.owner, &binding.installation)
                    .await =>
            {
                return Err(coded(app_call_errors::updating()));
            }
            Err(_) => return Err(unavailable()),
        };
        if tracked.is_cancelled() {
            return Err(view_error("CANCELLED", "The calling view went away"));
        }
        if lease.catalog().generation() != binding.generation {
            return Err(view_error(
                "APP_VIEW_STALE",
                "The App was updated; reopen its view",
            ));
        }
        if matches!(tool, "host.clipboard_write" | "host.open_link") {
            return crate::runtime::app_host_broker::AppHostBroker::new(
                self.owned.durable_state_store.clone(),
                binding.owner.clone(),
                lease.catalog().clone(),
                self.app_control().admission(),
            )
            .request(tool, input)
            .await
            .map_err(|error| view_error(&error.code, &error.message));
        }
        let tool = view_call_tool(lease.catalog(), binding.generation, tool)?;
        let slot = lease
            .reserve_call(Duration::from_secs(30))
            .map_err(view_call_error)?;
        slot.validate_input(&tool, &input)
            .map_err(|error| coded(app_call_errors::input_error(&error)))?;
        let permit = self
            .app_control()
            .try_admit()
            .map_err(|_| coded(app_call_errors::admission_busy()))?;
        let store = self.owned.durable_state_store.clone();
        let caller = view_caller(binding, session_id);
        let tool_name = tool;
        let operation_budget = tracked.operation_budget();
        let response = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            store.enqueue_app_tool(slot, &tool_name, input, caller, operation_budget)
        })
        .await
        .map_err(|_| unavailable())?
        .map_err(|error| coded(app_call_errors::enqueue_error(&error)))?;
        // Dropping the pending reply makes the worker peer send `cancel`.
        let reply = tokio::select! {
            reply = response.receive() => reply
                .map_err(|error| coded(app_call_errors::worker_call_error(&error)))?,
            () = tracked.cancelled() => {
                return Err(view_error("CANCELLED", "The calling view went away"));
            }
        };
        let permit = self
            .app_control()
            .admit_reply(reply.remaining(crate::session::unix_epoch_ms()))
            .await
            .map_err(|_| coded(app_call_errors::reply_unrecorded()))?;
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
/// see `app_call_errors` for the kernel's codes.
fn app_error(error: crate::durable_state::app_tools::AppToolsError) -> BrowserAppViewError {
    let (code, message) = crate::runtime::app_call_errors::tool_call_error(&error);
    view_error(&code, &message)
}

/// A view call runs as the view's owner, whoever drives the Tab: the view is
/// the owner's human surface, and a person or an agent operating the shared
/// Room browser acts through it with the owner's App authority (V-SDK-04: no
/// separate view privilege). Critical effects still need the kernel's human
/// validation, which a view click cannot provide.
fn view_caller(binding: &AppViewBinding, session_id: Option<&str>) -> CallerContext {
    CallerContext {
        actor: Actor::Human(binding.owner.clone()),
        room_id: session_id.map(str::to_owned),
        operation_id: format!("app-operation-{:016x}", rand::random::<u64>()),
        task_id: None,
        turn_id: None,
    }
}

/// One origin per owner and installation keeps each App's web storage apart.
pub(super) fn origin_label(owner: &str, installation: &str) -> String {
    let digest = Sha256::digest(format!("{owner}\0{installation}"));
    format!("a{}", &hex_prefix(&digest)[..24])
}

fn hex_prefix(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn coded((code, message): (String, String)) -> BrowserAppViewError {
    view_error(&code, &message)
}

pub(super) fn view_error(code: &str, message: &str) -> BrowserAppViewError {
    BrowserAppViewError {
        code: code.to_owned(),
        message: message.to_owned(),
    }
}

/// The tool a view's call runs. A view built for an older generation is stale
/// before its tool is looked up: a tool the update removed or changed (an API
/// mismatch) reloads the view instead of running on the new release.
fn view_call_tool(
    catalog: &chariox_app_runtime::app_outbox::EventCatalog,
    view_generation: u64,
    local: &str,
) -> Result<String, BrowserAppViewError> {
    if catalog.generation() != view_generation {
        return Err(view_error(
            "APP_VIEW_STALE",
            "The App was updated; reopen its view",
        ));
    }
    view_tool(catalog.app_catalog(), local)
        .ok_or_else(|| view_error("UNKNOWN_TOOL", "The App declares no such tool"))
}

fn view_call_error(error: AppWorkerError) -> BrowserAppViewError {
    match error {
        AppWorkerError::Busy => view_error("APP_BUSY", "app_worker_busy; retry after 500 ms"),
        AppWorkerError::Unavailable => view_error("APP_UNAVAILABLE", "The App is not running"),
        error => view_error("APP_ERROR", &error.to_string()),
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

pub(super) fn open_error(message: &str) -> DaemonError {
    DaemonError::AppViewUnavailable {
        message: message.to_owned(),
    }
}

fn view_reloading() -> BrowserAppViewError {
    view_error(
        "APP_VIEW_RELOADING",
        "The App changed; its view is reloading",
    )
}

/// Commands that take the slice's operation slot and, refused because another
/// Room command held it, never ran: they wait for the slot.
fn waits_for_slot(request: &BrowserAppViewRequest) -> bool {
    matches!(
        request,
        BrowserAppViewRequest::Open { .. } | BrowserAppViewRequest::Reload { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_open_errors_use_the_existing_nonretryable_request_error_envelope() {
        let error = open_error("This Room has no browser controller available.");
        let projected = crate::transport::kernel_protocol::map_kernel_error(&error);
        assert_eq!(projected.code, "kernel_request_failed");
        assert!(
            !projected.retryable,
            "rejected opens must not be replayed as transport failures"
        );
        assert!(projected
            .message
            .contains("no browser controller available"));
    }

    #[test]
    fn only_busy_view_admission_asks_the_page_to_retry() {
        let busy = view_call_error(AppWorkerError::Busy);
        assert_eq!(busy.code, "APP_BUSY");
        assert_eq!(busy.message, "app_worker_busy; retry after 500 ms");
        let stopped = view_call_error(AppWorkerError::Unavailable);
        assert_eq!(stopped.code, "APP_UNAVAILABLE");
        assert_eq!(stopped.message, "The App is not running");
        for error in [
            AppWorkerError::Identity,
            AppWorkerError::Deadline,
            AppWorkerError::Invalid,
        ] {
            let failed = view_call_error(error);
            assert_eq!(failed.code, "APP_ERROR");
            assert!(!failed.message.contains("retry"));
        }
    }

    #[test]
    fn view_calls_run_as_the_views_owner_in_its_room() {
        let binding = AppViewBinding {
            logical_tab: None,
            owner: "alice".into(),
            installation: "todo".into(),
            generation: 1,
            panel: Default::default(),
        };
        let caller = view_caller(&binding, Some("session-1"));
        assert!(matches!(&caller.actor, Actor::Human(owner) if owner == "alice"));
        assert_eq!(caller.room_id.as_deref(), Some("session-1"));
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
    fn a_view_of_an_older_generation_is_stale_before_its_tool_is_looked_up() {
        let root =
            std::env::temp_dir().join(format!("chariox-view-stale-{:016x}", rand::random::<u64>()));
        std::fs::create_dir(&root).unwrap();
        let store =
            crate::durable_state::DurableKernelStateStore::open_owned(root.join("kernel.sqlite"))
                .unwrap();
        let catalog = crate::durable_state::app_state::fixture_tool_catalog(&store);
        let current = catalog.generation();
        assert!(view_call_tool(&catalog, current, "echo").is_ok());
        assert_eq!(
            view_call_tool(&catalog, current, "missing")
                .unwrap_err()
                .code,
            "UNKNOWN_TOOL"
        );
        // A view built for another generation, calling a tool this release
        // lacks or has, never runs it here: it is stale and gets reloaded.
        for tool in ["echo", "missing"] {
            assert_eq!(
                view_call_tool(&catalog, current + 1, tool)
                    .unwrap_err()
                    .code,
                "APP_VIEW_STALE"
            );
        }
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_open_or_a_reload_waits_for_a_busy_slice() {
        let reload = BrowserAppViewRequest::Reload {
            target_id: "t1".into(),
            entry: "index.html".into(),
            assets: Vec::new(),
        };
        let open = BrowserAppViewRequest::Open {
            origin_label: "a".into(),
            installation_id: "app".into(),
            instance_id: None,
            entry: "index.html".into(),
            assets: Vec::new(),
            page: None,
        };
        assert!(waits_for_slot(&reload));
        assert!(waits_for_slot(&open));
        assert!(!waits_for_slot(&BrowserAppViewRequest::Calls));
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

pub(super) fn view_assets(
    view: crate::durable_state::app_view_assets::AppViewAssets,
) -> (String, Vec<BrowserAppViewAsset>) {
    let engine = base64::engine::general_purpose::STANDARD;
    let assets = view
        .assets
        .into_iter()
        .map(|asset| BrowserAppViewAsset {
            path: asset.path,
            content_type: asset.content_type.to_owned(),
            body_base64: engine.encode(asset.bytes),
        })
        .collect();
    (view.entry, assets)
}

/// Calls have already been drained from the controller. An inconclusive
/// identity check defers pruning only; it must never discard that call batch.
async fn retain_app_poll_targets<F, T, E>(
    views: &crate::runtime::app_views::AppViews,
    session: &str,
    batch: &BrowserAppViewCalls,
    up_to: u64,
    identity_probe_after: &mut tokio::time::Instant,
    reconcile: F,
) where
    F: std::future::Future<Output = Result<T, E>>,
{
    let Some(open) = &batch.open_targets else {
        return;
    };
    if views.has_missing_targets(session, open, up_to) {
        if tokio::time::Instant::now() < *identity_probe_after {
            return;
        }
        if reconcile.await.is_err() {
            // Throttle only the identity probe; drained call delivery keeps its
            // active cadence and always continues after this helper returns.
            *identity_probe_after =
                tokio::time::Instant::now() + super::app_view_poll::IDLE_INTERVAL;
            return;
        }
    }
    views.retain_open(session, open, up_to);
    views.set_open_tabs(session, open.len());
}

fn cold_restore_waiting_for_slice(status: Option<crate::slice::SliceStatus>) -> bool {
    // A local controller has no slice gate; it uses the same verified restore.
    status.is_some_and(|status| status != crate::slice::SliceStatus::Running)
}

pub(super) fn browser_recovery_downtime(message: &str) -> bool {
    [
        "browser_debugger_unavailable",
        "browser_cdp_disconnected",
        "browser_controller_lease_lost",
        "browser controller is not leased by Room ",
    ]
    .iter()
    .any(|code| message.contains(code))
}

#[cfg(test)]
#[path = "app_view_cold_start_tests.rs"]
mod cold_start_tests;
