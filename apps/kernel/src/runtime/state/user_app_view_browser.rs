//! Chromium fallback for ephemeral user-domain instances. One drain per owner
//! uses the existing host/controller; App authority stays in the shared queue.
use super::{
    app_view_host::{AppViewHost, KernelBrowserAppViewHost},
    app_view_runtime::{agent_not_granted, ViewCaller},
    KernelRuntimeState,
};
use crate::{
    error::DaemonError,
    local::{AppFrontendBundle, UserAppView, UserAppViewBrowser},
    runtime::app_views::AppViewBinding,
    runtime::browser_controller_app_view::{
        BrowserAppViewActor, BrowserAppViewCall, BrowserAppViewCalls, BrowserAppViewError,
        BrowserAppViewRequest,
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
        restoring: bool,
    ) -> Result<UserAppView, DaemonError> {
        let host = KernelBrowserAppViewHost {
            state: self,
            owner,
            generation: None,
        };
        let value = host
            .command(BrowserAppViewRequest::Open {
                projection: crate::runtime::browser_controller_app_view::AppViewProjection::Human,
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
            let bind = if restoring {
                crate::runtime::user_app_views::UserAppViews::bind_restoring_browser
            } else {
                crate::runtime::user_app_views::UserAppViews::bind_browser
            };
            bind(
                instances,
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
        self.remember_user_app_tabs(owner);
        if instances.begin_browser_pump(owner) {
            let state = self.clone();
            let owner = owner.to_owned();
            tokio::spawn(async move { state.pump_user_app_browser(owner).await });
        }
        Ok(bound)
    }

    /// MP-08/MP-11: one App, one owner and one kernel browser is one Tab.
    /// Opening it again shows that Tab with the current assets, as a Room's
    /// App Tab does. None when no live Tab of this release exists; the caller
    /// then opens a new one (a stale or updated Tab is retired first).
    pub(super) async fn focus_user_app_browser(
        &self,
        owner: &str,
        binding: &AppViewBinding,
        frontend: &AppFrontendBundle,
    ) -> Option<UserAppView> {
        let instances = self.app_control().user_views();
        let (hosted, view) = instances.hosted(owner, &binding.installation)?;
        let browser = view.browser.clone()?;
        let shown = hosted.generation == binding.generation
            && KernelBrowserAppViewHost {
                state: self,
                owner,
                generation: Some(browser.generation),
            }
            .command(BrowserAppViewRequest::Open {
                projection: crate::runtime::browser_controller_app_view::AppViewProjection::Human,
                origin_label: super::app_view_runtime::origin_label(owner, &view.installation_id),
                installation_id: view.installation_id.clone(),
                instance_id: Some(view.view_id.clone()),
                entry: frontend.entry.clone(),
                assets: frontend.assets.clone(),
                page: None,
            })
            .await
            .ok()
            .flatten()
            .and_then(|value| serde_json::from_value::<Opened>(value).ok())
            .is_some_and(|opened| {
                opened.origin == view.origin
                    && opened.tab_id == browser.tab_id
                    && opened.generation == browser.generation
            });
        if shown {
            return instances.get(owner, &view.view_id).map(|(_, view)| view);
        }
        self.retire_user_app_browser(owner, &view.view_id, false)
            .await;
        None
    }

    /// MP-08/MP-11: the agent whose input drove a call must still hold this
    /// App view in its own user-domain grant, for the view's owner.
    fn user_app_view_agent(
        &self,
        owner: &str,
        view_id: &str,
        actor: &BrowserAppViewActor,
    ) -> Result<crate::agent::AgentInstance, BrowserAppViewError> {
        let agent = (actor.kind == "agent")
            .then(|| self.owned.agent_store.get_agent(&actor.id).ok())
            .flatten()
            .ok_or_else(agent_not_granted)?;
        let user = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        let holds = self
            .user_domain_owner_aliases(&user)
            .iter()
            .any(|alias| alias == owner)
            && self.owned.kernel_browser_host.holds_resource(
                &user,
                agent.id(),
                &crate::local::UserDomainResource::AppView {
                    view_id: view_id.into(),
                },
            );
        holds.then_some(agent).ok_or_else(agent_not_granted)
    }

    pub(super) fn forget_user_app_view(&self, owner: &str, id: &str) {
        if self.app_control().user_views().close(owner, id) {
            self.remember_user_app_tabs(owner);
        }
        self.app_control().views().forget_session(id);
        self.app_control().views().keep_pumping(id);
    }

    /// The browser went away (stop, crash or restart): end the instance and
    /// keep the owner's intent, so the App reopens with the browser.
    pub(super) fn suspend_user_app_view(&self, owner: &str, id: &str) {
        // MP-08/MP-11: shutdown already retired this instance. A late pump
        // result must not replace the saved intent with the empty registry.
        if !self.app_control().user_views().suspend(owner, id) {
            return;
        }
        self.app_control().views().forget_session(id);
        self.app_control().views().keep_pumping(id);
        self.remember_user_app_tabs(owner);
    }

    pub(super) async fn close_user_app_view(&self, owner: &str, id: &str) {
        self.retire_user_app_browser(owner, id, false).await;
    }

    pub(super) async fn retire_user_app_browser(&self, owner: &str, id: &str, restore: bool) {
        let instances = self.app_control().user_views();
        let hosted = instances
            .get(owner, id)
            .and_then(|(_, view)| view.browser)
            .zip(instances.browser_target(owner, id));
        // Revocation wins immediately, even if Chromium is gone or close fails.
        if restore {
            self.suspend_user_app_view(owner, id);
        } else {
            self.forget_user_app_view(owner, id);
        }
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
                    // Browser loss invalidates these instances; an old instance
                    // never regains authority. The App reopens as a new one.
                    for view in polled.iter().filter(|v| {
                        v.browser
                            .as_ref()
                            .is_some_and(|b| b.generation == generation)
                    }) {
                        self.suspend_user_app_view(&owner, &view.view_id);
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
            for view in &polled {
                self.reload_updated_user_app_view(&owner, view).await;
            }
        }
    }

    /// An update that committed while a view is open reloads its page onto the
    /// new release (MP-08/MP-10): same Tab, and the App's drafts stay in its web
    /// storage. Until then the controller keeps serving the old generation's files.
    async fn reload_updated_user_app_view(&self, owner: &str, view: &UserAppView) {
        let instances = self.app_control().user_views();
        let (Some((binding, _)), Some(browser), Some(target)) = (
            instances.get(owner, &view.view_id),
            view.browser.as_ref(),
            instances.browser_target(owner, &view.view_id),
        ) else {
            return;
        };
        let running = self
            .app_control()
            .active_app_lease(owner, &binding.installation)
            .map(|lease| lease.catalog().generation());
        if running.is_none_or(|generation| generation <= binding.generation) {
            return;
        }
        let Ok((current, frontend)) = self.user_app_frontend(owner, &binding.installation).await
        else {
            return;
        };
        if current.generation <= binding.generation
            || !instances.set_generation(owner, &view.view_id, current.generation)
        {
            return;
        }
        let reloaded = KernelBrowserAppViewHost {
            state: self,
            owner,
            generation: Some(browser.generation),
        }
        .command(BrowserAppViewRequest::Reload {
            target_id: target,
            entry: frontend.entry,
            assets: frontend.assets,
        })
        .await;
        if reloaded.is_err() {
            // Not reloaded: stay on the old generation so the next poll retries.
            instances.set_generation(owner, &view.view_id, binding.generation);
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
        let agent = call
            .actor
            .as_ref()
            .map(|actor| self.user_app_view_agent(&owner, &view.view_id, actor))
            .transpose();
        let outcome = match &agent {
            Err(error) => Err(error.clone()),
            Ok(agent) => {
                let caller = agent
                    .as_ref()
                    .map_or(ViewCaller::Owner(None), ViewCaller::Agent);
                self.invoke_user_app_view_call(
                    &owner,
                    &view.view_id,
                    &binding,
                    &call.method,
                    call.params,
                    &mut tracked,
                    caller,
                )
                .await
            }
        };
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
