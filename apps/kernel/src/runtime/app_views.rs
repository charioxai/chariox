//! Open App view Tabs per session. The kernel, not the page or controller,
//! decides which owner and installation a view call runs as.
use crate::session::{AppPanelLayout, AppPanelPlacement};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppViewBinding {
    pub(crate) owner: String,
    pub(crate) installation: String,
    /// The generation whose UI files the Tab runs; calls after an update are
    /// refused so a view never talks to a backend it was not built for.
    pub(crate) generation: u64,
    /// The panel its manifest asks for; the page may ask for another.
    pub(crate) panel: PanelRequest,
}

/// The App's own agent panel choice: `placement` None shows no panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PanelRequest {
    pub(crate) placement: Option<AppPanelPlacement>,
    pub(crate) size: Option<u32>,
}

impl Default for PanelRequest {
    fn default() -> Self {
        Self {
            placement: Some(AppPanelPlacement::Right),
            size: None,
        }
    }
}

impl PanelRequest {
    pub(crate) fn from_manifest(panel: Option<&chariox_app_package::AgentPanel>) -> Self {
        use chariox_app_package::PanelPlacement;
        let Some(panel) = panel else {
            return Self::default();
        };
        Self {
            placement: match panel.placement {
                PanelPlacement::Right => Some(AppPanelPlacement::Right),
                PanelPlacement::Bottom => Some(AppPanelPlacement::Bottom),
                PanelPlacement::None => None,
            },
            size: panel.size,
        }
    }
}

/// The user's panel choice for an App in this session; it wins over the App.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct UserPanel {
    pub(crate) placement: Option<AppPanelPlacement>,
    pub(crate) minimized: bool,
}

