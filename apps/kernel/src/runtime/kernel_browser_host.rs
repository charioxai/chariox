//! MD-2: kernel authority for Chromium running directly on the host.
use super::browser_controller_process::{
    BrowserCancellation, BrowserControllerProcessBackend, BrowserControllerProcessState,
    BrowserControllerProcessStdioBackend,
};
use super::kernel_browser_actors::KernelBrowserActors;
pub(crate) use super::kernel_browser_actors::KernelBrowserDocumentBinding;
use super::user_domain_access::UserDomainAccess;
use crate::local::UserDomainResource;
use crate::session::{
    EnvironmentActionTerminal, EnvironmentActor, EnvironmentActorKind, TakeoverOutcome,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone)]
pub(crate) struct KernelBrowserHost {
    inner: Arc<Mutex<HostState>>,
    root: PathBuf,
}
#[derive(Default)]
struct HostState {
    browsers: BTreeMap<String, Arc<Mutex<BrowserControllerProcessStdioBackend>>>,
    stopped: bool,
    #[cfg(test)]
    after_controller_check: Option<Arc<dyn Fn() + Send + Sync>>,
    access: UserDomainAccess,
    actors: BTreeMap<String, Arc<Mutex<KernelBrowserActors>>>,
    loaded: BTreeSet<(String, String, KernelBrowserCapability)>,
    expiry_wake: Option<Arc<tokio::sync::Notify>>,
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum KernelBrowserCapability {
    Browser,
    Notes,
}

/// Internal admission bound to one user, agent and retained grant epoch.
#[derive(Clone)]
pub(crate) struct KernelBrowserAdmission {
    user: String,
    agent: Option<String>,
    agent_label: Option<String>,
    epoch: Arc<BrowserCancellation>,
    cancellation: Arc<BrowserCancellation>,
    terminal_lifetime: Option<crate::runtime::command::TerminalLifetime>,
    capability: KernelBrowserCapability,
    /// MP-08/MP-10/MP-11 A06: a live sudo window, re-evaluated by `cancellation`,
    /// stands in for focus on sensitive actions within the retained grant.
    elevated: bool,
}

impl KernelBrowserAdmission {
    /// Focus epochs have no authority callback. While holding the actor model,
    /// check only atomic revocation: callbacks may acquire the host lock, whose
    /// retirement path takes host then model. Full authority is checked outside.
    fn revoked_in_actor_lock(&self) -> bool {
        self.epoch.requested()
            || self
                .terminal_lifetime
                .as_ref()
                .is_some_and(|lifetime| !lifetime.is_live())
    }

    pub(crate) fn with_agent_label(mut self, label: &str) -> Self {
        self.agent_label = Some(label.into());
        self
    }

