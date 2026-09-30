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
    /// View calls still running, by call number.
    in_flight: HashMap<u64, InFlightCall>,
}

/// A running view call: dropping its sender cancels it.
struct InFlightCall {
    target: String,
    document: Option<String>,
    _cancel: tokio::sync::watch::Sender<()>,
}

/// Held by the task answering a view call; the call is cancelled once its
/// Tab closes or loads another document (see `AppViews::cancel_gone_calls`).
pub(crate) struct ViewCall {
    views: AppViews,
    session: String,
    number: u64,
    cancelled: tokio::sync::watch::Receiver<()>,
}

impl ViewCall {
    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.has_changed().is_err()
    }

    /// Resolves once the call is cancelled.
    pub(crate) async fn cancelled(&mut self) {
        while self.cancelled.changed().await.is_ok() {}
    }
}

impl Drop for ViewCall {
    fn drop(&mut self) {
        let mut sessions = self.views.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(views) = sessions.get_mut(&self.session) {
            views.in_flight.remove(&self.number);
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct AppViews(
    Arc<Mutex<HashMap<String, SessionViews>>>,
    /// Sessions whose Room this kernel already swept for leftover views.
    Arc<Mutex<std::collections::HashSet<String>>>,
    /// View call numbers. Never reset: a session's entry can be removed and
    /// recreated while an old call still holds its number.
    Arc<std::sync::atomic::AtomicU64>,
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
            views.called.retain(|target| open_targets.contains(target));
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
            views.in_flight.clear();
        }
    }

    /// Tracks a view call made by the Tab's `document`. A session no longer
    /// polled gets an already cancelled call.
    pub(crate) fn track_call(
        &self,
        session: &str,
        target: &str,
        document: Option<String>,
    ) -> ViewCall {
        let (cancel, cancelled) = tokio::sync::watch::channel(());
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let number = self.2.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        if let Some(views) = sessions.get_mut(session) {
            views.in_flight.insert(
                number,
                InFlightCall {
                    target: target.to_owned(),
                    document,
                    _cancel: cancel,
                },
            );
        }
        ViewCall {
            views: self.clone(),
            session: session.to_owned(),
            number,
            cancelled,
        }
    }

    /// Cancels the calls whose Tab closed or whose document the Tab no
    /// longer shows (it reloaded or navigated); returns how many. `None`
    /// (an older controller) judges nothing on that account.
    pub(crate) fn cancel_gone_calls(
        &self,
        session: &str,
        open_targets: Option<&[String]>,
        documents: Option<&HashMap<String, String>>,
    ) -> usize {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let Some(views) = sessions.get_mut(session) else {
            return 0;
        };
        let before = views.in_flight.len();
        views.in_flight.retain(|_, call| {
            let open = open_targets.is_none_or(|open| open.contains(&call.target));
            let current = match (documents, &call.document) {
                (Some(documents), Some(document)) => documents.get(&call.target) == Some(document),
                _ => true,
            };
            // Dropping a call's sender is its cancellation.
            open && current
        });
        before - views.in_flight.len()
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
mod call_cancellation_tests {
    use super::*;

    fn views_with_tab() -> AppViews {
        let views = AppViews::default();
        views.register(
            "s",
            "t1",
            AppViewBinding {
                owner: "user".into(),
                installation: "a".into(),
                generation: 1,
            },
        );
        views
    }

    fn open(targets: &[&str]) -> Vec<String> {
        targets.iter().map(|target| (*target).to_owned()).collect()
    }

    fn documents(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(target, document)| ((*target).to_owned(), (*document).to_owned()))
            .collect()
    }

    #[test]
    fn a_reload_or_navigation_cancels_only_the_old_documents_calls() {
        let views = views_with_tab();
        let old = views.track_call("s", "t1", Some("doc-1".into()));
        // The poll that delivered the call still shows its document.
        assert_eq!(
            views.cancel_gone_calls(
                "s",
                Some(&open(&["t1"])),
                Some(&documents(&[("t1", "doc-1")]))
            ),
            0
        );
        assert!(!old.is_cancelled());
        let new = views.track_call("s", "t1", Some("doc-2".into()));
        assert_eq!(
            views.cancel_gone_calls(
                "s",
                Some(&open(&["t1"])),
                Some(&documents(&[("t1", "doc-2")]))
            ),
            1
        );
        assert!(old.is_cancelled());
        assert!(!new.is_cancelled());
    }

    #[test]
    fn a_closed_tab_cancels_its_calls_and_other_tabs_keep_theirs() {
        let views = views_with_tab();
        let closing = views.track_call("s", "t1", Some("doc-1".into()));
        let other = views.track_call("s", "t2", Some("doc-9".into()));
        assert_eq!(
            views.cancel_gone_calls(
                "s",
                Some(&open(&["t2"])),
                Some(&documents(&[("t2", "doc-9")]))
            ),
            1
        );
        assert!(closing.is_cancelled());
        assert!(!other.is_cancelled());
        // The Room went away: every call of the session ends.
        views.forget_session("s");
        assert!(other.is_cancelled());
    }

    #[test]
    fn an_older_controller_cancels_only_on_close() {
        let views = views_with_tab();
        // No document on the call, or none in the poll: only closure counts.
        let unstamped = views.track_call("s", "t1", None);
        let stamped = views.track_call("s", "t1", Some("doc-1".into()));
        assert_eq!(
            views.cancel_gone_calls(
                "s",
                Some(&open(&["t1"])),
                Some(&documents(&[("t1", "doc-2")]))
            ),
            1
        );
        assert!(stamped.is_cancelled());
        assert!(!unstamped.is_cancelled());
        assert_eq!(views.cancel_gone_calls("s", None, None), 0);
        assert_eq!(views.cancel_gone_calls("s", Some(&open(&["t1"])), None), 0);
        assert!(!unstamped.is_cancelled());
        assert_eq!(views.cancel_gone_calls("s", Some(&[]), None), 1);
        assert!(unstamped.is_cancelled());
    }

    #[test]
    fn a_finished_call_is_forgotten_and_an_unpolled_session_cancels_at_once() {
        let views = views_with_tab();
        drop(views.track_call("s", "t1", Some("doc-1".into())));
        assert_eq!(views.cancel_gone_calls("s", Some(&[]), None), 0);
        assert!(views.track_call("gone", "t1", None).is_cancelled());
    }

    #[test]
    fn an_old_call_ending_after_its_session_was_recreated_leaves_new_calls_alone() {
        let views = views_with_tab();
        let old = views.track_call("s", "t1", Some("doc-1".into()));
        // The Tab closed and the pump removed the session; a new view reopens it.
        views.retain_open("s", &[], views.registrations("s"));
        assert!(!views.keep_pumping("s"));
        assert!(old.is_cancelled());
        views.register(
            "s",
            "t1",
            AppViewBinding {
                owner: "user".into(),
                installation: "a".into(),
                generation: 1,
            },
        );
        let new = views.track_call("s", "t1", Some("doc-2".into()));
        drop(old);
        assert!(!new.is_cancelled());
        assert_eq!(
            views.cancel_gone_calls(
                "s",
                Some(&open(&["t1"])),
                Some(&documents(&[("t1", "doc-2")]))
            ),
            0
        );
    }

    #[tokio::test]
    async fn cancellation_wakes_the_waiting_call() {
        let views = views_with_tab();
        let mut call = views.track_call("s", "t1", Some("doc-1".into()));
        let pending = tokio::time::timeout(std::time::Duration::from_millis(20), call.cancelled());
        assert!(pending.await.is_err());
        let waiter = tokio::spawn(async move {
            call.cancelled().await;
            call.cancelled().await;
        });
        tokio::task::yield_now().await;
        views.cancel_gone_calls("s", Some(&[]), None);
        tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .expect("cancelled call wakes")
            .unwrap();
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
        assert_eq!(views.binding("s", "t1"), Some(binding));
        assert!(views.reloadable("s", "t1"));
        views.unbind("s", "t1");
        assert_eq!(views.binding("s", "t1"), None);
        assert!(!views.reloadable("s", "t1"));
    }
}
