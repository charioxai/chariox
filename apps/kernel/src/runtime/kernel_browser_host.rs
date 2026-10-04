//! MD-2: kernel authority for Chromium running directly on the host.
use super::browser_controller_process::{
    BrowserControllerProcessBackend, BrowserControllerProcessState,
    BrowserControllerProcessStdioBackend,
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
    focus: BTreeMap<String, String>,
    loaded: BTreeSet<(String, String)>,
    stopped: bool,
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
        if state.focus.get(user).map(String::as_str) != agent {
            state.loaded.retain(|(owner, _)| owner != user);
        }
        if let Some(agent) = agent {
            state.focus.insert(user.into(), agent.into());
        } else {
            state.focus.remove(user);
        }
        crate::transport::mcp_server::catalog_changed();
    }
    pub(crate) fn is_focused(&self, user: &str, agent: &str) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .focus
            .get(user)
            .is_some_and(|focused| focused == agent)
    }
    pub(crate) fn is_loaded(&self, user: &str, agent: &str) -> bool {
        let state = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        state
            .focus
            .get(user)
            .is_some_and(|focused| focused == agent)
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
    fn ensure_ready(backend: &mut BrowserControllerProcessStdioBackend) -> Result<(), String> {
        if !matches!(backend.health(), Ok(health) if health.state == BrowserControllerProcessState::Ready)
        {
            backend.start()?;
        }
        Ok(())
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
    pub(crate) fn protected_request(
        &self,
        user: &str,
        agent: Option<&str>,
        method: &str,
        params: Value,
        policy: Value,
    ) -> Result<Value, String> {
        if let Some(agent) = agent {
            let state = self.inner.lock().map_err(|_| "MD-3: focus lock poisoned")?;
            require_loaded(&state, user, agent)?;
        }
        let browser = self.backend(user)?;
        let mut backend = browser
            .lock()
            .map_err(|_| "MD-2: browser operation lock poisoned")?;
        self.require_running()?;
        if let Some(agent) = agent {
            let state = self.inner.lock().map_err(|_| "MD-3: focus lock poisoned")?;
            require_loaded(&state, user, agent)?;
        }
        if method == "host.browser" && params["op"] == "stop" {
            backend.stop()?;
            return Ok(serde_json::json!({ "state": "stopped", "tabs": [] }));
        }
        Self::ensure_ready(&mut backend)?;
        let mut params = params;
        if agent.is_some() { params["_agent_input"] = true.into(); }
        backend.host_request("host.protect", policy)?;
        backend.host_request(method, params)
    }
    pub(crate) fn shutdown(&self) -> Result<(), String> {
        let browsers = {
            let mut state = self
                .inner
                .lock()
                .map_err(|_| "MD-2: browser host lock poisoned")?;
            state.stopped = true;
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
fn require_focus(state: &HostState, user: &str, agent: &str) -> Result<(), String> {
    if state
        .focus
        .get(user)
        .is_some_and(|focused| focused == agent)
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
    fn retained_host_cannot_restart_after_shutdown() {
        let host = KernelBrowserHost::new(PathBuf::from("/tmp/mdint-stopped-host"));
        let retained = host.clone();
        host.shutdown().unwrap();
        assert!(retained
            .protected_request("alice", None, "host.browser", serde_json::json!({"op":"start"}), Value::Null)
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
}
