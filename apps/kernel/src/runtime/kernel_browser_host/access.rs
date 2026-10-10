//! MP-08/MP-11: host wiring for grants, scope checks and subscription retirement.
use super::*;
use crate::runtime::user_domain_access::{DEFAULT_GRANT_LIFETIME, MAX_GRANT_LIFETIME};
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

    // MP-11: a replacement must close the old epoch's subscriptions, including
    // delegated descendants. Cleanup retains exact scope IDs across refocus.
    fn update_grant<T>(
        &self,
        user: &str,
        agent: Option<&str>,
        update: impl FnOnce(&mut HostState) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "MD-3: grant lock unavailable")?;
        let retired = agent
            .map(|id| state.access.retirement_snapshot(user, id))
            .unwrap_or_default();
        let result = update(&mut state);
        let retired: Vec<_> = retired
            .into_iter()
            .filter(|old| old.epoch.requested())
            .collect();
        for old in &retired {
            if state.access.grant(user, &old.agent).is_err() {
                state
                    .loaded
                    .retain(|(owner, id, _)| owner != user || id != &old.agent);
            }
        }
        let model = state.actors.get(user).cloned();
        let backend = state.browsers.get(user).cloned();
        drop(state);
        let agents = retired
            .iter()
            .map(|old| old.agent.clone())
            .collect::<Vec<_>>();
        let subscriptions = retired
            .iter()
            .flat_map(|old| old.subscriptions.iter().cloned())
            .collect();
        let scopes = retired.into_iter().map(|old| old.scope).collect();
        self.retire_actors(user, &agents, model);
        Self::cancel_subscriptions(backend, subscriptions, scopes);
        result
    }

    pub(crate) fn set_focus(&self, user: &str, agent: Option<&str>) {
        let _ = self.update_grant(user, agent, |state| {
            state.access.focus(user, agent);
            Ok(())
        });
        self.arm_expiry();
        crate::transport::mcp_server::catalog_changed();
    }
    /// MP-08/MP-11: lifetime of a new grant; an operator override is bounded
    /// by the 24-hour maximum.
    pub(crate) fn grant_lifetime(requested_hours: Option<u64>) -> Result<Duration, String> {
        if let Some(hours) = requested_hours {
            if !(1..=24).contains(&hours) {
                return Err("MP-08: grant lifetime must be 1 to 24 hours".into());
            }
            return Ok(Duration::from_secs(hours * 3600));
        }
        Ok(std::env::var("CHARIOX_USER_DOMAIN_GRANT_LIFETIME_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_GRANT_LIFETIME)
            .min(MAX_GRANT_LIFETIME))
    }
    /// MP-08/MP-11: an owner-requested turn grants and loads browser tools
    /// without focus. Returns whether a new grant was created.
    pub(crate) fn request_grant(
        &self,
        user: &str,
        agent: &str,
        prompt_id: &str,
        lifetime: Duration,
        expected_fence: Option<u64>,
        session: &str,
    ) -> Result<bool, String> {
        tokio::runtime::Handle::try_current()
            .map_err(|_| "MP-11: a live runtime is required for grant expiry")?;
        let created = self.update_grant(user, Some(agent), |state| {
            if state.stopped
                || expected_fence
                    .is_some_and(|fence| state.access.acquisition_fence(user, agent) != fence)
            {
                return Err(
                    "MP-11: not_granted: grant state changed while acquisition was pending".into(),
                );
            }
            let created = state.access.request(user, agent, prompt_id, lifetime);
            state.access.bind(
                user,
                agent,
                session,
                true,
                (Instant::now(), crate::session::unix_epoch_ms()),
                Self::idle_window(),
            );
            state
                .loaded
                .insert((user.into(), agent.into(), KernelBrowserCapability::Browser));
            Ok(created)
        })?;
        self.arm_expiry();
        crate::transport::mcp_server::catalog_changed();
        Ok(created)
    }
    /// MP-08/MP-11: explicit subset transfer to a direct child (checked by the caller).
    pub(crate) fn transfer_grant(
        &self,
        admission: &KernelBrowserAdmission,
        child: &str,
        resources: &[UserDomainResource],
    ) -> Result<(), String> {
        self.check_admission(Some(admission))?;
        let parent = admission
            .agent
            .as_deref()
            .ok_or("MP-08: admitted agent required")?;
        self.update_grant(&admission.user, Some(child), |state| {
            Self::check_admission_epoch(state, admission)?;
            state
                .access
                .transfer(&admission.user, parent, child, resources)?;
            let user = admission.user.clone();
            state
                .loaded
                .insert((user.clone(), child.into(), KernelBrowserCapability::Browser));
            if resources
                .iter()
                .any(|resource| matches!(resource, UserDomainResource::Note { .. }))
            {
                state
                    .loaded
                    .insert((user, child.into(), KernelBrowserCapability::Notes));
            }
            Ok(())
        })?;
        self.arm_expiry();
        crate::transport::mcp_server::catalog_changed();
        Ok(())
    }
    /// MP-08/MP-11: one live wake per host revokes grants at their absolute
    /// expiry, including idle subscribers and in-flight calls (epoch cancel).
    pub(super) fn arm_expiry(&self) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(wake) = state.expiry_wake.as_ref() {
            wake.notify_one();
            return;
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let wake = Arc::new(tokio::sync::Notify::new());
        state.expiry_wake = Some(wake.clone());
        drop(state);
        let inner = Arc::downgrade(&self.inner);
        let root = self.root.clone();
        handle.spawn(async move {
            loop {
                let Some(inner) = inner.upgrade() else {
                    return;
                };
                let host = KernelBrowserHost {
                    inner,
                    root: root.clone(),
                };
                if host.inner.lock().unwrap_or_else(|e| e.into_inner()).stopped {
                    return;
                }
                let next = host.expire_due();
                drop(host);
                match next {
                    Some(deadline) => {
                        tokio::select! {
                            _ = tokio::time::sleep_until(deadline.into()) => {}
                            _ = wake.notified() => {}
                        }
                    }
                    None => wake.notified().await,
                }
            }
        });
    }
    /// Revokes every expired grant; returns the next expiry still pending.
    pub(crate) fn expire_due(&self) -> Option<Instant> {
        let due = {
            let state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            state
                .access
                .due_retention(Instant::now(), Self::idle_window())
                .into_iter()
                .filter_map(|(user, agent)| {
                    state
                        .access
                        .raw_epoch(&user, &agent)
                        .map(|epoch| (user, agent, epoch))
                })
                .collect::<Vec<_>>()
        };
        for (user, agent, epoch) in due {
            self.revoke_grants_at_epoch(&user, Some(&agent), Some(epoch));
        }
        let expired = {
            let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            state
                .access
                .expired_subscriptions(Instant::now())
                .into_iter()
                .map(|(user, ids)| (state.browsers.get(&user).cloned(), ids))
                .collect::<Vec<_>>()
        };
        for (backend, ids) in expired {
            Self::cancel_subscriptions(backend, ids, Vec::new());
        }
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .access
            .next_expiry(Self::idle_window())
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
        let holders = state.access.revocation_set(user, Some(agent));
        let subscriptions = holders
            .iter()
            .flat_map(|(owner, id)| {
                state
                    .access
                    .grant(owner, id)
                    .map(|grant| grant.subscriptions.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default()
            })
            .collect();
        let scopes = holders
            .iter()
            .filter_map(|(owner, id)| state.access.subscription_owner(owner, id))
            .collect();
        if state.access.bind(
            user,
            agent,
            session,
            busy,
            (Instant::now(), crate::session::unix_epoch_ms()),
            Self::idle_window(),
        ) {
            state.loaded.retain(|(owner, id, _)| {
                owner != user || !holders.iter().any(|(_, held)| held == id)
            });
            let model = state.actors.get(user).cloned();
            let backend = state.browsers.get(user).cloned();
            drop(state);
            self.retire_actors(
                user,
                &holders.into_iter().map(|(_, id)| id).collect::<Vec<_>>(),
                model,
            );
            Self::cancel_subscriptions(backend, subscriptions, scopes);
            crate::transport::mcp_server::catalog_changed();
        } else {
            drop(state);
        }
        self.arm_expiry();
    }
    pub(crate) fn acquisition_fence(&self, user: &str, agent: &str) -> u64 {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .access
            .acquisition_fence(user, agent)
    }
    pub(crate) fn grant_snapshot(&self, user: &str, kernel: &str) -> Value {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .access
            .snapshot(user, kernel, Self::idle_window())
    }
    pub(crate) fn revoke_grants(&self, user: &str, agent: Option<&str>) {
        self.revoke_grants_at_epoch(user, agent, None);
    }
    fn revoke_grants_at_epoch(
        &self,
        user: &str,
        agent: Option<&str>,
        expected: Option<Arc<BrowserCancellation>>,
    ) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if expected.is_some_and(|epoch| {
            agent
                .and_then(|agent| state.access.raw_epoch(user, agent))
                .is_none_or(|current| !Arc::ptr_eq(&epoch, &current))
                || !state
                    .access
                    .due_retention(Instant::now(), Self::idle_window())
                    .iter()
                    .any(|(owner, id)| owner == user && Some(id.as_str()) == agent)
        }) {
            return;
        }
        // Delegated child grants are revoked with their parent.
        let holders = state.access.revocation_set(user, agent);
        let scopes = holders
            .iter()
            .filter_map(|(owner, id)| state.access.subscription_owner(owner, id))
            .collect();
        let subscriptions = state.access.revoke(user, agent);
        state
            .loaded
            .retain(|(owner, id, _)| owner != user || !holders.iter().any(|(_, held)| held == id));
        let model = state.actors.get(user).cloned();
        let backend = state.browsers.get(user).cloned();
        let retired = holders.into_iter().map(|(_, id)| id).collect::<Vec<_>>();
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
        state
            .access
            .prune_subscriptions(&admission.user, agent, Instant::now());
        // MP-08/MP-11 SB-02: Stop shuts down the owner's entire browser,
        // including unrelated tabs/displays. New-tabs approval cannot grant it.
        if params["op"] == "stop" && state.access.focused(&admission.user) != Some(agent) {
            return Err("MD-3: not_focused_agent: browser stop requires live owner focus".into());
        }
        // MP-08/MP-11: a delegated subset never widens: no new tabs or lifecycle.
        if matches!(
            state.access.grant(&admission.user, agent)?.cause,
            crate::runtime::user_domain_access::GrantCause::Delegated(_)
        ) && state.access.focused(&admission.user) != Some(agent)
            && matches!(params["op"].as_str(), Some("open" | "start" | "stop"))
        {
            return Err(crate::runtime::user_domain_access::NOT_REQUESTED_OPEN.into());
        }
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
                method == "host.secret" && !admission.elevated,
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
