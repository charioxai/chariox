//! MD-2: kernel authority for Chromium running directly on the host.
use super::browser_controller_process::{
    BrowserCancellation, BrowserControllerProcessBackend, BrowserControllerProcessState,
    BrowserControllerProcessStdioBackend,
};
use super::kernel_browser_actors::KernelBrowserActors;
pub(crate) use super::kernel_browser_actors::KernelBrowserDocumentBinding;
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
    focus: BTreeMap<String, FocusedAgent>,
    actors: BTreeMap<String, Arc<Mutex<KernelBrowserActors>>>,
    loaded: BTreeSet<(String, String)>,
}
struct FocusedAgent {
    agent_id: String,
    epoch: Arc<BrowserCancellation>,
}
/// Internal admission bound to one user, agent and uninterrupted focus interval.
#[derive(Clone)]
pub(crate) struct KernelBrowserAdmission {
    user: String,
    agent: String,
    epoch: Arc<BrowserCancellation>,
    cancellation: Arc<BrowserCancellation>,
}

impl KernelBrowserAdmission {
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
    pub(crate) fn set_focus(&self, user: &str, agent: Option<&str>) {
        let mut state = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        if state.focus.get(user).map(|focus| focus.agent_id.as_str()) != agent {
            if let Some(previous) = state.focus.get(user) {
                previous.epoch.request_cancel();
            }
            state.loaded.retain(|(owner, _)| owner != user);
            if let Some(agent) = agent {
                state.focus.insert(
                    user.into(),
                    FocusedAgent {
                        agent_id: agent.into(),
                        epoch: Arc::new(BrowserCancellation::default()),
                    },
                );
            } else {
                state.focus.remove(user);
            }
        }
        crate::transport::mcp_server::catalog_changed();
    }
    pub(crate) fn revoke_agent(&self, agent: &str) {
        let mut state = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        let users: Vec<_> = state
            .focus
            .iter()
            .filter(|(_, focus)| focus.agent_id == agent)
            .map(|(user, _)| user.clone())
            .collect();
        for user in users {
            if let Some(previous) = state.focus.remove(&user) {
                previous.epoch.request_cancel();
            }
            state.loaded.retain(|(owner, _)| owner != &user);
        }
        drop(state);
        crate::transport::mcp_server::catalog_changed();
    }
    pub(crate) fn is_focused(&self, user: &str, agent: &str) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .focus
            .get(user)
            .is_some_and(|focused| focused.agent_id == agent)
    }
    pub(crate) fn is_loaded(&self, user: &str, agent: &str) -> bool {
        let state = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        state
            .focus
            .get(user)
            .is_some_and(|focused| focused.agent_id == agent)
            && state.loaded.contains(&(user.into(), agent.into()))
    }
    pub(crate) fn load(&self, user: &str, agent: &str) -> Result<(), String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "MD-3: browser host lock poisoned")?;
        require_focus(&state, user, agent)?;
        state.loaded.insert((user.into(), agent.into()));
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
    fn ensure_ready(backend: &mut BrowserControllerProcessStdioBackend) -> Result<(), String> {
        if !matches!(backend.health(), Ok(health) if health.state == BrowserControllerProcessState::Ready)
        {
            backend.start()?;
        }
        Ok(())
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
    }
    pub(crate) fn admit(&self, user: &str, agent: &str) -> Result<KernelBrowserAdmission, String> {
        let state = self.inner.lock().map_err(|_| "MD-3: focus lock poisoned")?;
        require_loaded(&state, user, agent)?;
        Ok(KernelBrowserAdmission {
            user: user.into(),
            agent: agent.into(),
            epoch: state.focus[user].epoch.clone(),
            cancellation: state.focus[user].epoch.clone(),
        })
    }
    pub(crate) fn check_admission(
        &self,
        admission: Option<&KernelBrowserAdmission>,
    ) -> Result<(), String> {
        if let Some(admission) = admission {
            if admission.cancellation.requested() {
                return Err("MD-3: browser authority revoked".into());
            }
            let state = self.inner.lock().map_err(|_| "MD-3: focus lock poisoned")?;
            require_loaded(&state, &admission.user, &admission.agent)?;
            if !Arc::ptr_eq(&state.focus[&admission.user].epoch, &admission.epoch) {
                return Err("MD-3: browser focus changed; request fresh tools".into());
            }
        }
        Ok(())
    }
    pub(crate) fn protected_request_admitted(
        &self,
        user: &str,
        admission: Option<&KernelBrowserAdmission>,
        method: &str,
        params: Value,
        policy: Value,
    ) -> Result<Value, String> {
        if admission.is_some_and(|admission| admission.user != user) {
            return Err("MD-3: browser admission belongs to another user".into());
        }
        self.check_admission(admission)?;
        let browser = self.backend(user)?;
        let mut backend = browser
            .lock()
            .map_err(|_| "MD-2: browser operation lock poisoned")?;
        self.check_admission(admission)?;
        if method == "host.browser" && params["op"] == "stop" {
            let model = self.actor_model(user)?;
            let (id, cancellation) = model
                .lock()
                .map_err(|_| "MD-3: actor lock poisoned")?
                .begin(browser_actor(admission, &params), &params)?;
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
            result?;
            self.check_admission(admission)?;
            return Ok(serde_json::json!({ "state": "stopped", "tabs": [] }));
        }
        Self::ensure_ready(&mut backend)?;
        self.check_admission(admission)?;
        backend.host_request("host.protect", policy)?;
        self.check_admission(admission)?;
        let model = self.actor_model(user)?;
        let mutation = method == "host.secret"
            || (method == "host.browser"
                && matches!(
                    params["op"].as_str(),
                    Some("open" | "close" | "navigate" | "input")
                ));
        let action = if mutation {
            // Reconcile the SAME supervised browser before ledger admission. Takeover
            // uses only the model lock, so it can cancel while CDP holds the backend.
            let state = backend.host_request("host.browser", serde_json::json!({"op":"state"}))?;
            model
                .lock()
                .map_err(|_| "MD-3: actor lock poisoned")?
                .reconcile(&state)?;
            let actor = browser_actor(admission, &params);
            let (id, cancellation) = model
                .lock()
                .map_err(|_| "MD-3: actor lock poisoned")?
                .begin(actor, &params)?;
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
        let result = backend.host_request_cancellable(method, params, cancellation.clone());
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
        if let Ok(state) = &result {
            model
                .lock()
                .map_err(|_| "MD-3: actor lock poisoned")?
                .reconcile(state)?;
        }
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
    pub(crate) fn request_takeover(
        &self,
        user: &str,
        actor: EnvironmentActor,
        tab: &str,
        generation: u64,
    ) -> Result<TakeoverOutcome, String> {
        self.actor_model(user)?
            .lock()
            .map_err(|_| "MD-3: actor lock poisoned")?
            .takeover(actor, tab, generation)
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

    pub(crate) fn shutdown(&self) -> Result<(), String> {
        let browsers = {
            let mut state = self
                .inner
                .lock()
                .map_err(|_| "MD-2: browser host lock poisoned")?;
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
    if let Some(admission) = admission {
        EnvironmentActor::new(
            format!("agent:{}", admission.agent),
            EnvironmentActorKind::Agent,
            "Focused agent",
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
    if state
        .focus
        .get(user)
        .is_some_and(|focused| focused.agent_id == agent)
    {
        Ok(())
    } else {
        Err("MD-3: user-domain browser access follows current local agent focus".into())
    }
}

fn require_loaded(state: &HostState, user: &str, agent: &str) -> Result<(), String> {
    require_focus(state, user, agent)?;
    if state.loaded.contains(&(user.into(), agent.into())) {
        Ok(())
    } else {
        Err("MD-3: load the browser tools first".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn focus_revokes_loaded_tools_and_never_regrants_previous_agent() {
        let host = KernelBrowserHost::new(PathBuf::from("/tmp/md3-state"));
        assert!(host.load("a", "agent1").is_err());
        host.set_focus("a", Some("agent1"));
        host.load("a", "agent1").unwrap();
        assert!(host.is_loaded("a", "agent1"));
        host.set_focus("a", Some("agent2"));
        assert!(!host.is_loaded("a", "agent1"));
        assert!(host
            .protected_request(
                "a",
                Some("agent1"),
                "host.browser",
                serde_json::json!({"op":"state"}),
                serde_json::Value::Null
            )
            .is_err());
        assert!(!host.is_focused("b", "agent2"));
        host.set_focus("a", Some("agent1"));
        assert!(!host.is_loaded("a", "agent1"));
    }
    #[test]
    fn admission_cannot_revive_after_focus_returns_and_tools_reload() {
        let host = KernelBrowserHost::new(PathBuf::from("/unused/md3-focus-epoch"));
        host.set_focus("a", Some("agent1"));
        host.load("a", "agent1").unwrap();
        let old = host.admit("a", "agent1").unwrap();
        host.set_focus("a", Some("agent2"));
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
        host.set_focus("a", None);
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
        host.set_focus("b", None);
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
