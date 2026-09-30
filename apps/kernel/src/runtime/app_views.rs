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
    /// Tabs being reloaded onto a new generation: until the reload is sent,
    /// the old page's calls are not run against the new backend.
    reconnecting: std::collections::HashSet<String>,
}

impl SessionViews {
    /// A (re)opened or reconnected Tab counts as new, and a failed
    /// reconnection's cooldown no longer applies to it.
    fn bind(&mut self, target: &str, binding: AppViewBinding) {
        self.registrations += 1;
        self.called.remove(target);
        self.unreloadable.remove(target);
        self.tabs
            .insert(target.to_owned(), (binding, self.registrations));
    }
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
        views.bind(target, binding);
        !std::mem::replace(&mut views.pumping, true)
    }

    /// Binds a Tab whose call found it built for an older generation, or
    /// unbound, and marks it reconnecting until `finish_reconnect`. A view's
    /// concurrent calls all get here; only the first binds it and reloads the
    /// page. False while that reload is under way, or once the Tab is bound to
    /// this generation: a second reload would find the slice busy with the
    /// first, fail, and unbind the Tab the first one reconnected.
    pub(crate) fn claim_reconnect(
        &self,
        session: &str,
        target: &str,
        binding: AppViewBinding,
    ) -> bool {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let views = sessions.entry(session.to_owned()).or_default();
        if views.reconnecting.contains(target)
            || views
                .tabs
                .get(target)
                .is_some_and(|(current, _)| *current == binding)
        {
            return false;
        }
        views.bind(target, binding);
        views.reconnecting.insert(target.to_owned());
        true
    }

    /// The reconnect's reload was sent (or failed): calls run again.
    pub(crate) fn finish_reconnect(&self, session: &str, target: &str) {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(views) = sessions.get_mut(session) {
            views.reconnecting.remove(target);
        }
    }

    /// The Tab's binding and whether it is reconnecting, read together.
    pub(crate) fn binding_state(
        &self,
        session: &str,
        target: &str,
    ) -> Option<(AppViewBinding, bool)> {
        let sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let views = sessions.get(session)?;
        let (binding, _) = views.tabs.get(target)?;
        Some((binding.clone(), views.reconnecting.contains(target)))
    }

    /// True for a Tab's first call since it was (re)opened: its document has
    /// loaded, so the Room can project its real title and URL.
    /// A call the old page makes while its Tab reconnects is not the reloaded
    /// page's first call.
    pub(crate) fn first_call(&self, session: &str, target: &str) -> bool {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let views = sessions.entry(session.to_owned()).or_default();
        !views.reconnecting.contains(target) && views.called.insert(target.to_owned())
    }

    /// The Room could not be projected again after this Tab's first call.
    pub(crate) fn forget_call(&self, session: &str, target: &str) {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(views) = sessions.get_mut(session) {
            views.called.remove(target);
        }
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
            views.called.retain(|target| open_targets.contains(target));
            views
                .reconnecting
                .retain(|target| open_targets.contains(target));
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

    /// Bound Tabs, not already reconnecting, whose installation now runs a
    /// newer generation (`running`) than the one their page was built for.
    pub(crate) fn outdated(
        &self,
        session: &str,
        running: impl Fn(&AppViewBinding) -> Option<u64>,
    ) -> Vec<(String, AppViewBinding)> {
        let sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        sessions.get(session).map_or_else(Vec::new, |views| {
            views
                .tabs
                .iter()
                .filter(|(target, (binding, _))| {
                    !views.reconnecting.contains(*target)
                        && running(binding)
                            .is_some_and(|generation| generation > binding.generation)
                })
                .map(|(target, (binding, _))| (target.clone(), binding.clone()))
                .collect()
        })
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

/// Another Room command holds the slice's exclusive operation slot; the
/// refusal text comes from the slice environment store.
pub(crate) fn slice_busy(error: &str) -> bool {
    error.contains("already has an active")
}

/// Projects the Room again after a view's first call. Another Room command
/// may hold the slice briefly, so a busy refusal is retried every `pause`
/// until `window` ends. False when it gave up: the caller un-marks the Tabs,
/// so their next call tries again.
pub(crate) async fn reproject<F, Fut>(
    mut reconcile: F,
    window: std::time::Duration,
    pause: std::time::Duration,
) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let deadline = tokio::time::Instant::now() + window;
    loop {
        match reconcile().await {
            Ok(()) => return true,
            Err(error) if tokio::time::Instant::now() < deadline && slice_busy(&error) => {
                tokio::time::sleep(pause).await;
            }
            Err(_) => return false,
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
        assert_eq!(
            views.binding_state("s", "t2").map(|(binding, _)| binding),
            Some(binding("b"))
        );
        assert_eq!(
            views
                .binding_state("other", "t2")
                .map(|(binding, _)| binding),
            None
        );
        assert!(views.keep_pumping("s"));
        views.forget_session("s");
        assert!(!views.keep_pumping("s"));
        assert!(views.register("s", "t3", binding("a")));
        // A view opened while a poll was in flight survives that poll's answer.
        let before = views.registrations("s");
        assert!(!views.register("s", "t4", binding("b")));
        views.retain_open("s", &[], before);
        assert_eq!(
            views.binding_state("s", "t4").map(|(binding, _)| binding),
            Some(binding("b"))
        );
        assert_eq!(
            views.binding_state("s", "t3").map(|(binding, _)| binding),
            None
        );
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
        assert_eq!(
            views.binding_state("s", "t5").map(|(binding, _)| binding),
            None
        );
        assert_eq!(
            views.binding_state("s", "t6").map(|(binding, _)| binding),
            Some(binding("b"))
        );
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
        // A failed re-projection is retried on the Tab's next call.
        views.forget_call("s", "t1");
        assert!(views.first_call("s", "t1"));
        // Two Tabs in one batch are each marked.
        assert!(views.first_call("s", "t2"));
        assert!(!views.first_call("s", "t2"));
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

    #[tokio::test]
    async fn reprojection_retries_only_a_busy_slice_and_only_within_its_window() {
        use std::time::Duration;
        let busy =
            || async { Err::<(), _>("environment already has an active command".to_owned()) };
        let attempts = std::cell::Cell::new(0);
        let settles = || {
            attempts.set(attempts.get() + 1);
            let attempt = attempts.get();
            async move {
                if attempt < 3 {
                    Err("environment already has an active command".to_owned())
                } else {
                    Ok(())
                }
            }
        };
        let pause = Duration::from_millis(1);
        assert!(reproject(settles, Duration::from_secs(5), pause).await);
        assert_eq!(attempts.get(), 3);
        assert!(!reproject(busy, Duration::from_millis(20), pause).await);
        // Any other refusal gives up at once, without a retry.
        let failures = std::cell::Cell::new(0);
        let failed = || {
            failures.set(failures.get() + 1);
            async { Err::<(), _>("slice is gone".to_owned()) }
        };
        assert!(!reproject(failed, Duration::from_secs(5), pause).await);
        assert_eq!(failures.get(), 1);
    }

    #[test]
    fn a_closed_tabs_first_call_mark_is_dropped() {
        let views = AppViews::default();
        assert!(views.first_call("s", "t1"));
        assert!(views.first_call("s", "t2"));
        views.retain_open("s", &["t2".to_owned()], 0);
        // t1 closed: a Tab with that id would count as new again.
        assert!(views.first_call("s", "t1"));
        assert!(!views.first_call("s", "t2"));
    }

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
        assert_eq!(
            views.binding_state("s", "t1").map(|(binding, _)| binding),
            Some(binding)
        );
        assert!(views.reloadable("s", "t1"));
        views.unbind("s", "t1");
        assert_eq!(
            views.binding_state("s", "t1").map(|(binding, _)| binding),
            None
        );
        assert!(!views.reloadable("s", "t1"));
        // Opening the view again ends the failed reconnection's cooldown.
        views.register("s", "t1", binding_at(2));
        assert!(views.reloadable("s", "t1"));
    }

    fn binding_at(generation: u64) -> AppViewBinding {
        AppViewBinding {
            owner: "user".into(),
            installation: "a".into(),
            generation,
        }
    }

    #[test]
    fn an_update_outdates_its_installations_open_views_until_they_reconnect() {
        let views = AppViews::default();
        views.register("s", "t1", binding_at(1));
        views.register("s", "t2", binding_at(1));
        views.register(
            "s",
            "other-app",
            AppViewBinding {
                installation: "b".into(),
                ..binding_at(1)
            },
        );
        views.register("other-session", "t3", binding_at(1));
        let running = |generation| {
            move |binding: &AppViewBinding| (binding.installation == "a").then_some(generation)
        };
        // Not while the App still runs the generation the pages were built for
        // (or does not run at all: its next call finds out).
        assert!(views.outdated("s", running(1)).is_empty());
        assert!(views.outdated("s", |_| None).is_empty());
        let mut outdated: Vec<_> = views
            .outdated("s", running(2))
            .into_iter()
            .map(|(target, binding)| (target, binding.generation))
            .collect();
        outdated.sort();
        assert_eq!(outdated, [("t1".to_owned(), 1), ("t2".to_owned(), 1)]);
        // A Tab being reconnected, or already bound to the new generation, is not.
        assert!(views.claim_reconnect("s", "t1", binding_at(2)));
        assert_eq!(
            views
                .outdated("s", running(2))
                .into_iter()
                .map(|(target, _)| target)
                .collect::<Vec<_>>(),
            ["t2"]
        );
        views.finish_reconnect("s", "t1");
        assert!(views.claim_reconnect("s", "t2", binding_at(2)));
        views.finish_reconnect("s", "t2");
        assert!(views.outdated("s", running(2)).is_empty());
    }

    #[test]
    fn only_the_first_of_a_views_concurrent_reconnects_reloads_it() {
        let views = AppViews::default();
        views.register("s", "t1", binding_at(1));
        // Three calls from the view built for generation 1 find generation 2.
        assert!(views.claim_reconnect("s", "t1", binding_at(2)));
        assert!(!views.claim_reconnect("s", "t1", binding_at(2)));
        assert!(!views.claim_reconnect("s", "t1", binding_at(2)));
        assert_eq!(
            views.binding_state("s", "t1").map(|(binding, _)| binding),
            Some(binding_at(2))
        );
        // Until the reload is sent, the old page's calls do not run, and they
        // do not count as the reloaded page's first call.
        assert_eq!(views.binding_state("s", "t1"), Some((binding_at(2), true)));
        assert!(!views.first_call("s", "t1"));
        views.finish_reconnect("s", "t1");
        assert_eq!(views.binding_state("s", "t1"), Some((binding_at(2), false)));
        // A stale call answered after the reload still finds it bound.
        assert!(!views.claim_reconnect("s", "t1", binding_at(2)));
        // The reload counts as a new document, whose first call re-projects the Room.
        assert!(views.first_call("s", "t1"));
        // A later update is claimed again.
        assert!(views.claim_reconnect("s", "t1", binding_at(3)));
        views.finish_reconnect("s", "t1");
        // An unbound Tab (after a failed reload, once its cooldown passed) is claimed once.
        views.unbind("s", "t1");
        assert!(views.claim_reconnect("s", "t1", binding_at(3)));
        assert!(!views.claim_reconnect("s", "t1", binding_at(3)));
        assert!(views.reloadable("s", "t1"));
        // A Tab closed mid-reconnect leaves no mark: a new Tab with its id
        // counts its first call.
        views.retain_open("s", &[], views.registrations("s"));
        assert!(views.first_call("s", "t1"));
    }
}
