//! An owner's agents use the owner's kernel browser, Computer and Apps with no
//! grant, focus, lifetime or delegation. The host only remembers, per agent,
//! the fence that cancels its in-flight calls when its run ends, and the
//! subscription scope the controller retires with it.
use super::*;

/// What the host keeps for one running agent of an owner.
pub(super) struct AgentPresence {
    pub(super) fence: Arc<BrowserCancellation>,
    pub(super) scope: String,
}

impl AgentPresence {
    fn new() -> Self {
        Self {
            fence: Arc::new(BrowserCancellation::default()),
            scope: format!("agent-{:032x}", rand::random::<u128>()),
        }
    }
}

impl KernelBrowserHost {
    /// The agent lists the browser or Computer tools. This only changes the
    /// catalog; calls are never refused for not having loaded.
    pub(crate) fn load_for(&self, user: &str, agent: &str, capability: KernelBrowserCapability) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.loaded.insert((user.into(), agent.into(), capability));
        drop(state);
        crate::transport::mcp_server::catalog_changed();
    }
    pub(crate) fn is_loaded_for(
        &self,
        user: &str,
        agent: &str,
        capability: KernelBrowserCapability,
    ) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .loaded
            .contains(&(user.into(), agent.into(), capability))
    }
    pub(crate) fn is_loaded(&self, user: &str, agent: &str) -> bool {
        self.is_loaded_for(user, agent, KernelBrowserCapability::Browser)
    }
    /// Fails only when the browser host is shut down.
    pub(crate) fn admit(&self, user: &str, agent: &str) -> Result<KernelBrowserAdmission, String> {
        self.admit_for(user, agent, KernelBrowserCapability::Browser)
    }
    pub(crate) fn admit_for(
        &self,
        user: &str,
        agent: &str,
        capability: KernelBrowserCapability,
    ) -> Result<KernelBrowserAdmission, String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "MD-3: browser host lock poisoned")?;
        let presence = state
            .presence
            .entry((user.into(), agent.into()))
            .or_insert_with(AgentPresence::new);
        let fence = presence.fence.clone();
        Ok(KernelBrowserAdmission {
            user: user.into(),
            agent: Some(agent.into()),
            agent_label: None,
            capability,
            epoch: fence.clone(),
            cancellation: fence,
            terminal_lifetime: None,
        })
    }
    /// The fence the next call of this agent runs under.
    pub(crate) fn run_fence(&self, user: &str, agent: &str) -> AgentRunFence {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        AgentRunFence(
            state
                .presence
                .entry((user.into(), agent.into()))
                .or_insert_with(AgentPresence::new)
                .fence
                .clone(),
        )
    }
    /// An agent names a Tab or starts, stops or opens; it never names a stream, which only the
    /// person's viewer owns.
    pub(super) fn scope_browser_request(
        &self,
        admission: Option<&KernelBrowserAdmission>,
        params: &Value,
    ) -> Result<(), String> {
        if admission.is_none_or(|admission| admission.agent.is_none()) {
            return Ok(());
        }
        if params["subscription_id"].is_string() {
            return Err("MP-11: foreign browser subscription".into());
        }
        if !params["tab_id"].is_string()
            && !matches!(
                params["op"].as_str(),
                Some("state" | "start" | "stop" | "restart" | "open")
            )
        {
            return Err("MP-11: a browser request must name a tab".into());
        }
        Ok(())
    }
    pub(super) fn subscription_scope(state: &mut HostState, user: &str, agent: &str) -> String {
        state
            .presence
            .entry((user.into(), agent.into()))
            .or_insert_with(AgentPresence::new)
            .scope
            .clone()
    }
    /// The agent's run ended (agent removed, session ended, run replaced):
    /// cancel its in-flight calls, retire its subscriptions and its listing.
    pub(crate) fn revoke_agent(&self, agent: &str) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let keys: Vec<_> = state
            .presence
            .keys()
            .filter(|(_, id)| id == agent)
            .cloned()
            .collect();
        let mut retired = Vec::new();
        for key in keys {
            if let Some(presence) = state.presence.remove(&key) {
                presence.fence.request_cancel();
                retired.push((state.browsers.get(&key.0).cloned(), presence.scope));
            }
        }
        state.loaded.retain(|(_, id, _)| id != agent);
        let models: Vec<_> = state.actors.values().cloned().collect();
        drop(state);
        // Take the actor models after releasing the host lock: authority callbacks hold a
        // model while they read the host.
        for model in models {
            model
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .disconnect(&format!("agent:{agent}"));
        }
        for (backend, scope) in retired {
            Self::retire_scopes(backend, vec![scope]);
        }
        crate::transport::mcp_server::catalog_changed();
    }
    /// Agents of every owner that currently hold a presence.
    pub(crate) fn running_agents(&self) -> Vec<(String, String)> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .presence
            .keys()
            .cloned()
            .collect()
    }
    /// What a hot reload hands over: the tools each agent listed.
    #[cfg(unix)]
    pub(crate) fn hand_over_loaded(&self) -> LoadedHandover {
        LoadedHandover(
            self.inner
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .loaded
                .iter()
                .cloned()
                .collect(),
        )
    }
    #[cfg(unix)]
    pub(crate) fn adopt_loaded(&self, handover: LoadedHandover) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .loaded
            .extend(handover.0);
        crate::transport::mcp_server::catalog_changed();
    }
    pub(super) fn retire_scopes(
        backend: Option<Arc<Mutex<BrowserControllerProcessStdioBackend>>>,
        scopes: Vec<String>,
    ) {
        if scopes.is_empty() {
            return;
        }
        if let Some(backend) = backend {
            std::thread::spawn(move || {
                if let Ok(mut backend) = backend.lock() {
                    // Internal cleanup never starts/restarts a browser or observes pixels.
                    let _ = backend.host_request(
                        "host.revoke_subscriptions",
                        serde_json::json!({"subscription_owners":scopes}),
                    );
                }
            });
        }
    }
    /// The Room Computer switches of `user` changed.
    pub(crate) fn access_projection_changed(&self, user: &str) {
        *self
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .access_cursors
            .entry(user.into())
            .or_default() += 1;
    }
    pub(crate) fn access_cursor(&self, user: &str) -> u64 {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .access_cursors
            .get(user)
            .copied()
            .unwrap_or(0)
    }
    /// A cancelled run fence fails the call it admitted.
    pub(crate) fn check_admission(
        &self,
        admission: Option<&KernelBrowserAdmission>,
    ) -> Result<(), String> {
        match admission {
            Some(admission) if admission.cancellation.requested() => {
                Err("The browser call was cancelled because the agent's run ended".into())
            }
            _ => Ok(()),
        }
    }
}

/// The user-domain tools each agent listed, carried across a hot reload.
#[cfg(unix)]
#[derive(Debug, Default, serde::Serialize)]
pub(crate) struct LoadedHandover(Vec<(String, String, KernelBrowserCapability)>);

/// L12.1 handed over `{focus, grants, loaded}`; focus and grants ended with the
/// no-grant kernel browser, so a handover from that release keeps only `loaded`.
#[cfg(unix)]
impl<'de> serde::Deserialize<'de> for LoadedHandover {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Shape {
            Loaded(Vec<(String, String, KernelBrowserCapability)>),
            Previous {
                loaded: Vec<(String, String, KernelBrowserCapability)>,
            },
        }
        Ok(match Shape::deserialize(deserializer)? {
            Shape::Loaded(loaded) | Shape::Previous { loaded } => Self(loaded),
        })
    }
}
