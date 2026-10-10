//! MP-08/MP-11: host wiring for grants, scope checks and subscription retirement.
use super::*;
use std::time::Instant;

impl KernelBrowserHost {
    // MP-11: use while holding the grant lock; never run authority callbacks here.
    pub(super) fn check_admission_epoch(
        state: &HostState,
        admission: &KernelBrowserAdmission,
    ) -> Result<(), String> {
        if let Some(agent) = admission.agent.as_deref() {
            require_loaded(state, &admission.user, agent, admission.capability)?;
            if !Arc::ptr_eq(
                &state.access.grant(&admission.user, agent)?.epoch,
                &admission.epoch,
            ) {
                return Err(
                    "MP-11: not_granted: browser grant changed; request fresh tools".into(),
                );
            }
        }
        Ok(())
    }

    pub(crate) fn set_focus(&self, user: &str, agent: Option<&str>) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .access
            .focus(user, agent);
        crate::transport::mcp_server::catalog_changed();
    }
    pub(crate) fn focused_agent(&self, user: &str) -> Option<String> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .access
            .focused(user)
            .map(str::to_owned)
    }
    pub(crate) fn grant_holders(&self) -> Vec<(String, String)> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .access
            .holders()
    }
    pub(crate) fn has_grant(&self, user: &str, agent: &str) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .access
            .grant(user, agent)
            .is_ok()
    }
    pub(crate) fn idle_window() -> Duration {
        Duration::from_secs(
            std::env::var("CHARIOX_USER_DOMAIN_IDLE_TIMEOUT_SECONDS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|value| *value > 0)
                .unwrap_or(1800),
        )
    }
    pub(crate) fn bind_activity(&self, user: &str, agent: &str, session: &str, busy: bool) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let subscriptions = state
            .access
            .grant(user, agent)
            .map(|g| g.subscriptions.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let scopes = state
            .access
            .grant(user, agent)
            .map(|g| vec![g.subscription_owner.clone()])
            .unwrap_or_default();
        if state.access.bind(
            user,
            agent,
            session,
            busy,
            (Instant::now(), crate::session::unix_epoch_ms()),
            Self::idle_window(),
        ) {
            state
                .loaded
                .retain(|(owner, id, _)| owner != user || id != agent);
            let model = state.actors.get(user).cloned();
            let backend = state.browsers.get(user).cloned();
            drop(state);
            self.retire_actors(user, &[agent.to_string()], model);
            Self::cancel_subscriptions(backend, subscriptions, scopes);
            crate::transport::mcp_server::catalog_changed();
        }
    }
    // MP-08 / MP-10 / MP-11: Room permissions share the Access cursor.
    pub(crate) fn access_projection_changed(&self, user: &str) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .access
            .changed(user);
    }
    pub(crate) fn grant_snapshot(&self, user: &str, kernel: &str) -> Value {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .access
            .snapshot(user, kernel, Self::idle_window())
    }
    pub(crate) fn revoke_grants(&self, user: &str, agent: Option<&str>) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let holders = state.access.holders();
        let scopes = holders
            .iter()
            .filter(|(owner, id)| owner == user && agent.is_none_or(|agent| id == agent))
            .filter_map(|(owner, id)| {
                state
                    .access
                    .grant(owner, id)
                    .ok()
                    .map(|grant| grant.subscription_owner.clone())
            })
            .collect();
        let subscriptions = state.access.revoke(user, agent);
        state
            .loaded
            .retain(|(owner, id, _)| owner != user || agent.is_some_and(|agent| id != agent));
        let model = state.actors.get(user).cloned();
        let backend = state.browsers.get(user).cloned();
        let retired = holders
            .into_iter()
            .filter(|(owner, id)| owner == user && agent.is_none_or(|agent| id == agent))
            .map(|(_, id)| id)
            .collect::<Vec<_>>();
        drop(state);
        self.retire_actors(user, &retired, model);
        Self::cancel_subscriptions(backend, subscriptions, scopes);
        crate::transport::mcp_server::catalog_changed();
    }
    fn retire_actors(
        &self,
        user: &str,
        agents: &[String],
        model: Option<Arc<Mutex<KernelBrowserActors>>>,
    ) {
        if let Some(model) = model {
            // Authority callbacks can hold the actor lock while reading grants.
            // Match that lock order; never wait for actors while holding grants.
            let mut model = model.lock().unwrap_or_else(|e| e.into_inner());
            let state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            for agent in agents {
                // Refocus may have created a fresh epoch during retirement. The
                // cancelled old epoch still fences its work; preserve new presence.
                if state.access.grant(user, agent).is_err() {
                    model.disconnect(&format!("agent:{agent}"));
                }
            }
        }
    }
    fn cancel_subscriptions(
        backend: Option<Arc<Mutex<BrowserControllerProcessStdioBackend>>>,
        subscriptions: Vec<String>,
        scopes: Vec<String>,
    ) {
        if subscriptions.is_empty() && scopes.is_empty() {
            return;
        }
        if let Some(backend) = backend {
            std::thread::spawn(move || {
                if let Ok(mut backend) = backend.lock() {
                    // Internal cleanup never starts/restarts a browser or observes pixels.
                    let _ = backend.host_request("host.revoke_subscriptions", serde_json::json!({"subscription_ids":subscriptions,"subscription_owners":scopes}));
                }
            });
        }
    }
    pub(crate) fn revoke_agent(&self, agent: &str) {
        for (user, id) in self.grant_holders() {
            if id == agent {
                self.revoke_grants(&user, Some(agent));
            }
        }
    }
    pub(crate) fn is_focused(&self, user: &str, agent: &str) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .access
            .focused(user)
            == Some(agent)
    }
    pub(crate) fn claim_resource(
        &self,
        admission: Option<&KernelBrowserAdmission>,
        resource: UserDomainResource,
        sensitive: bool,
    ) -> Result<(), String> {
        self.check_admission(admission)?;
        if let Some(admission) = admission {
            if let Some(agent) = admission.agent.as_deref() {
                let mut state = self
                    .inner
                    .lock()
                    .map_err(|_| "MP-11: grant lock unavailable")?;
                Self::check_admission_epoch(&state, admission)?;
                state
                    .access
                    .claim(&admission.user, agent, resource, sensitive)?;
            }
        }
        Ok(())
    }
    pub(crate) fn resource_visible(
        &self,
        admission: Option<&KernelBrowserAdmission>,
        resource: &UserDomainResource,
    ) -> bool {
        let Some(admission) = admission else {
            return true;
        };
        let Some(agent) = admission.agent.as_deref() else {
            return true;
        };
        let state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.access.focused(&admission.user) == Some(agent)
            || state
                .access
                .grant(&admission.user, agent)
                .is_ok_and(|grant| grant.resources.contains(resource))
    }
    pub(crate) fn resource_notice(
        &self,
        admission: Option<&KernelBrowserAdmission>,
        resource: UserDomainResource,
    ) {
        if let Some(admission) = admission {
            if let Some(agent) = admission.agent.as_deref() {
                self.inner
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .access
                    .acted(&admission.user, agent, resource);
            }
        }
    }
    pub(super) fn scope_browser_request(
        &self,
        admission: Option<&KernelBrowserAdmission>,
        method: &str,
        params: &Value,
    ) -> Result<(), String> {
        let Some(admission) = admission else {
            return Ok(());
        };
        let Some(agent) = admission.agent.as_deref() else {
            return Ok(());
        };
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "MP-11: grant lock unavailable")?;
        Self::check_admission_epoch(&state, admission)?;
        // MP-11: Stop affects the entire owner browser, including unrelated
        // tabs and App views. A retained resource grant cannot authorize it,
        // even if the request supplies an owned tab_id or subscription_id.
        if method == "host.browser" && params["op"] == "stop" {
            require_focus(&state, &admission.user, agent)?;
        }
        state
            .access
            .prune_subscriptions(&admission.user, agent, Instant::now());
        if params["op"] == "open"
            && state.access.grant(&admission.user, agent)?.resources.len() >= 1024
        {
            return Err("MP-08: user-domain resource limit reached; revoke unused grants".into());
        }
        if params["op"] == "subscribe"
            && state
                .access
                .grant(&admission.user, agent)?
                .subscriptions
                .len()
                >= 16
        {
            return Err("MP-08: user-domain subscription limit reached".into());
        }
        if let Some(id) = params["subscription_id"].as_str() {
            let tab = state
                .access
                .grant(&admission.user, agent)?
                .subscriptions
                .get(id)
                .cloned()
                .ok_or("MP-11: foreign or revoked browser subscription")?;
            state.access.claim(
                &admission.user,
                agent,
                UserDomainResource::BrowserTab { tab_id: tab },
                false,
            )?;
        } else if let Some(tab) = params["tab_id"].as_str() {
            state.access.claim(
                &admission.user,
                agent,
                UserDomainResource::BrowserTab { tab_id: tab.into() },
                method == "host.secret",
            )?;
        } else if !matches!(
            params["op"].as_str(),
            Some("state" | "start" | "stop" | "open")
        ) {
            return Err("MP-11: user-domain browser request requires a scoped resource".into());
        }
        Ok(())
    }
}
