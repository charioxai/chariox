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
    mutation_lanes: BTreeMap<String, Arc<Mutex<()>>>,
    display_gates: BTreeMap<String, Arc<super::kernel_browser_display_gate::DisplayGate>>,
    #[cfg(test)]
    after_controller_check: Option<Arc<dyn Fn() + Send + Sync>>,
    access: UserDomainAccess,
    actors: BTreeMap<String, Arc<Mutex<KernelBrowserActors>>>,
    loaded: BTreeSet<(String, String, KernelBrowserCapability)>,
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
    epoch: Arc<BrowserCancellation>,
    cancellation: Arc<BrowserCancellation>,
    terminal_lifetime: Option<crate::runtime::command::TerminalLifetime>,
    capability: KernelBrowserCapability,
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
}

impl KernelBrowserHost {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HostState::default())),
            root: root.join("kernel-browser"),
        }
    }
    // MD-DISPLAY-04: queued credits wait asynchronously, away from the
    // controller mutex. Input can enter between captures rather than behind
    // an entire WAN window of blocking capture/encode/pacing operations.
    pub(crate) fn display_gate(
        &self,
        user: &str,
    ) -> Arc<super::kernel_browser_display_gate::DisplayGate> {
        let mut state = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        state
            .display_gates
            .entry(user.into())
            .or_insert_with(|| Arc::new(super::kernel_browser_display_gate::DisplayGate::default()))
            .clone()
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
            capability,
            epoch: state.access.grant(user, agent)?.epoch.clone(),
            cancellation: state.access.grant(user, agent)?.epoch.clone(),
            terminal_lifetime: None,
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
            epoch: epoch.clone(),
            capability: KernelBrowserCapability::Browser,
            terminal_lifetime: Some(lifetime.clone()),
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
        // MP-08/MP-10/MP-11: mutations keep their existing serial admission,
        // but never hold the shared controller lock during input/CDP waits.
        // Takeover/cancellation use the actor model and remain independent.
        let mutation = method == "host.secret"
            || (method == "host.browser"
                && matches!(
                    params["op"].as_str(),
                    Some("open" | "close" | "navigate" | "input" | "stop")
                ));
        let mutation_lane = if mutation {
            Some(
                self.inner
                    .lock()
                    .map_err(|_| "MD-2: browser host lock poisoned")?
                    .mutation_lanes
                    .entry(user.into())
                    .or_default()
                    .clone(),
            )
        } else {
            None
        };
        let _mutation_guard = mutation_lane
            .as_ref()
            .map(|lane| lane.lock())
            .transpose()
            .map_err(|_| "MD-3: browser mutation lane poisoned")?;
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
            // Focus can move while the all-tab actor action is being admitted.
            self.scope_browser_request(admission, method, &params)?;
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
            if !backend
                .host_is_live()
                .map_err(crate::error::HostFailure::Other)?
            {
                return Err("MD-APP: App host is no longer live".into());
            }
        } else if !backend
            .host_is_live()
            .map_err(crate::error::HostFailure::Other)?
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
                            | "mirror_subscribe"
                            | "mirror_next"
                            | "mirror_close"
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
        backend.protect_host(policy)?;
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
            let observed_input = method == "host.browser"
                && params["op"] == "input"
                && model
                    .lock()
                    .map_err(|_| "MD-3: actor lock poisoned")?
                    .input_is_current(&params);
            if !observed_input {
                let state = backend.host_request_cancellable(
                "host.browser",
                serde_json::json!({"op":if params["op"] == "open" { "start" } else { "state" }}),
                admission.map(|a| a.cancellation.clone()),
            )?;
                model
                    .lock()
                    .map_err(|_| "MD-3: actor lock poisoned")?
                    .reconcile(&state)?;
            }
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
        let request_params = params.clone();
        let display = method == "host.browser"
            && params["op"] == "screenshot"
            && params["display_subscription_id"].is_string();
        let mut result = if display || (method == "host.browser" && params["op"] == "input") {
            let signal = cancellation
                .clone()
                .unwrap_or_else(|| Arc::new(BrowserCancellation::default()));
            let pending = backend.begin_cancellable_mutation(method, &params, &signal)?;
            drop(backend);
            pending
                .wait(&signal)
                .map_err(crate::error::HostFailure::Other)
                .and_then(|response| response.into_host_result(method))
        } else {
            backend.host_request_cancellable(method, params, cancellation.clone())
        };
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
        if let Ok(state) = &result {
            model
                .lock()
                .map_err(|_| "MD-3: actor lock poisoned")?
                .reconcile(state)?;
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
            "Agent",
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
