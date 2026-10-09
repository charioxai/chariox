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
    /// Kernel Tab identity retained across browser/controller loss. This is
    /// recovery intent, never authority received from a browser placeholder.
    pub(crate) logical_tab: Option<crate::session::EnvironmentTab>,
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
    /// Target → (binding, registration number).
    tabs: HashMap<String, (AppViewBinding, u64)>,
    registrations: u64,
    /// Physical browser identity from the existing authenticated inventory.
    /// CDP connection generations alone do not prove a document was lost.
    browser_identity: Option<(u64, Vec<String>)>,
    /// App Tabs the controller last reported open, bound or not.
    open_tabs: usize,
    pumping: bool,
    /// Open views retained across an explicit slice stop, without old target
    /// authority. Re-open through verified assets once the slice is ready.
    restoring: Vec<(AppViewBinding, u32)>,
    /// Tabs whose reconnection failed (a Room controller without reload, a
    /// transient Room failure, or a view the host does not own), with when:
    /// answered unbound until the cooldown passes.
    unreloadable: HashMap<String, std::time::Instant>,
    /// Tabs that called since they were (re)opened: their document loaded.
    called: std::collections::HashSet<String>,
    /// View calls still running, by call number.
    in_flight: HashMap<u64, InFlightCall>,
    /// (agent, installation) bindings the user revoked: a focus change does
    /// not bind them again; opening the App again does.
    revoked: std::collections::HashSet<(String, String)>,
    /// Panel placements a Tab's page asked for, over its manifest default.
    panel_requests: HashMap<String, PanelRequest>,
    /// The user's panel choices by installation.
    user_panels: HashMap<String, UserPanel>,
    /// The page size each Tab's controller last got, in CSS pixels.
    sent_pages: HashMap<String, (u32, u32)>,
    /// The session's browser controller lays App pages out beside the panel
    /// (it reports `app_panels`); an older one gets no panel and no layout.
    app_panels: bool,
    /// Tabs being reloaded onto a new generation: until the reload is sent,
    /// the old page's calls are not run against the new backend.
    reconnecting: std::collections::HashSet<String>,
    /// Tabs handed out for reloading after their App updated, until that
    /// reconnect ends: the next polls do not hand them out again.
    refreshing: std::collections::HashSet<String>,
}

impl SessionViews {
    fn suspend_targets(&mut self, lost: impl Fn(&str) -> bool) {
        let targets: Vec<_> = self
            .tabs
            .keys()
            .filter(|target| lost(target))
            .cloned()
            .collect();
        for target in targets {
            let (binding, _) = self.tabs.remove(&target).unwrap();
            if !self.restoring.iter().any(|(old, _)| {
                old.owner == binding.owner && old.installation == binding.installation
            }) {
                self.restoring.push((binding, 0));
            }
            self.called.remove(&target);
        }
        // Dropping senders cancels only the documents proved lost.
        self.in_flight.retain(|_, call| !lost(&call.target));
    }

    /// A (re)opened or reconnected Tab counts as new, and a failed
    /// reconnection's cooldown no longer applies to it.
    fn bind(&mut self, target: &str, mut binding: AppViewBinding) {
        if binding.logical_tab.is_none() {
            binding.logical_tab = self
                .tabs
                .get(target)
                .and_then(|(old, _)| old.logical_tab.clone());
        }
        self.registrations += 1;
        self.called.remove(target);
        self.panel_requests.remove(target);
        self.unreloadable.remove(target);
        self.tabs
            .insert(target.to_owned(), (binding, self.registrations));
    }
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

    /// Queue work retains this document's cancellation even while SQLite
    /// admission waits, so a closed view never starts queued work later.
    pub(crate) fn operation_budget(
        &self,
    ) -> crate::runtime::app_operation_budget::AppOperationBudget {
        let cancelled = self.cancelled.clone();
        crate::runtime::app_operation_budget::AppOperationBudget::from_supervisor(move || {
            cancelled.has_changed().is_err()
        })
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
    pub(crate) fn suspend_for_cold_start(&self, session: &str) {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(views) = sessions.get_mut(session) {
            views.suspend_targets(|_| true);
            views.open_tabs = 0;
            views.called.clear();
        }
    }

    pub(crate) fn has_missing_targets(&self, session: &str, open: &[String], up_to: u64) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(session)
            .is_some_and(|views| {
                views
                    .tabs
                    .iter()
                    .any(|(target, (_, registered))| *registered <= up_to && !open.contains(target))
            })
    }

