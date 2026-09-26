//! Open App view Tabs per session. The kernel, not the page or controller,
//! decides which owner and installation a view call runs as.
use crate::session::EnvironmentTabApp;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppViewBinding {
    pub(crate) owner: String,
    pub(crate) installation: String,
    /// The generation whose UI files the Tab runs; calls after an update are
    /// refused so a view never talks to a backend it was not built for.
    pub(crate) generation: u64,
}

#[derive(Default)]
struct SessionViews {
    /// The App most recently opened in this session: (owner, installation).
    foreground: Option<(String, String)>,
    /// Target → (binding, registration number).
    tabs: HashMap<String, (AppViewBinding, u64)>,
    registrations: u64,
    /// App Tabs the controller last reported open, bound or not.
    open_tabs: usize,
    pumping: bool,
    /// The App Tab markers the Room last showed, by target.
    published: BTreeMap<String, EnvironmentTabApp>,
    /// Tabs whose reconnection failed (a Room controller without reload, a
    /// transient Room failure, or a view the host does not own), with when:
    /// answered unbound until the cooldown passes.
    unreloadable: HashMap<String, std::time::Instant>,
    /// Tabs that called since they were (re)opened: their document loaded.
    called: std::collections::HashSet<String>,
}

#[derive(Clone, Default)]
pub(crate) struct AppViews(
    Arc<Mutex<HashMap<String, SessionViews>>>,
    /// Sessions whose Room this kernel already swept for leftover views.
    Arc<Mutex<std::collections::HashSet<String>>>,
);

/// A failed reconnection is not retried sooner than this.
const RELOAD_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(60);

impl AppViews {
    /// Records the Tab; returns true when the caller must start the session's
    /// call pump.
    pub(crate) fn register(&self, session: &str, target: &str, binding: AppViewBinding) -> bool {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let views = sessions.entry(session.to_owned()).or_default();
        views.registrations += 1;
        views.called.remove(target);
        views
            .tabs
            .insert(target.to_owned(), (binding, views.registrations));
        !std::mem::replace(&mut views.pumping, true)
    }

    /// True for a Tab's first call since it was (re)opened: its document has
    /// loaded, so the Room can project its real title and URL.
    pub(crate) fn first_call(&self, session: &str, target: &str) -> bool {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        sessions
            .entry(session.to_owned())
            .or_default()
            .called
            .insert(target.to_owned())
    }

    /// True once per session and kernel: App Tabs outlive a kernel restart in
    /// the Room browser, but their bindings do not; the caller resumes polling
    /// them once the session's Room slice is known.
    pub(crate) fn take_resume(&self, session: &str) -> bool {
        self.1
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(session.to_owned())
    }

    /// Polls a session's Room for App Tabs this kernel did not open (left from
    /// before a restart); true when the caller must start the pump. The first
    /// poll reports how many are really open.
    pub(crate) fn begin_pumping(&self, session: &str) -> bool {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let views = sessions.entry(session.to_owned()).or_default();
        views.open_tabs = views.open_tabs.max(1);
        !std::mem::replace(&mut views.pumping, true)
    }

    /// A failed reconnection: the Tab stays unbound until the cooldown passes.
    pub(crate) fn unbind(&self, session: &str, target: &str) {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(views) = sessions.get_mut(session) {
            views.tabs.remove(target);
            views
                .unreloadable
                .insert(target.to_owned(), std::time::Instant::now());
        }
    }

    pub(crate) fn reloadable(&self, session: &str, target: &str) -> bool {
        let sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        sessions.get(session).is_none_or(|views| {
            views
                .unreloadable
                .get(target)
                .is_none_or(|failed| failed.elapsed() >= RELOAD_COOLDOWN)
        })
    }

    pub(crate) fn set_foreground(&self, session: &str, owner: &str, installation: &str) {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        sessions.entry(session.to_owned()).or_default().foreground =
            Some((owner.to_owned(), installation.to_owned()));
    }