    pub(crate) fn with_authority(
        mut self,
        authority: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Self {
        self.cancellation = Arc::new(BrowserCancellation::for_authority(
            self.epoch.clone(),
            authority,
        ));
        self
    }

    /// Pair only with `with_authority` that checks the same sudo window.
    pub(crate) fn elevated(mut self) -> Self {
        self.elevated = true;
        self
    }
}

impl KernelBrowserHost {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HostState::default())),
            root: root.join("kernel-browser"),
        }
    }
    pub(crate) fn profile_root(&self, user: &str) -> PathBuf {
        self.root.join(Self::profile_key(user))
    }
    pub(crate) fn profile_key(user: &str) -> String {
        format!("{:x}", Sha256::digest(user.as_bytes()))
    }
    pub(crate) fn profile_keys(&self) -> Result<Vec<String>, String> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err("MD-5: cannot enumerate browser profiles".into()),
        };
        let mut keys = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|_| "MD-5: cannot enumerate browser profiles")?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.len() == 64 && name.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                keys.push(name);
            }
        }
        Ok(keys)
    }
    pub(crate) fn is_loaded(&self, user: &str, agent: &str) -> bool {
        self.is_loaded_for(user, agent, KernelBrowserCapability::Browser)
    }
    pub(crate) fn is_loaded_for(
        &self,
        user: &str,
        agent: &str,
        capability: KernelBrowserCapability,
    ) -> bool {
        let state = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        state.access.grant(user, agent).is_ok()
            && state
                .loaded
                .contains(&(user.into(), agent.into(), capability))
    }
    pub(crate) fn load(&self, user: &str, agent: &str) -> Result<(), String> {
        self.load_for(user, agent, KernelBrowserCapability::Browser)
    }
    pub(crate) fn load_for(
        &self,
        user: &str,
        agent: &str,
        capability: KernelBrowserCapability,
    ) -> Result<(), String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "MD-3: browser host lock poisoned")?;
        require_focus(&state, user, agent)?;
        state.loaded.insert((user.into(), agent.into(), capability));
        drop(state);
        crate::transport::mcp_server::catalog_changed();
        Ok(())
    }
    fn backend(
        &self,
        user: &str,
    ) -> Result<Arc<Mutex<BrowserControllerProcessStdioBackend>>, String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "MD-2: browser host lock poisoned")?;
        if state.stopped {
            return Err("MD integration: browser host is shut down".into());
        }
        if !state.browsers.contains_key(user) {
            if state.browsers.len() >= 16 {
                return Err("MD-2: browser user limit reached".into());
            }
            let script = match std::env::var_os("CHARIOX_KERNEL_BROWSER_SCRIPT") {
                Some(script) => PathBuf::from(script),
                None => super::kernel_browser_assets::materialize(&self.root)?,
            };
            let node = std::env::var_os("CHARIOX_BROWSER_CONTROLLER_NODE")
                .unwrap_or_else(|| "node".into());
            let backend = BrowserControllerProcessStdioBackend::new(
                PathBuf::from(node),
                vec![
                    script.display().to_string(),
                    "stdio".into(),
                    self.profile_root(user).display().to_string(),
                ],
                Duration::from_secs(20),
            )
            .for_host();
            state
                .browsers
                .insert(user.into(), Arc::new(Mutex::new(backend)));
        }
        Ok(state.browsers.get(user).unwrap().clone())
    }
    #[cfg(test)]
    pub(crate) fn install_fixture_backend(
        &self,
        user: &str,
        script: &std::path::Path,
        root: &std::path::Path,
    ) {
        let backend = BrowserControllerProcessStdioBackend::new(
            "/bin/sh",
            vec![script.display().to_string(), root.display().to_string()],
            Duration::from_secs(2),
        )
        .for_host();
        self.inner
            .lock()
            .unwrap()
            .browsers
            .insert(user.into(), Arc::new(Mutex::new(backend)));
    }
    fn require_running(&self) -> Result<(), String> {
        if self
            .inner
            .lock()
            .map_err(|_| "MD-2: browser host lock poisoned")?
            .stopped
        {
            Err("MD integration: browser host is shut down".into())
        } else {
            Ok(())
        }
    }
    #[cfg(test)]
    pub(crate) fn protected_request(
        &self,
        user: &str,
        agent: Option<&str>,
        method: &str,
        params: Value,
        policy: Value,
    ) -> Result<Value, String> {
        let admission = agent.map(|agent| self.admit(user, agent)).transpose()?;
        self.protected_request_admitted(user, admission.as_ref(), method, params, policy)
            .map_err(|error| error.to_string())
    }
    pub(crate) fn admit(&self, user: &str, agent: &str) -> Result<KernelBrowserAdmission, String> {
        self.admit_for(user, agent, KernelBrowserCapability::Browser)
    }
    pub(crate) fn admit_for(
        &self,
        user: &str,
        agent: &str,
        capability: KernelBrowserCapability,
    ) -> Result<KernelBrowserAdmission, String> {
        let state = self.inner.lock().map_err(|_| "MD-3: focus lock poisoned")?;
        require_loaded(&state, user, agent, capability)?;
        Ok(KernelBrowserAdmission {
            user: user.into(),
            agent: Some(agent.into()),
            agent_label: Some(agent.into()),
            capability,
            epoch: state.access.grant(user, agent)?.epoch.clone(),
            cancellation: state.access.grant(user, agent)?.epoch.clone(),
            terminal_lifetime: None,
            elevated: false,
        })
    }
    pub(crate) fn admit_terminal(
        &self,
        user: &str,
        lifetime: crate::runtime::command::TerminalLifetime,
    ) -> KernelBrowserAdmission {
        let epoch = Arc::new(BrowserCancellation::default());
        KernelBrowserAdmission {
            user: user.into(),
            agent: None,
            agent_label: None,
            epoch: epoch.clone(),
            capability: KernelBrowserCapability::Browser,
            terminal_lifetime: Some(lifetime.clone()),
            elevated: false,
            cancellation: Arc::new(BrowserCancellation::for_authority(epoch, move || {
                lifetime.is_live()
            })),
        }
    }

    pub(crate) fn check_admission(
        &self,
        admission: Option<&KernelBrowserAdmission>,
    ) -> Result<(), String> {
        if let Some(admission) = admission {
            if admission.cancellation.requested() {
                return Err("MP-11: not_granted: browser authority revoked".into());
            }
            let state = self.inner.lock().map_err(|_| "MD-3: focus lock poisoned")?;
            Self::check_admission_epoch(&state, admission)?;
        }
        Ok(())
    }
    /// MD-N4 / MP-11: a note read/write commits within one uninterrupted grant epoch.
    pub(crate) fn note_operation<T>(
        &self,
        user: &str,
        admission: Option<&KernelBrowserAdmission>,
        call: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        self.check_admission(admission)?;
        if let Some(admission) = admission {
            if admission.user != user {
                return Err("MD-N4: foreign note owner".into());
            }
            if let Some(agent) = admission.agent.as_deref() {
                let state = self
                    .inner
                    .lock()
                    .map_err(|_| "MD-N4: focus lock unavailable")?;
                require_loaded(&state, user, agent, admission.capability)?;
                if !Arc::ptr_eq(&state.access.grant(user, agent)?.epoch, &admission.epoch) {
                    return Err("MD-N4: note grant changed".into());
                }
                call()
            } else {
                call()
            }
        } else {
            call()
        }
    }
    pub(crate) fn protected_request_admitted(
        &self,
        user: &str,
        admission: Option<&KernelBrowserAdmission>,
        method: &str,
        params: Value,
        policy: Value,
    ) -> Result<Value, crate::error::HostFailure> {
        if admission.is_some_and(|admission| admission.user != user) {
            return Err("MD-3: browser admission belongs to another user".into());
        }
        self.check_admission(admission)?;
        let browser = self.backend(user)?;
        let mut backend = browser
            .lock()
            .map_err(|_| "MD-2: browser operation lock poisoned")?;
        self.require_running()?;
        self.check_admission(admission)?;
        self.scope_browser_request(admission, method, &params)?;
        if method == "host.browser" && params["op"] == "stop" {
            let model = self.actor_model(user)?;
            let (id, cancellation) = {
                let mut ledger = model.lock().map_err(|_| "MD-3: actor lock poisoned")?;
                if admission.is_some_and(|admission| admission.revoked_in_actor_lock()) {
                    return Err("MP-11: not_granted: browser authority revoked".into());
                }
                ledger.begin(browser_actor(admission, &params), &params)?
            };
            let action = BrowserActorAction {
                model,
                id,
                cancellation,
                finished: false,
            };
            self.check_admission(admission)?;
            let result = backend.stop();
            action.finish(if result.is_ok() {
                EnvironmentActionTerminal::Completed
            } else {
                EnvironmentActionTerminal::Failed
            });
            result.map_err(crate::error::HostFailure::Other)?;
            self.check_admission(admission)?;
            return Ok(serde_json::json!({ "state": "stopped", "tabs": [] }));
        }
        if params["_host_generation"].is_u64() {
            if !matches!(backend.health(), Ok(health) if health.state == BrowserControllerProcessState::Ready)
            {
                return Err("MD-APP: App host is no longer live".into());
            }
        } else if !matches!(backend.health(), Ok(health) if health.state == BrowserControllerProcessState::Ready)
        {
            // MP-11: state/observation reads cannot start or recover a
            // controller. Startup is explicit and uses the same grant for
            // focused and retained agents, without holding its lock over I/O.
            if method == "host.browser"
                && matches!(
                    params["op"].as_str(),
                    Some(
                        "state"
                            | "snapshot"
                            | "screenshot"
                            | "subscribe"
                            | "poll"
                            | "unsubscribe"
                            | "display_subscribe"
                            | "display_attach"
                            | "note_selection"
                            | "note_reanchor"
                    )
                )
            {
                return Err("MP-11: browser_unavailable: user browser controller is stopped or unavailable; explicitly start/open the browser".into());
            }
            self.check_admission(admission)?;
            backend.start().map_err(crate::error::HostFailure::Other)?;
            self.check_admission(admission)?;
        }
        let mut params = params;
        if let Some(admission) = admission.filter(|admission| admission.agent.is_some()) {
            let agent = admission.agent.as_deref().unwrap();
            params["_agent_input"] = true.into();
            params["observed_by"] = format!("agent:{agent}").into();
            let state = self
                .inner
                .lock()
                .map_err(|_| "MP-11: grant lock unavailable")?;
            Self::check_admission_epoch(&state, admission)?;
            params["_subscription_owner"] = state
                .access
                .grant(user, agent)?
                .subscription_owner
                .clone()
                .into();
        }
        self.check_admission(admission)?;
        backend.host_request_classified("host.protect", policy)?;
        self.check_admission(admission)?;
        let model = self.actor_model(user)?;
        if method == "host.browser"
            && matches!(
                params["op"].as_str(),
                Some("mirror_subscribe" | "display_subscribe")
            )
        {
            // MP-08: attach may be the first post-recovery request. Seed the same
            // tab ledger before takeover, without registering another actor.
            let state = backend
                .host_request_classified("host.browser", serde_json::json!({"op":"state"}))?;
            model
                .lock()
                .map_err(|_| "MD-3: actor lock poisoned")?
                .reconcile(&state)?;
            self.check_admission(admission)?;
        }
        let mutation = method == "host.secret"
            || (method == "host.browser"
                && matches!(
                    params["op"].as_str(),
                    Some("open" | "close" | "navigate" | "input")
                ));
        let action = if mutation {
            // Reconcile the SAME supervised browser before ledger admission. Takeover
            // uses only the model lock, so it can cancel while CDP holds the backend.
            let state = backend.host_request_cancellable(
                "host.browser",
                serde_json::json!({"op":if params["op"] == "open" { "start" } else { "state" }}),
                admission.map(|a| a.cancellation.clone()),
            )?;
            model
                .lock()
                .map_err(|_| "MD-3: actor lock poisoned")?
                .reconcile(&state)?;
            let actor = browser_actor(admission, &params);
            let (id, cancellation) = {
                // Disconnect/focus retirement takes the same model lock. Either
                // it removes a registered actor or this check prevents late registration.
                let mut ledger = model.lock().map_err(|_| "MD-3: actor lock poisoned")?;
                if admission.is_some_and(|admission| admission.revoked_in_actor_lock()) {
                    return Err("MP-11: not_granted: browser authority revoked".into());
                }
                ledger.begin(actor, &params)?
            };
            Some(BrowserActorAction {
                model: model.clone(),
                id,
                cancellation,
                finished: false,
            })
        } else {
            None
        };
        let cancellation = match (&action, admission) {
            (Some(action), Some(admission)) => {
                let authority = admission.cancellation.clone();
                Some(Arc::new(BrowserCancellation::for_authority(
                    action.cancellation.clone(),
                    move || !authority.requested(),
                )))
            }
            (Some(action), None) => Some(action.cancellation.clone()),
            (None, Some(admission)) => Some(admission.cancellation.clone()),
            (None, None) => None,
        };
        // MP-11: focused and retained input share grant/run cancellation.
        // Vault requests also carry their live focus authority from admission.
        // MP-08/MP-11: controller creation evidence names this kernel-admitted
        // action. Never accept a caller's action identity at the private seam.
        params["_action_id"] = action
            .as_ref()
            .map(|action| Value::String(action.id.clone()))
            .unwrap_or(Value::Null);
        let request_params = params.clone();
        let mut result = backend.host_request_cancellable(method, params, cancellation.clone());
        if let Some(action) = action {
            let terminal = if cancellation
                .as_ref()
                .is_some_and(|cancel| cancel.requested())
            {
                EnvironmentActionTerminal::Cancelled
            } else if result.is_ok() {
                EnvironmentActionTerminal::Completed
            } else {
                EnvironmentActionTerminal::Failed
            };
            action.finish(terminal);
        }
        self.check_admission(admission)?;
        #[cfg(test)]
        {
            let hook = self.inner.lock().unwrap().after_controller_check.take();
            if let Some(hook) = hook {
                hook();
            }
        }
        // Reconcile authority with full controller state before applying a
        // retained agent's projection; scoped inventory cannot remove another
        // actor's tabs or input ownership from the shared ledger.
        // Creation evidence belongs to the private controller seam. Never expose
        // it in the serialized client result or use it to widen capability grants.
        let tab_creation_actions = result
            .as_mut()
            .ok()
            .and_then(|state| state.as_object_mut())
            .and_then(|state| {
                state.remove("_tab_openers");
                state.remove("_tab_creation_actions")
            })
            .unwrap_or(Value::Null);
        if let Ok(state) = &result {
            let mut ledger = model.lock().map_err(|_| "MD-3: actor lock poisoned")?;
            ledger.reconcile_inventory(state, &tab_creation_actions)?;
        }
        if let Ok(payload) = &mut result {
            let mut ledger = model.lock().map_err(|_| "MD-3: actor lock poisoned")?;
            if admission.is_some_and(|a| a.revoked_in_actor_lock()) {
                return Err("MP-11: not_granted: browser authority revoked".into());
            }
            if request_params["op"] == "open" {
                if let Some(tab) = payload["tab_id"].as_str() {
                    ledger.opened_tab(browser_actor(admission, &request_params), tab);
                }
            }
            ledger.project_tabs(payload);
        }
        if let (Some(admission), Ok(payload)) = (admission, &mut result) {
            if let Some(agent) = admission.agent.as_deref() {
                let mut state = self
                    .inner
                    .lock()
                    .map_err(|_| "MP-11: grant lock unavailable")?;
                // MP-11: revoke/refocus can replace the grant while reconciliation
                // waits. Register/project only into this exact admitted epoch.
                Self::check_admission_epoch(&state, admission)?;
                if let Some(id) = payload.get("tab_id").and_then(Value::as_str) {
                    if request_params["op"] == "open" {
                        state.access.opened_tab(user, agent, id)?;
                    }
                }
                if let (Some(id), Some(tab)) = (
                    payload.get("subscription_id").and_then(Value::as_str),
                    request_params.get("tab_id").and_then(Value::as_str),
                ) {
                    state.access.subscribe(user, agent, id, tab)?;
                }
                if state.access.focused(user) != Some(agent) {
                    let grant = state.access.grant(user, agent)?;
                    if let Some(tabs) = payload.get_mut("tabs").and_then(Value::as_array_mut) {
                        tabs.retain(|tab| {
                            tab["tab_id"].as_str().is_some_and(|id| {
                                grant
                                    .resources
                                    .contains(&UserDomainResource::BrowserTab { tab_id: id.into() })
                            })
                        });
                    }
                }
                if let Some(tab) = payload["agent_activity"]["tab_id"].as_str() {
                    if state.access.focused(user) != Some(agent)
                        && !state
                            .access
                            .grant(user, agent)?
                            .resources
                            .contains(&UserDomainResource::BrowserTab { tab_id: tab.into() })
                    {
                        payload["agent_activity"] = Value::Null;
                    }
                }
                let resource = request_params["tab_id"]
                    .as_str()
                    .map(|tab| UserDomainResource::BrowserTab { tab_id: tab.into() })
                    .or_else(|| {
                        request_params["subscription_id"].as_str().and_then(|id| {
                            state
                                .access
                                .grant(user, agent)
                                .ok()?
                                .subscriptions
                                .get(id)
                                .map(|tab| UserDomainResource::BrowserTab {
                                    tab_id: tab.clone(),
                                })
                        })
                    })
                    .or_else(|| {
                        state
                            .access
                            .grant(user, agent)
                            .ok()?
                            .resources
                            .iter()
                            .next()
                            .cloned()
                    });
                if let Some(resource) = resource {
                    state.access.acted(user, agent, resource);
                }
                if request_params["op"] == "poll" {
                    if let Some(id) = request_params["subscription_id"].as_str() {
                        state.access.subscription_polled(user, agent, id);
                    }
                }
                if request_params["op"] == "unsubscribe" {
                    if let Some(id) = request_params["subscription_id"].as_str() {
                        state.access.unsubscribe(user, agent, id);
                    }
                }
            }
        }
        self.arm_expiry();
        // No grant lock across authority callbacks: they can read kernel state.
        self.check_admission(admission)?;
        result
    }
    fn actor_model(&self, user: &str) -> Result<Arc<Mutex<KernelBrowserActors>>, String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "MD-3: actor registry lock poisoned")?;
        if !state.actors.contains_key(user) && state.actors.len() >= 16 {
            return Err("MD-3: browser user limit reached".into());
        }
        Ok(state
            .actors
            .entry(user.into())
            .or_insert_with(|| Arc::new(Mutex::new(KernelBrowserActors::default())))
            .clone())
    }
    #[cfg(test)]
    pub(crate) fn request_takeover(
        &self,
        user: &str,
        actor: EnvironmentActor,
        tab: &str,
        generation: u64,
    ) -> Result<TakeoverOutcome, String> {
        self.request_takeover_admitted(user, actor, tab, generation, None)
    }
    pub(crate) fn request_takeover_admitted(
        &self,
        user: &str,
        actor: EnvironmentActor,
        tab: &str,
        generation: u64,
        admission: Option<&KernelBrowserAdmission>,
    ) -> Result<TakeoverOutcome, String> {
        self.check_admission(admission)?;
        let model = self.actor_model(user)?;
        let mut model = model.lock().map_err(|_| "MD-3: actor lock poisoned")?;
        if admission.is_some_and(|admission| admission.revoked_in_actor_lock()) {
            return Err("MP-11: not_granted: browser authority revoked".into());
        }
        model.takeover(actor, tab, generation)
    }
    pub(crate) fn release_input(
        &self,
        user: &str,
        actor_id: &str,
        tab: &str,
        generation: u64,
    ) -> Result<(), String> {
        self.actor_model(user)?
            .lock()
            .map_err(|_| "MD-3: actor lock poisoned")?
            .release(actor_id, tab, generation)
    }
    pub(crate) fn actor_snapshot(&self, user: &str) -> Result<Value, String> {
        Ok(self
            .actor_model(user)?
            .lock()
            .map_err(|_| "MD-3: actor lock poisoned")?
            .snapshot())
    }

    pub(crate) fn disconnect_terminal(&self, user: &str, actor: &str) {
        let model = self
            .inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .actors
            .get(user)
            .cloned();
        if let Some(model) = model {
            model
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .disconnect(actor);
        }
    }

    pub(crate) fn shutdown(&self) -> Result<(), String> {
        let browsers = {
            let mut state = self
                .inner
                .lock()
                .map_err(|_| "MD-2: browser host lock poisoned")?;
            state.stopped = true;
            if let Some(wake) = &state.expiry_wake {
                wake.notify_one();
            }
            let owners: BTreeSet<_> = state
                .access
                .holders()
                .into_iter()
                .map(|(owner, _)| owner)
                .collect();
            for owner in owners {
                state.access.revoke(&owner, None);
            }
            state.loaded.clear();
            std::mem::take(&mut state.browsers)
        };
        let mut first = None;
        for (_, browser) in browsers {
            let mut backend = browser
                .lock()
                .map_err(|_| "MD-2: browser operation lock poisoned")?;
            if let Err(error) = backend.stop() {
                first.get_or_insert(error);
            }
        }
        first.map_or(Ok(()), Err)
    }
}
fn browser_actor(admission: Option<&KernelBrowserAdmission>, params: &Value) -> EnvironmentActor {
    if let Some(agent) = admission.and_then(|admission| admission.agent.as_deref()) {
        EnvironmentActor::new(
            format!("agent:{agent}"),
            EnvironmentActorKind::Agent,
            admission
                .and_then(|a| a.agent_label.as_deref())
                .unwrap_or(agent),
        )
    } else {
        EnvironmentActor::new(
            params["observed_by"].as_str().unwrap_or("kernel-adapter"),
            EnvironmentActorKind::Human,
            "Human",
        )
    }
}
struct BrowserActorAction {
    model: Arc<Mutex<KernelBrowserActors>>,
    id: String,
    cancellation: Arc<BrowserCancellation>,
    finished: bool,
}
impl BrowserActorAction {
    fn finish(mut self, terminal: EnvironmentActionTerminal) {
        self.model
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .finish(&self.id, terminal);
        self.finished = true;
    }
}
impl Drop for BrowserActorAction {
    fn drop(&mut self) {
        if !self.finished {
            self.cancellation.request_cancel();
            self.model
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .finish(&self.id, EnvironmentActionTerminal::Cancelled);
        }
    }
}
fn require_focus(state: &HostState, user: &str, agent: &str) -> Result<(), String> {
    if state.access.focused(user) == Some(agent) {
        Ok(())
    } else {
        Err("MP-08: not_focused_agent: user-domain browser access requires current local agent focus; ask the user to focus this agent".into())
    }
}

