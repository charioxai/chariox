//! Open App view Tabs per session. The kernel, not the page or controller,
//! decides which owner and installation a view call runs as.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppViewBinding {
    pub(crate) owner: String,
    pub(crate) installation: String,
}

#[derive(Default)]
struct SessionViews {
    tabs: HashMap<String, AppViewBinding>,
    pumping: bool,
}

#[derive(Clone, Default)]
pub(crate) struct AppViews(Arc<Mutex<HashMap<String, SessionViews>>>);

impl AppViews {
    /// Records the Tab; returns true when the caller must start the session's
    /// call pump.
    pub(crate) fn register(&self, session: &str, target: &str, binding: AppViewBinding) -> bool {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let views = sessions.entry(session.to_owned()).or_default();
        views.tabs.insert(target.to_owned(), binding);
        !std::mem::replace(&mut views.pumping, true)
    }

    pub(crate) fn binding(&self, session: &str, target: &str) -> Option<AppViewBinding> {
        let sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        sessions.get(session)?.tabs.get(target).cloned()
    }

    /// Drops bindings whose Tab the controller no longer serves (closed,
    /// crashed or lost on reconnect), so the pump can stop.
    pub(crate) fn retain_open(&self, session: &str, open_targets: &[String]) {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(views) = sessions.get_mut(session) {
            views.tabs.retain(|target, _| open_targets.contains(target));
        }
    }

    pub(crate) fn remove(&self, session: &str, target: &str) {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(views) = sessions.get_mut(session) {
            views.tabs.remove(target);
        }
    }

    /// The pump keeps running while the session has views; the last pass
    /// removes the session atomically so a concurrent open restarts it.
    pub(crate) fn keep_pumping(&self, session: &str) -> bool {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match sessions.get(session) {
            Some(views) if !views.tabs.is_empty() => true,
            _ => {
                sessions.remove(session);
                false
            }
        }
    }

    pub(crate) fn forget_session(&self, session: &str) {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(views) = sessions.get_mut(session) {
            views.tabs.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(installation: &str) -> AppViewBinding {
        AppViewBinding {
            owner: "user".into(),
            installation: installation.into(),
        }
    }

    #[test]
    fn one_pump_per_session_until_its_last_view_closes() {
        let views = AppViews::default();
        assert!(views.register("s", "t1", binding("a")));
        assert!(!views.register("s", "t2", binding("b")));
        assert_eq!(views.binding("s", "t2"), Some(binding("b")));
        assert_eq!(views.binding("other", "t2"), None);
        views.remove("s", "t1");
        assert!(views.keep_pumping("s"));
        views.forget_session("s");
        assert!(!views.keep_pumping("s"));
        assert!(views.register("s", "t3", binding("a")));
        // A Tab closed in the browser stops the pump on the next poll.
        views.retain_open("s", &[]);
        assert!(!views.keep_pumping("s"));
    }
}