    pub(crate) fn foreground(&self, session: &str) -> Option<(String, String)> {
        let sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        sessions.get(session)?.foreground.clone()
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
            views
                .unreloadable
                .retain(|target, _| open_targets.contains(target));
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

    /// An uninstalled App's open views stay on screen, but every call from
    /// them is refused (`APP_VIEW_UNBOUND`), even if the installation returns.
    pub(crate) fn forget_installation(&self, owner: &str, installation: &str) {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        for views in sessions.values_mut() {
            views.tabs.retain(|_, (binding, _)| {
                binding.owner != owner || binding.installation != installation
            });
            if views
                .foreground
                .as_ref()
                .is_some_and(|(o, i)| o == owner && i == installation)
            {
                views.foreground = None;
            }
        }
    }

    /// Bound Tabs: target → installation.
    pub(crate) fn installations(&self, session: &str) -> Vec<(String, String)> {
        let sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        sessions.get(session).map_or_else(Vec::new, |views| {
            views
                .tabs
                .iter()
                .map(|(target, (binding, _))| (target.clone(), binding.installation.clone()))
                .collect()
        })
    }

    /// Records the Room's App Tab markers; true when they changed.
    pub(crate) fn publish(
        &self,
        session: &str,
        apps: &BTreeMap<String, EnvironmentTabApp>,
    ) -> bool {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let Some(views) = sessions.get_mut(session) else {
            return false;
        };
        if views.published == *apps {
            return false;
        }
        views.published = apps.clone();
        true
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
            generation: 1,
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
        // Uninstall unbinds only that owner's installation.
        views.register("s", "t5", binding("a"));
        views.register("s", "t6", binding("b"));
        views.forget_installation("user", "a");
        assert_eq!(views.binding("s", "t5"), None);
        assert_eq!(views.binding("s", "t6"), Some(binding("b")));
    }

    #[test]
    fn a_views_first_call_after_it_opens_asks_for_the_room_again() {
        let views = AppViews::default();
        views.register("s", "t1", binding("a"));
        assert!(views.first_call("s", "t1"));
        assert!(!views.first_call("s", "t1"));
        // Reopening loads a new document: its first call counts again.
        views.register("s", "t1", binding("a"));
        assert!(views.first_call("s", "t1"));
    }

    #[test]
    fn app_tab_markers_are_published_only_when_they_change() {
        let views = AppViews::default();
        views.register("s", "t1", binding("a"));
        assert_eq!(
            views.installations("s"),
            vec![("t1".to_string(), "a".to_string())]
        );
        let apps = BTreeMap::from([(
            "t1".to_string(),
            EnvironmentTabApp {
                installation_id: "a".into(),
                panel: None,
            },
        )]);
        assert!(views.publish("s", &apps));
        assert!(!views.publish("s", &apps));
        assert!(views.publish("s", &BTreeMap::new()));
        assert!(!views.publish("other", &apps));
    }
}

#[cfg(test)]
mod reconnect_tests {
    use super::*;

    #[test]
    fn a_restart_resumes_polling_once_and_a_reconnected_tab_can_be_unbound() {
        let views = AppViews::default();
        assert!(views.take_resume("s"));
        assert!(!views.take_resume("s"));
        assert!(views.take_resume("other"));
        // A Room with no views this kernel opened is still polled once.
        assert!(views.begin_pumping("s"));
        assert!(!views.begin_pumping("s"));
        assert!(views.keep_pumping("s"));
        views.set_open_tabs("s", 0);
        assert!(!views.keep_pumping("s"));
        let binding = AppViewBinding {
            owner: "user".into(),
            installation: "a".into(),
            generation: 2,
        };
        views.register("s", "t1", binding.clone());
        assert_eq!(views.binding("s", "t1"), Some(binding));
        assert!(views.reloadable("s", "t1"));
        views.unbind("s", "t1");
        assert_eq!(views.binding("s", "t1"), None);
        assert!(!views.reloadable("s", "t1"));
    }
}