fn require_loaded(
    state: &HostState,
    user: &str,
    agent: &str,
    capability: KernelBrowserCapability,
) -> Result<(), String> {
    state.access.grant(user, agent)?;
    if state
        .loaded
        .contains(&(user.into(), agent.into(), capability))
    {
        Ok(())
    } else {
        Err("MD-N4: load the requested user-domain tools first".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// MP-08/MP-11 A05: absolute expiry is a live wake. With no further call,
    /// the grant disappears, its epoch cancels in-flight work and clients see it.
    #[tokio::test]
    async fn capability_grant_expiry_is_a_live_wake() {
        let host = KernelBrowserHost::new(PathBuf::from("/unused/capability-expiry"));
        host.request_grant(
            "owner",
            "agent",
            "prompt",
            Duration::from_millis(200),
            None,
            "session",
        )
        .unwrap();
        host.set_focus("owner", Some("focused"));
        let admission = host.admit("owner", "agent").unwrap();
        let cursor = host.grant_snapshot("owner", "kernel")["cursor"].as_u64();
        tokio::time::timeout(Duration::from_secs(3), async {
            while host
                .grant_holders()
                .contains(&("owner".into(), "agent".into()))
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the expiry wake revokes without any later request");
        assert!(admission.cancellation.requested());
        assert!(host.grant_snapshot("owner", "kernel")["cursor"].as_u64() > cursor);
        assert!(
            host.has_grant("owner", "focused"),
            "unexpired grants remain"
        );
        assert!(KernelBrowserHost::grant_lifetime(Some(25)).is_err());
        assert!(KernelBrowserHost::grant_lifetime(Some(0)).is_err());
        assert_eq!(
            KernelBrowserHost::grant_lifetime(Some(24)).unwrap(),
            Duration::from_secs(24 * 3600)
        );
    }
    #[tokio::test]
    async fn capability_empty_revoke_invalidates_a_pending_browser_acquisition() {
        let host = KernelBrowserHost::new(PathBuf::from("/unused/capability-revoke"));
        let fence = host.acquisition_fence("owner", "agent");
        host.revoke_grants("owner", None);
        assert!(
            host.request_grant(
                "owner",
                "agent",
                "request",
                Duration::from_secs(60),
                Some(fence),
                "room"
            )
            .is_err(),
            "MP-11: a revoke must fence pending acquisition before any holder exists"
        );
        assert!(host.grant_holders().is_empty());
    }
    /// MP-11 (#922 review 1): only a revoke that reaches this agent fences
    /// its pending acquisition; other holders' grant changes do not.
    #[tokio::test]
    async fn capability_unrelated_grant_changes_keep_a_pending_browser_acquisition() {
        let host = KernelBrowserHost::new(PathBuf::from("/unused/capability-fence"));
        let lifetime = Duration::from_secs(60);
        host.request_grant("owner", "busy", "prompt", lifetime, None, "room")
            .unwrap();
        let fence = host.acquisition_fence("owner", "agent");
        host.set_focus("owner", Some("focused"));
        host.load("owner", "focused").unwrap();
        host.bind_activity("owner", "busy", "room", false);
        host.bind_activity("owner", "busy", "room", true);
        host.revoke_grants("owner", Some("focused"));
        host.request_grant("other-owner", "agent", "prompt", lifetime, None, "room")
            .unwrap();
        assert!(host
            .request_grant("owner", "agent", "request", lifetime, Some(fence), "room")
            .unwrap());
        let fence = host.acquisition_fence("owner", "child");
        host.revoke_grants("owner", Some("child"));
        assert!(host
            .request_grant("owner", "child", "request", lifetime, Some(fence), "room")
            .is_err());
    }
    #[test]
    fn mdaccess_focus_switch_keeps_existing_grant_and_admission() {
        let host = KernelBrowserHost::new(PathBuf::from("/unused/mdaccess"));
        host.set_focus("owner", Some("waiting-agent"));
        host.load("owner", "waiting-agent").unwrap();
        let admission = host.admit("owner", "waiting-agent").unwrap();
        host.set_focus("owner", Some("second-agent"));
        assert!(host.is_loaded("owner", "waiting-agent"));
        assert!(host.check_admission(Some(&admission)).is_ok());
        host.revoke_agent("waiting-agent");
        assert!(host.check_admission(Some(&admission)).is_err());
    }
    #[test]
    fn notes_loader_is_independent_and_revoked_admissions_cannot_commit() {
        let host = KernelBrowserHost::new(PathBuf::from("/unused/mdnotes-focus"));
        host.set_focus("a", Some("first"));
        host.load_for("a", "first", KernelBrowserCapability::Notes)
            .unwrap();
        assert!(!host.is_loaded("a", "first"));
        let notes = host
            .admit_for("a", "first", KernelBrowserCapability::Notes)
            .unwrap();
        assert!(host
            .note_operation::<()>("other", Some(&notes), || panic!("foreign commit"))
            .is_err());
        host.load("a", "first").unwrap();
        host.revoke_agent("first");
        host.set_focus("a", Some("first"));
        host.load_for("a", "first", KernelBrowserCapability::Notes)
            .unwrap();
        assert!(host
            .note_operation::<()>("a", Some(&notes), || panic!("revoked commit"))
            .is_err());
        assert!(!host.is_loaded("a", "first"));
        let fresh = host
            .admit_for("a", "first", KernelBrowserCapability::Notes)
            .unwrap();
        assert_eq!(
            host.note_operation("a", Some(&fresh), || Ok(42)).unwrap(),
            42
        );
    }
    #[test]
    fn retained_host_cannot_restart_after_shutdown() {
        let host = KernelBrowserHost::new(PathBuf::from("/tmp/mdint-stopped-host"));
        let retained = host.clone();
        host.shutdown().unwrap();
        assert!(retained
            .protected_request(
                "alice",
                None,
                "host.browser",
                serde_json::json!({"op":"start"}),
                Value::Null
            )
            .is_err());
    }
    #[test]
    fn profile_selection_is_private_stable_and_user_separated() {
        let host = KernelBrowserHost::new(PathBuf::from("/tmp/md2-state"));
        assert_eq!(
            host.profile_root("user/../../a"),
            host.profile_root("user/../../a")
        );
        assert_ne!(host.profile_root("a"), host.profile_root("b"));
        assert_eq!(
            host.profile_root("user/../../a").parent(),
            Some(host.root.as_path())
        );
    }
    #[test]
    fn admission_cannot_revive_after_revocation_and_tools_reload() {
        let host = KernelBrowserHost::new(PathBuf::from("/unused/md3-focus-epoch"));
        host.set_focus("a", Some("agent1"));
        host.load("a", "agent1").unwrap();
        let old = host.admit("a", "agent1").unwrap();
        host.revoke_agent("agent1");
        host.set_focus("a", Some("agent1"));
        host.load("a", "agent1").unwrap();
        assert!(host.check_admission(Some(&old)).is_err());
        assert!(host
            .protected_request_admitted(
                "a",
                Some(&old),
                "host.browser",
                serde_json::json!({"op":"stop"}),
                Value::Null
            )
            .is_err());
        assert!(host.inner.lock().unwrap().browsers.is_empty());
        let current = host.admit("a", "agent1").unwrap();
        assert!(host.check_admission(Some(&current)).is_ok());
        host.revoke_agent("agent1");
        assert!(host.check_admission(Some(&current)).is_err());
    }
    #[test]
    fn admission_survives_same_focus_and_another_users_focus_change() {
        let host = KernelBrowserHost::new(PathBuf::from("/unused/md3-focus-users"));
        host.set_focus("a", Some("agent1"));
        host.load("a", "agent1").unwrap();
        let admission = host.admit("a", "agent1").unwrap();
        host.set_focus("a", Some("agent1"));
        host.set_focus("b", Some("agent2"));
        assert!(host.check_admission(Some(&admission)).is_ok());
        assert!(host.admit("b", "agent1").is_err());
        assert!(host.admit("b", "agent2").is_err());
    }
    #[test]
    fn revocation_requests_cancellation_and_is_scoped_to_the_agent() {
        let host = KernelBrowserHost::new(PathBuf::from("/unused/md3-cancellation"));
        host.set_focus("a", Some("agent1"));
        host.load("a", "agent1").unwrap();
        host.set_focus("b", Some("agent2"));
        host.load("b", "agent2").unwrap();
        let first = host.admit("a", "agent1").unwrap();
        let second = host.admit("b", "agent2").unwrap();
        host.revoke_agent("agent1");
        assert!(first.cancellation.requested());
        assert!(!second.cancellation.requested());
        assert!(host.check_admission(Some(&second)).is_ok());
        host.revoke_agent("agent2");
        assert!(second.cancellation.requested());
    }
    #[test]
    fn authority_revocation_cancels_without_a_focus_change() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let host = KernelBrowserHost::new(PathBuf::from("/unused/md3-authority"));
        host.set_focus("a", Some("agent1"));
        host.load("a", "agent1").unwrap();
        let valid = Arc::new(AtomicBool::new(true));
        let check = valid.clone();
        let admission = host
            .admit("a", "agent1")
            .unwrap()
            .with_authority(move || check.load(Ordering::Acquire));
        assert!(host.check_admission(Some(&admission)).is_ok());
        valid.store(false, Ordering::Release);
        assert!(admission.cancellation.requested());
        assert!(host.check_admission(Some(&admission)).is_err());
    }
}

#[cfg(test)]
mod actor_tests;

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod native_input_tests;

mod access;
#[cfg(test)]
mod tabs_tests;
