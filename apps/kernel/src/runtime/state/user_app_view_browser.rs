//! Chromium fallback for ephemeral user-domain instances. One drain per owner
//! uses the existing host/controller; App authority stays in the shared queue.
use super::{
    app_view_host::{AppViewHost, KernelBrowserAppViewHost},
    KernelRuntimeState,
};
use crate::{
    error::DaemonError,
    local::{AppFrontendBundle, UserAppView, UserAppViewBrowser},
    runtime::browser_controller_app_view::{
        BrowserAppViewCall, BrowserAppViewCalls, BrowserAppViewRequest,
    },
};

#[derive(serde::Deserialize)]
struct Opened {
    target_id: String,
    origin: String,
    tab_id: String,
    generation: u64,
}

impl KernelRuntimeState {
    pub(super) async fn open_user_app_browser(
        &self,
        owner: &str,
        view: &UserAppView,
        frontend: &AppFrontendBundle,
    ) -> Result<UserAppView, DaemonError> {
        let host = KernelBrowserAppViewHost {
            state: self,
            owner,
            generation: None,
        };
        let value = host
            .command(BrowserAppViewRequest::Open {
                origin_label: super::app_view_runtime::origin_label(owner, &view.installation_id),
                installation_id: view.installation_id.clone(),
                instance_id: Some(view.view_id.clone()),
                entry: frontend.entry.clone(),
                assets: frontend.assets.clone(),
                page: None,
            })
            .await?
            .ok_or_else(|| super::app_view_runtime::open_error("Kernel App host unavailable"))?;
        let opened: Opened = serde_json::from_value(value)
            .map_err(|_| super::app_view_runtime::open_error("Invalid kernel App host response"))?;
        let instances = self.app_control().user_views();
        let bound = if opened.origin == view.origin
            && !opened.target_id.is_empty()
            && opened.tab_id.starts_with("host-tab-")
            && opened.generation > 0
        {
            instances.bind_browser(
                owner,
                &view.view_id,
                opened.target_id.clone(),
                UserAppViewBrowser {
                    tab_id: opened.tab_id,
                    generation: opened.generation,
                },
            )
        } else {
            None
        };
        let Some(bound) = bound else {
            let _ = KernelBrowserAppViewHost {
                state: self,
                owner,
                generation: Some(opened.generation),
            }
            .command(BrowserAppViewRequest::Close {
                target_id: opened.target_id,
            })
            .await;
            return Err(super::app_view_runtime::open_error(
                "App opening cancelled or host identity changed",
            ));
        };
        if instances.begin_browser_pump(owner) {
            let state = self.clone();
            let owner = owner.to_owned();
            tokio::spawn(async move { state.pump_user_app_browser(owner).await });
        }
        Ok(bound)
    }

    pub(super) fn forget_user_app_view(&self, owner: &str, id: &str) {
        self.app_control().user_views().close(owner, id);
        self.app_control().views().forget_session(id);
        self.app_control().views().keep_pumping(id);
    }

    pub(super) async fn close_user_app_view(&self, owner: &str, id: &str) {
        let instances = self.app_control().user_views();
        let hosted = instances
            .get(owner, id)
            .and_then(|(_, view)| view.browser)
            .zip(instances.browser_target(owner, id));
        // Revocation wins immediately, even if Chromium is gone or close fails.
        self.forget_user_app_view(owner, id);
        if let Some((browser, target_id)) = hosted {
            let _ = KernelBrowserAppViewHost {
                state: self,
                owner,
                generation: Some(browser.generation),
            }
            .command(BrowserAppViewRequest::Close { target_id })
            .await;
        }
    }

    async fn pump_user_app_browser(self, owner: String) {
        let instances = self.app_control().user_views();
        let views = self.app_control().views();
        let mut cadence = super::app_view_poll::AppViewPoll::new();
        while instances.keep_browser_pumping(&owner) {
            cadence.tick().await;
            let polled = instances.browser_views(&owner);
            let Some(generation) = polled
                .first()
                .and_then(|v| v.browser.as_ref())
                .map(|b| b.generation)
            else {
                continue;
            };
            let host = KernelBrowserAppViewHost {
                state: &self,
                owner: &owner,
                generation: Some(generation),
            };
            let batch = match host
                .command(BrowserAppViewRequest::Calls)
                .await
                .and_then(|v| {
                    serde_json::from_value::<BrowserAppViewCalls>(v.unwrap_or_default())
                        .map_err(|_| super::app_view_runtime::open_error("Invalid App call drain"))
                }) {
                Ok(batch) => batch,
                Err(_) => {
                    // Browser loss invalidates these instances; ordinary tabs may
                    // restore, App instances never gain authority on restoration.
                    for view in polled.iter().filter(|v| {
                        v.browser
                            .as_ref()
                            .is_some_and(|b| b.generation == generation)
                    }) {
                        self.close_user_app_view(&owner, &view.view_id).await;
                    }
                    cadence.failed();
                    continue;
                }
            };
            cadence.observed_calls(!batch.calls.is_empty());
            for view in &polled {
                let Some(target) = instances.browser_target(&owner, &view.view_id) else {
                    continue;
                };
                if batch
                    .open_targets
                    .as_ref()
                    .is_some_and(|open| !open.contains(&target))
                {
                    self.forget_user_app_view(&owner, &view.view_id);
                    continue;
                }
                let documents = batch.documents.as_ref().map(|docs| {
                    docs.get(&target)
                        .cloned()
                        .map(|doc| std::collections::HashMap::from([(view.view_id.clone(), doc)]))
                        .unwrap_or_default()
                });
                views.cancel_gone_calls(&view.view_id, None, documents.as_ref());
            }
            for call in batch.calls {
                let Some(view) = instances.browser_view(&owner, &call.target_id) else {
                    continue;
                };
                if view.installation_id != call.installation_id
                    || view
                        .browser
                        .as_ref()
                        .is_none_or(|b| b.generation != generation)
                    || call.document_id.as_ref().is_some_and(|doc| {
                        batch
                            .documents
                            .as_ref()
                            .is_some_and(|docs| docs.get(&call.target_id) != Some(doc))
                    })
                {
                    continue;
                }
                let tracked =
                    views.track_call(&view.view_id, &view.view_id, call.document_id.clone());
                let state = self.clone();
                let owner = owner.clone();
                tokio::spawn(async move {
                    state
                        .answer_user_app_browser_call(owner, view, call, tracked)
                        .await
                });
            }
        }
    }

    async fn answer_user_app_browser_call(
        self,
        owner: String,
        view: UserAppView,
        call: BrowserAppViewCall,
        mut tracked: crate::runtime::app_views::ViewCall,
    ) {
        let Some((binding, _)) = self.app_control().user_views().get(&owner, &view.view_id) else {
            return;
        };
        let outcome = self
            .invoke_user_app_view_call(
                &owner,
                &view.view_id,
                &binding,
                &call.method,
                call.params,
                &mut tracked,
            )
            .await;
        if tracked.is_cancelled()
            || self
                .app_control()
                .user_views()
                .get(&owner, &view.view_id)
                .is_none()
        {
            return;
        }
        let (result, error) = match outcome {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error)),
        };
        let _ = KernelBrowserAppViewHost {
            state: &self,
            owner: &owner,
            generation: view.browser.map(|b| b.generation),
        }
        .command(BrowserAppViewRequest::Respond {
            target_id: call.target_id,
            call_id: call.call_id,
            result,
            error,
        })
        .await;
    }
}
