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
    /// Target → (binding, registration number).
    tabs: HashMap<String, (AppViewBinding, u64)>,
    registrations: u64,
    /// App Tabs the controller last reported open, bound or not.
    open_tabs: usize,
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
        views.registrations += 1;
        views
            .tabs
            .insert(target.to_owned(), (binding, views.registrations));
        !std::mem::replace(&mut views.pumping, true)
    }

    pub(crate) fn binding(&self, session: &str, target: &str) -> Option<AppViewBinding> {
        let sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        sessions
            .get(session)?
            .tabs
            .get(target)
            .map(|(binding, _)| binding.clone())
    }

    /// Registration count to capture before a poll is sent.
    pub(crate) fn registrations(&self, session: &str) -> u64 {
        let sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        sessions.get(session).map_or(0, |views| views.registrations)
    }

    /// Drops bindings whose Tab the controller no longer serves (closed,
    /// crashed or lost on reconnect), so the pump can stop. Only Tabs
    /// registered before the poll (`up_to`) are judged: a view opened while the
    /// poll was in flight is not in its answer yet.
    pub(crate) fn retain_open(&self, session: &str, open_targets: &[String], up_to: u64) {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(views) = sessions.get_mut(session) {
            views.tabs.retain(|target, (_, registered)| {
                *registered > up_to || open_targets.contains(target)
            });
        }
    }

    pub(crate) fn set_open_tabs(&self, session: &str, open: usize) {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(views) = sessions.get_mut(session) {
            views.open_tabs = open;
        }
    }

    /// The pump keeps running while the session has views, or App Tabs the
    /// controller still shows (their calls are answered, as unbound); the last
    /// pass removes the session atomically so a concurrent open restarts it.
    pub(crate) fn keep_pumping(&self, session: &str) -> bool {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match sessions.get(session) {
            Some(views) if !views.tabs.is_empty() || views.open_tabs > 0 => true,
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
            views.open_tabs = 0;
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
        assert!(views.keep_pumping("s"));
        views.forget_session("s");
        assert!(!views.keep_pumping("s"));
        assert!(views.register("s", "t3", binding("a")));
        // A view opened while a poll was in flight survives that poll's answer.
        let before = views.registrations("s");
        assert!(!views.register("s", "t4", binding("b")));
        views.retain_open("s", &[], before);
        assert_eq!(views.binding("s", "t4"), Some(binding("b")));
        assert_eq!(views.binding("s", "t3"), None);
        // A Tab closed in the browser stops the pump on the next poll.
        views.retain_open("s", &[], views.registrations("s"));
        assert!(!views.keep_pumping("s"));
        // An unbound App Tab the controller still shows keeps the pump, so
        // its calls are answered instead of hanging.
        views.register("s", "t7", binding("a"));
        views.retain_open("s", &[], views.registrations("s"));
        views.set_open_tabs("s", 1);
        assert!(views.keep_pumping("s"));
        views.set_open_tabs("s", 0);
        assert!(!views.keep_pumping("s"));
    }
}