/// The user's choice, else the App's request (its page's, else its manifest's).
pub(crate) fn resolve_panel(request: PanelRequest, user: UserPanel) -> AppPanelLayout {
    let placement = user.placement.or(request.placement);
    AppPanelLayout {
        placement,
        size: request.size.filter(|_| placement == request.placement),
        minimized: user.minimized && placement.is_some(),
    }
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
    /// Tabs whose reconnection failed (a Room controller without reload, a
    /// transient Room failure, or a view the host does not own), with when:
    /// answered unbound until the cooldown passes.
    unreloadable: HashMap<String, std::time::Instant>,
    /// Tabs that called since they were (re)opened: their document loaded.
    called: std::collections::HashSet<String>,
    /// Panel placements a Tab's page asked for, over its manifest default.
    panel_requests: HashMap<String, PanelRequest>,
    /// The user's panel choices by installation.
    user_panels: HashMap<String, UserPanel>,
    /// The page size each Tab's controller last got, in CSS pixels.
    sent_pages: HashMap<String, (u32, u32)>,
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
        views.panel_requests.remove(target);
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
    /// Bound Tabs: target → (installation, resolved panel layout).
    pub(crate) fn layouts(
        &self,
        session: &str,
    ) -> std::collections::BTreeMap<String, (String, AppPanelLayout)> {
        let sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let Some(views) = sessions.get(session) else {
            return Default::default();
        };
        views
            .tabs
            .iter()
            .map(|(target, (binding, _))| {
                let request = views
                    .panel_requests
                    .get(target)
                    .copied()
                    .unwrap_or(binding.panel);
                let user = views
                    .user_panels
                    .get(&binding.installation)
                    .copied()
                    .unwrap_or_default();
                (
                    target.clone(),
                    (binding.installation.clone(), resolve_panel(request, user)),
                )
            })
            .collect()
    }

    /// The layout a new view of `installation` opens with.
    pub(crate) fn opening_layout(
        &self,
        session: &str,
        installation: &str,
        request: PanelRequest,
    ) -> AppPanelLayout {
        let sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let user = sessions
            .get(session)
            .and_then(|views| views.user_panels.get(installation).copied())
            .unwrap_or_default();
        resolve_panel(request, user)
    }

    /// The page asked for another panel placement; false for an unbound Tab.
    pub(crate) fn request_panel(&self, session: &str, target: &str, request: PanelRequest) -> bool {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let Some(views) = sessions
            .get_mut(session)
            .filter(|views| views.tabs.contains_key(target))
        else {
            return false;
        };
        views.panel_requests.insert(target.to_owned(), request);
        true
    }

    /// The user's choice for an App's panel in this session; returns the new
    /// choice, or None when no view of it is open.
    pub(crate) fn set_user_panel(
        &self,
        session: &str,
        installation: &str,
        placement: Option<AppPanelPlacement>,
        minimized: Option<bool>,
    ) -> Option<UserPanel> {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let views = sessions.get_mut(session)?;
        if !views
            .tabs
            .values()
            .any(|(binding, _)| binding.installation == installation)
        {
            return None;
        }
        let user = views
            .user_panels
            .entry(installation.to_owned())
            .or_default();
        if placement.is_some() {
            user.placement = placement;
        }
        if let Some(minimized) = minimized {
            user.minimized = minimized;
        }
        Some(*user)
    }

    /// The page sizes that changed since each Tab's controller last got one.
    pub(crate) fn changed_pages(
        &self,
        session: &str,
        pages: &std::collections::BTreeMap<String, (u32, u32)>,
    ) -> Vec<(String, (u32, u32))> {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let Some(views) = sessions.get_mut(session) else {
            return Vec::new();
        };
        views
            .sent_pages
            .retain(|target, _| pages.contains_key(target));
        let mut changed = Vec::new();
        for (target, page) in pages {
            if views.sent_pages.insert(target.clone(), *page) != Some(*page) {
                changed.push((target.clone(), *page));
            }
        }
        changed
    }

    /// A layout that did not reach the controller is sent again next time.
    pub(crate) fn forget_page(&self, session: &str, target: &str) {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(views) = sessions.get_mut(session) {
            views.sent_pages.remove(target);
        }
    }

    /// Records the page size a Tab opened with.
    pub(crate) fn sent_page(&self, session: &str, target: &str, page: (u32, u32)) {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(views) = sessions.get_mut(session) {
            views.sent_pages.insert(target.to_owned(), page);
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

    #[test]
    fn the_users_panel_choice_wins_over_the_apps_request_and_its_manifest() {
        let bottom = Some(AppPanelPlacement::Bottom);
        let right = Some(AppPanelPlacement::Right);
        let app = PanelRequest {
            placement: bottom,
            size: Some(300),
        };
        // The App's own choice (its page's request, else its manifest).
        assert_eq!(
            resolve_panel(app, UserPanel::default()),
            AppPanelLayout {
                placement: bottom,
                size: Some(300),
                minimized: false
            }
        );
        // The user moved it: the App's size was for another placement.
        let moved = UserPanel {
            placement: right,
            minimized: false,
        };
        assert_eq!(
            resolve_panel(app, moved),
            AppPanelLayout {
                placement: right,
                size: None,
                minimized: false
            }
        );
        // Minimized only when there is a panel; the user can bring one back
        // to an App that asked for none.
        let none = PanelRequest {
            placement: None,
            size: None,
        };
        let minimized = UserPanel {
            placement: None,
            minimized: true,
        };
        assert_eq!(resolve_panel(none, minimized).minimized, false);
        assert_eq!(resolve_panel(none, moved).placement, right);
        assert_eq!(PanelRequest::from_manifest(None), PanelRequest::default());
    }

    #[test]
    fn panel_requests_and_user_choices_resolve_per_tab() {
        let views = AppViews::default();
        views.register("s", "t1", binding("a"));
        views.register("s", "t2", binding("b"));
        let bottom = PanelRequest {
            placement: Some(AppPanelPlacement::Bottom),
            size: None,
        };
        assert!(views.request_panel("s", "t1", bottom));
        assert!(!views.request_panel("s", "missing", bottom));
        assert!(views.set_user_panel("s", "b", None, Some(true)).is_some());
        assert!(views
            .set_user_panel("s", "closed", None, Some(true))
            .is_none());
        let layouts = views.layouts("s");
        assert_eq!(layouts["t1"].1.placement, Some(AppPanelPlacement::Bottom));
        assert!(layouts["t2"].1.minimized);
        // A new document (reload, reopen) starts from its manifest again.
        views.register("s", "t1", binding("a"));
        assert_eq!(
            views.layouts("s")["t1"].1.placement,
            Some(AppPanelPlacement::Right)
        );
        // Page sizes are sent once per change.
        let pages = std::collections::BTreeMap::from([("t1".to_string(), (900, 800))]);
        assert_eq!(views.changed_pages("s", &pages).len(), 1);
        assert!(views.changed_pages("s", &pages).is_empty());
        views.forget_page("s", "t1");
        assert_eq!(views.changed_pages("s", &pages).len(), 1);
    }

    fn binding(installation: &str) -> AppViewBinding {
        AppViewBinding {
            generation: 1,
            owner: "user".into(),
            installation: installation.into(),
            panel: PanelRequest::default(),
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
            panel: PanelRequest::default(),
        };
        views.register("s", "t1", binding.clone());
        assert_eq!(views.binding("s", "t1"), Some(binding));
        assert!(views.reloadable("s", "t1"));
        views.unbind("s", "t1");
        assert_eq!(views.binding("s", "t1"), None);
        assert!(!views.reloadable("s", "t1"));
    }
}