    pub(crate) fn observe_browser_identity(
        &self,
        session: &str,
        room_generation: u64,
        browser_ids: &[String],
        open: &[String],
    ) {
        // Older controllers without inventory cannot prove a physical restart.
        if browser_ids.is_empty() {
            return;
        }
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let Some(views) = sessions.get_mut(session) else {
            return;
        };
        if views
            .browser_identity
            .as_ref()
            .is_some_and(|(generation, old)| *generation == room_generation && old != browser_ids)
        {
            // Targets created after the restart may already be registered.
            // Preserve them; only the old browser's missing documents ended.
            views.suspend_targets(|target| !open.iter().any(|id| id == target));
        }
        views.browser_identity = Some((room_generation, browser_ids.to_vec()));
    }

    pub(crate) fn remember_logical_tab(
        &self,
        session: &str,
        target: &str,
        tab: crate::session::EnvironmentTab,
    ) {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((binding, _)) = sessions
            .get_mut(session)
            .and_then(|views| views.tabs.get_mut(target))
        {
            binding.logical_tab = Some(tab);
        }
    }

    pub(crate) fn cold_start_views(&self, session: &str) -> Vec<AppViewBinding> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(session)
            .map_or_else(Vec::new, |views| {
                views
                    .restoring
                    .iter()
                    .map(|(binding, _)| binding.clone())
                    .collect()
            })
    }

    /// One restore per poll, rotating past failures so they cannot starve
    /// another view. Only admitted failures spend the three attempts.
    pub(crate) fn next_cold_start_attempt(&self, session: &str) -> Option<AppViewBinding> {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let views = sessions.get_mut(session)?;
        views.restoring.retain(|(_, attempts)| *attempts < 3);
        let (binding, _) = views.restoring.first()?;
        let binding = binding.clone();
        views.restoring.rotate_left(1);
        Some(binding)
    }

    pub(crate) fn fail_cold_start_view(&self, session: &str, binding: &AppViewBinding) {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(views) = sessions.get_mut(session) {
            if let Some((_, attempts)) = views.restoring.iter_mut().find(|(old, _)| {
                old.owner == binding.owner && old.installation == binding.installation
            }) {
                *attempts += 1;
            }
        }
    }

    pub(crate) fn finish_cold_start_view(&self, session: &str, binding: &AppViewBinding) {
        if let Some(views) = self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(session)
        {
            views.restoring.retain(|(old, _)| {
                old.owner != binding.owner || old.installation != binding.installation
            });
        }
    }

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
            || views.tabs.get(target).is_some_and(|(current, _)| {
                current.owner == binding.owner
                    && current.installation == binding.installation
                    && current.generation == binding.generation
                    && current.panel == binding.panel
            })
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

    /// Records (true) or forgets (false) a user revocation of this binding.
    pub(crate) fn set_revoked(
        &self,
        session: &str,
        agent: &str,
        installation: &str,
        revoked: bool,
    ) {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let key = (agent.to_owned(), installation.to_owned());
        match sessions.get_mut(session) {
            Some(views) if !revoked => {
                views.revoked.remove(&key);
            }
            Some(views) => {
                views.revoked.insert(key);
            }
            // Recorded even before the session opens an App: a later focus
            // change must not bind the revoked pair again.
            None if revoked => {
                sessions
                    .entry(session.to_owned())
                    .or_default()
                    .revoked
                    .insert(key);
            }
            None => {}
        }
    }

    pub(crate) fn is_revoked(&self, session: &str, agent: &str, installation: &str) -> bool {
        let sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        sessions.get(session).is_some_and(|views| {
            views
                .revoked
                .contains(&(agent.to_owned(), installation.to_owned()))
        })
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
            views
                .refreshing
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
    /// The user's revocations outlive the views: a later focus change still
    /// does not bind a revoked pair. Such an entry holds only the session's
    /// revoked (agent, App) pairs and stays for the kernel's lifetime.
    pub(crate) fn keep_pumping(&self, session: &str) -> bool {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match sessions.get(session) {
            Some(views)
                if !views.tabs.is_empty() || views.open_tabs > 0 || !views.restoring.is_empty() =>
            {
                true
            }
            _ => {
                let revoked = sessions
                    .remove(session)
                    .map(|views| views.revoked)
                    .unwrap_or_default();
                if !revoked.is_empty() {
                    sessions.insert(
                        session.to_owned(),
                        SessionViews {
                            revoked,
                            ..Default::default()
                        },
                    );
                }
                false
            }
        }
    }

    /// An uninstalled App's open views stay on screen, but every call from
    /// them is refused (`APP_VIEW_UNBOUND`), even if the installation returns.
    pub(crate) fn forget_installation(&self, owner: &str, installation: &str) {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        for views in sessions.values_mut() {
            views.restoring.retain(|(binding, _)| {
                binding.owner != owner || binding.installation != installation
            });
            views.tabs.retain(|_, (binding, _)| {
                binding.owner != owner || binding.installation != installation
            });
        }
    }

    /// Bound Tabs whose installation now runs a newer generation (`running`)
    /// than the one their page was built for. Each is handed out once, and
    /// not while it reconnects, until `finish_refresh`.
    pub(crate) fn take_outdated(
        &self,
        session: &str,
        running: impl Fn(&AppViewBinding) -> Option<u64>,
    ) -> Vec<(String, AppViewBinding)> {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let Some(views) = sessions.get_mut(session) else {
            return Vec::new();
        };
        let outdated: Vec<(String, AppViewBinding)> = views
            .tabs
            .iter()
            .filter(|(target, (binding, _))| {
                !views.reconnecting.contains(*target)
                    && !views.refreshing.contains(*target)
                    && running(binding).is_some_and(|generation| generation > binding.generation)
            })
            .map(|(target, (binding, _))| (target.clone(), binding.clone()))
            .collect();
        views
            .refreshing
            .extend(outdated.iter().map(|(target, _)| target.clone()));
        outdated
    }

    /// The reload an update asked for ended (sent, failed or not needed).
    pub(crate) fn finish_refresh(&self, session: &str, target: &str) {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(views) = sessions.get_mut(session) {
            views.refreshing.remove(target);
        }
    }

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
        reset: bool,
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
        if reset {
            views.user_panels.remove(installation);
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

    /// Whether the session's browser controller draws App pages beside a panel.
    pub(crate) fn set_app_panels(&self, session: &str, app_panels: bool) {
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(views) = sessions.get_mut(session) {
            views.app_panels = app_panels;
        }
    }

    pub(crate) fn app_panels(&self, session: &str) -> bool {
        let sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        sessions.get(session).is_some_and(|views| views.app_panels)
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
            views.restoring.clear();
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

/// Another Room command holds the slice's exclusive operation slot, or its
/// admission queue timed out before dispatch. Neither ran the view operation,
/// so retry without consuming the cold-restore failure budget.
pub(crate) fn slice_busy(error: &str) -> bool {
    error.contains("already has an active")
        || error.contains(crate::slice::ENVIRONMENT_USE_ADMISSION_EXPIRED)
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
        assert!(views
            .set_user_panel("s", "b", None, Some(true), false)
            .is_some());
        assert!(views
            .set_user_panel("s", "closed", None, Some(true), false)
            .is_none());
        let layouts = views.layouts("s");
        assert_eq!(layouts["t1"].1.placement, Some(AppPanelPlacement::Bottom));
        assert!(layouts["t2"].1.minimized);
        // A reset hands the panel back to the App.
        views.set_user_panel("s", "b", None, None, true);
        assert!(!views.layouts("s")["t2"].1.minimized);
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
            logical_tab: None,
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
    fn a_revoked_binding_is_remembered_until_the_app_binds_it_again() {
        let views = AppViews::default();
        assert!(!views.is_revoked("s", "agent-1", "a"));
        // Recorded even before the session has App views.
        views.set_revoked("s", "agent-1", "a", true);
        assert!(views.is_revoked("s", "agent-1", "a"));
        assert!(!views.is_revoked("s", "agent-2", "a"));
        assert!(!views.is_revoked("other", "agent-1", "a"));
        // Kept when the session's last App view closes; a new view starts
        // the pump again.
        assert!(views.register("s", "t", binding("a")));
        views.retain_open("s", &[], u64::MAX);
        assert!(!views.keep_pumping("s"));
        assert!(views.is_revoked("s", "agent-1", "a"));
        assert_eq!(views.binding_state("s", "t"), None);
        assert!(views.register("s", "t", binding("a")));
        views.set_revoked("s", "agent-1", "a", false);
        assert!(!views.is_revoked("s", "agent-1", "a"));
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
                logical_tab: None,
                owner: "user".into(),
                installation: "a".into(),
                generation: 1,
                panel: PanelRequest::default(),
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
                logical_tab: None,
                owner: "user".into(),
                installation: "a".into(),
                generation: 1,
                panel: PanelRequest::default(),
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

    #[test]
    fn failed_restores_are_bounded_and_other_registered_views_keep_call_authority() {
        let views = AppViews::default();
        let binding = |installation: &str| AppViewBinding {
            logical_tab: None,
            owner: "owner".into(),
            installation: installation.into(),
            generation: 1,
            panel: PanelRequest::default(),
        };
        let bound = |target: &str| {
            views
                .binding_state("room", target)
                .map(|(binding, _)| binding)
        };
        views.register("room", "bad-old", binding("bad"));
        views.register("room", "good-old", binding("good"));
        views.suspend_for_cold_start("room");
        let mut bad_attempts = 0;
        let mut polls = 0;
        while let Some(next) = views.next_cold_start_attempt("room") {
            if next.installation == "good" {
                views.register("room", "good-new", next.clone());
                views.finish_cold_start_view("room", &next);
            } else {
                bad_attempts += 1; // Persistent controller/storage failure.
                views.fail_cold_start_view("room", &next);
            }
            // The pump polls between attempts; a normal Open and recovered
            // view retain their independent authority while another fails.
            views.register("room", "manual", binding("manual"));
            let up_to = views.registrations("room");
            views.retain_open("room", &["manual".into(), "good-new".into()], up_to);
            assert_eq!(bound("manual"), Some(binding("manual")));
            assert!(bound("bad-old").is_none());
            polls += 1;
        }
        assert_eq!(bad_attempts, 3);
        assert_eq!(polls, 4);
        assert_eq!(bound("good-new"), Some(binding("good")));
        assert!(views.cold_start_views("room").is_empty());
        assert!(views.keep_pumping("room"));
        views.suspend_for_cold_start("room");
        views.forget_session("room");
        assert!(views.cold_start_views("room").is_empty());
        assert!(!views.keep_pumping("room"));
    }

    #[test]
    fn a_cold_slice_stop_keeps_only_open_view_intent_until_reauthorized() {
        let views = AppViews::default();
        let binding = AppViewBinding {
            logical_tab: None,
            owner: "owner".into(),
            installation: "app".into(),
            generation: 1,
            panel: PanelRequest::default(),
        };
        views.register("room", "closed", binding.clone());
        views.retain_open("room", &[], views.registrations("room"));
        views.suspend_for_cold_start("room");
        assert!(views.cold_start_views("room").is_empty());
        views.register("room", "old", binding.clone());
        views.suspend_for_cold_start("room");
        assert_eq!(views.cold_start_views("room"), vec![binding.clone()]);
        assert!(views.binding_state("room", "old").is_none());
        views.set_open_tabs("room", 0);
        assert!(views.keep_pumping("room"));
        views.register("room", "restored", binding.clone());
        views.finish_cold_start_view("room", &binding);
        assert!(views.cold_start_views("room").is_empty());
        assert_eq!(
            views.binding_state("room", "restored"),
            Some((binding.clone(), false))
        );
        views.suspend_for_cold_start("room");
        views.forget_installation("owner", "app");
        assert!(views.cold_start_views("room").is_empty());
        assert!(!views.keep_pumping("room"));
    }

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

    #[tokio::test]
    async fn queued_reprojection_timeout_preserves_the_retry_until_dispatch() {
        use std::time::Duration;
        let attempts = std::cell::Cell::new(0);
        let reconcile = || {
            attempts.set(attempts.get() + 1);
            let attempt = attempts.get();
            async move {
                if attempt == 1 {
                    Err(format!(
                        "local transport `browser_controller.route` failed: {}",
                        crate::slice::ENVIRONMENT_USE_ADMISSION_EXPIRED
                    ))
                } else {
                    Ok(())
                }
            }
        };
        assert!(reproject(reconcile, Duration::from_secs(5), Duration::from_millis(1)).await);
        assert_eq!(attempts.get(), 2);
    }

    #[test]
    fn controller_failures_are_not_admission_contention() {
        for error in [
            "controller exited",
            "browser_controller_scope_denied",
            "command deadline expired after dispatch",
        ] {
            assert!(!slice_busy(error));
        }
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
            logical_tab: None,
            owner: "user".into(),
            installation: "a".into(),
            generation: 2,
            panel: PanelRequest::default(),
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
            logical_tab: None,
            owner: "user".into(),
            installation: "a".into(),
            generation,
            panel: PanelRequest::default(),
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
        let targets = |taken: Vec<(String, AppViewBinding)>| {
            let mut targets: Vec<_> = taken
                .into_iter()
                .map(|(target, binding)| (target, binding.generation))
                .collect();
            targets.sort();
            targets
        };
        // Not while the App still runs the generation the pages were built for
        // (or does not run at all: its next call finds out).
        assert!(views.take_outdated("s", running(1)).is_empty());
        assert!(views.take_outdated("s", |_| None).is_empty());
        assert_eq!(
            targets(views.take_outdated("s", running(2))),
            [("t1".to_owned(), 1), ("t2".to_owned(), 1)]
        );
        // Handed out once: the next polls wait for those reloads to end.
        assert!(views.take_outdated("s", running(2)).is_empty());
        views.finish_refresh("s", "t2");
        assert_eq!(
            targets(views.take_outdated("s", running(2))),
            [("t2".to_owned(), 1)]
        );
        // A Tab being reconnected, or already bound to the new generation, is not.
        views.finish_refresh("s", "t1");
        views.finish_refresh("s", "t2");
        assert!(views.claim_reconnect("s", "t1", binding_at(2)));
        assert_eq!(
            targets(views.take_outdated("s", running(2))),
            [("t2".to_owned(), 1)]
        );
        views.finish_refresh("s", "t2");
        views.finish_reconnect("s", "t1");
        assert!(views.claim_reconnect("s", "t2", binding_at(2)));
        views.finish_reconnect("s", "t2");
        assert!(views.take_outdated("s", running(2)).is_empty());
        assert!(views.take_outdated("other-session", running(1)).is_empty());
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
    #[test]
    fn browser_loss_retains_the_logical_app_tab_and_cancels_old_document_calls() {
        let views = AppViews::default();
        let saved = crate::session::EnvironmentTab {
            tab_id: "tab-7".into(),
            url: "https://app.todo.invalid/".into(),
            title: "Todo".into(),
            document_revision: 4,
            focused: true,
            app: None,
        };
        views.register("room", "old", binding_at(1));
        views.remember_logical_tab("room", "old", saved.clone());
        let call = views.track_call("room", "old", Some("old-document".into()));
        views.suspend_for_cold_start("room");
        assert!(call.is_cancelled());
        assert!(views.binding_state("room", "old").is_none());
        let intent = views.next_cold_start_attempt("room").unwrap();
        assert_eq!(intent.logical_tab, Some(saved.clone()));
        views.register("room", "new", intent.clone());
        views.finish_cold_start_view("room", &intent);
        // An ordinary generation reconnect does not overwrite saved identity.
        assert!(!views.claim_reconnect("room", "new", binding_at(1)));
        assert_eq!(
            views.binding_state("room", "new").unwrap().0.logical_tab,
            Some(saved)
        );
        views.retain_open("room", &[], views.registrations("room"));
        views.suspend_for_cold_start("room");
        assert!(views.cold_start_views("room").is_empty());
    }
    fn recovery_binding(installation: &str) -> AppViewBinding {
        AppViewBinding {
            installation: installation.into(),
            ..binding_at(1)
        }
    }

    #[test]
    fn fast_browser_restart_preserves_missing_views_without_reloading_live_targets() {
        let views = AppViews::default();
        views.register("room", "old", recovery_binding("todo"));
        views.register("room", "closed", recovery_binding("closed"));
        views.observe_browser_identity(
            "room",
            1,
            &["browser-pid-1".into()],
            &["old".into(), "closed".into()],
        );
        let old_call = views.track_call("room", "old", Some("old-doc".into()));
        views.unbind("room", "closed"); // an acknowledged close must stay closed
        let up_to = views.registrations("room");
        views.register("room", "new", recovery_binding("other"));
        let live_call = views.track_call("room", "new", Some("new-doc".into()));
        assert!(views.has_missing_targets("room", &[], up_to));
        views.observe_browser_identity("room", 1, &["browser-pid-2".into()], &["new".into()]);
        views.retain_open("room", &[], up_to); // the older poll cannot drop a new registration
        assert_eq!(
            views.cold_start_views("room"),
            vec![recovery_binding("todo")]
        );
        assert!(old_call.is_cancelled());
        assert!(!live_call.is_cancelled());
        assert!(views.binding_state("room", "new").is_some());
    }

    #[test]
    fn same_browser_reconnect_and_ordinary_close_do_not_restore_a_view() {
        let views = AppViews::default();
        views.register("room", "old", recovery_binding("todo"));
        views.observe_browser_identity("room", 1, &["browser-pid-1".into()], &["old".into()]);
        views.observe_browser_identity("room", 1, &["browser-pid-1".into()], &[]);
        views.retain_open("room", &[], views.registrations("room"));
        assert!(views.cold_start_views("room").is_empty());
        views.register("room", "new", recovery_binding("todo"));
        views.observe_browser_identity("room", 2, &["browser-pid-2".into()], &[]);
        assert!(views.cold_start_views("room").is_empty()); // separate Room incarnation
    }
}
