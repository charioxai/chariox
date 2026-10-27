//! Kernel-owned, ephemeral user-domain instances. No Session or Room exists
//! for these identifiers. Owner checks are repeated for every lookup/close.
use super::app_views::AppViewBinding;
use crate::local::{AppRequestErrorCode, UserAppView};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Registry {
    views: BTreeMap<String, (AppViewBinding, UserAppView)>,
    cursors: HashMap<String, u64>,
    browser_targets: HashMap<String, String>,
    pumping: BTreeSet<String>,
    stopped: bool,
    /// MP-08/MP-11: Apps an owner left open when the browser or kernel went
    /// away. They reopen as new Tabs once the browser serves the owner again.
    restore: HashMap<String, Vec<String>>,
    restoring: HashMap<String, (bool, std::time::Instant)>,
}

impl Registry {
    fn hosted(&self, owner: &str) -> Vec<String> {
        self.views
            .values()
            .filter(|(b, v)| b.owner == owner && v.browser.is_some())
            .map(|(b, _)| b.installation.clone())
            .collect()
    }

    fn intent(&self, owner: &str) -> Vec<String> {
        let mut intent = self.hosted(owner);
        intent.extend(self.pending(owner));
        intent.truncate(32);
        intent
    }

    fn remove_view(&mut self, owner: &str, id: &str) -> Option<AppViewBinding> {
        if self.stopped || self.views.get(id).is_none_or(|(b, _)| b.owner != owner) {
            return None;
        }
        let (binding, _) = self.views.remove(id)?;
        self.browser_targets.remove(id);
        *self.cursors.entry(owner.into()).or_default() += 1;
        Some(binding)
    }

    fn pending(&self, owner: &str) -> Vec<String> {
        self.restore.get(owner).cloned().unwrap_or_default()
    }

    fn settle_restore(&mut self, owner: &str, installation: &str) {
        if let Some(pending) = self.restore.get_mut(owner) {
            if let Some(index) = pending.iter().position(|app| app == installation) {
                pending.remove(index);
            }
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct UserAppViews {
    registry: Arc<Mutex<Registry>>,
    changed: Arc<tokio::sync::Notify>,
    /// One kernel-browser open per owner at a time (see `opening`).
    opening: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

impl UserAppViews {
    /// MP-08/MP-11: opening an App as a kernel-browser Tab, from first check to
    /// bound Tab, is one step per owner; racing Opens (Web, TUI, restore) then
    /// find the Tab the first one made instead of each allocating their own.
    pub(crate) async fn opening(&self, owner: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut opening = self.opening.lock().unwrap_or_else(|e| e.into_inner());
            opening.retain(|_, lock| Arc::strong_count(lock) > 1);
            opening.entry(owner.into()).or_default().clone()
        };
        lock.lock_owned().await
    }

    pub(crate) fn open(
        &self,
        owner: &str,
        binding: AppViewBinding,
        label: &str,
    ) -> Result<UserAppView, AppRequestErrorCode> {
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        if registry.stopped {
            return Err(AppRequestErrorCode::Conflict);
        }
        if registry.views.len() >= 128
            || registry
                .views
                .values()
                .filter(|(b, _)| b.owner == owner)
                .count()
                >= 32
        {
            return Err(AppRequestErrorCode::LimitExceeded);
        }
        let id = format!("user-app-{:032x}", rand::random::<u128>());
        let view = UserAppView {
            view_id: id.clone(),
            installation_id: binding.installation.clone(),
            generation: binding.generation.to_string(),
            origin: format!("https://app.{label}.invalid"),
            browser: None,
            access: None,
        };
        registry.views.insert(id, (binding, view.clone()));
        *registry.cursors.entry(owner.into()).or_default() += 1;
        drop(registry);
        self.changed.notify_waiters();
        Ok(view)
    }

    pub(crate) fn get(&self, owner: &str, id: &str) -> Option<(AppViewBinding, UserAppView)> {
        self.registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .views
            .get(id)
            .filter(|(b, _)| b.owner == owner)
            .cloned()
    }

    /// The owner's view of this installation that holds a browser Tab.
    pub(crate) fn hosted(
        &self,
        owner: &str,
        installation: &str,
    ) -> Option<(AppViewBinding, UserAppView)> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        registry
            .views
            .values()
            .find(|(b, v)| {
                b.owner == owner && b.installation == installation && v.browser.is_some()
            })
            .cloned()
    }

    pub(crate) fn list(&self, owner: &str) -> Vec<UserAppView> {
        self.registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .views
            .values()
            .filter(|(b, _)| b.owner == owner)
            .map(|(_, v)| v.clone())
            .collect()
    }

    pub(crate) fn bind_browser(
        &self,
        owner: &str,
        id: &str,
        target: String,
        browser: crate::local::UserAppViewBrowser,
    ) -> Option<UserAppView> {
        self.bind_browser_selected(owner, id, target, browser, false)
    }

    pub(crate) fn bind_restoring_browser(
        &self,
        owner: &str,
        id: &str,
        target: String,
        browser: crate::local::UserAppViewBrowser,
    ) -> Option<UserAppView> {
        self.bind_browser_selected(owner, id, target, browser, true)
    }

    fn bind_browser_selected(
        &self,
        owner: &str,
        id: &str,
        target: String,
        browser: crate::local::UserAppViewBrowser,
        restoring: bool,
    ) -> Option<UserAppView> {
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        if restoring {
            let (binding, _) = registry.views.get(id)?;
            if !registry.pending(owner).contains(&binding.installation) {
                return None;
            }
        }
        let (binding, view) = registry.views.get_mut(id)?;
        if binding.owner != owner || view.browser.is_some() {
            return None;
        }
        view.browser = Some(browser);
        let installation = binding.installation.clone();
        let view = view.clone();
        registry.settle_restore(owner, &installation);
        registry.browser_targets.insert(id.into(), target);
        *registry.cursors.entry(owner.into()).or_default() += 1;
        drop(registry);
        self.changed.notify_waiters();
        Some(view)
    }

    /// Binds a view to another release generation: an update reloads its page onto it.
    pub(crate) fn set_generation(&self, owner: &str, id: &str, generation: u64) -> bool {
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        let Some((binding, view)) = registry.views.get_mut(id) else {
            return false;
        };
        if binding.owner != owner {
            return false;
        }
        binding.generation = generation;
        view.generation = generation.to_string();
        *registry.cursors.entry(owner.into()).or_default() += 1;
        drop(registry);
        self.changed.notify_waiters();
        true
    }

    pub(crate) fn browser_target(&self, owner: &str, id: &str) -> Option<String> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        registry.views.get(id).filter(|(b, _)| b.owner == owner)?;
        registry.browser_targets.get(id).cloned()
    }

    pub(crate) fn browser_view(&self, owner: &str, target: &str) -> Option<UserAppView> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        let (id, _) = registry.browser_targets.iter().find(|(id, t)| {
            *t == target
                && registry
                    .views
                    .get(*id)
                    .is_some_and(|(b, _)| b.owner == owner)
        })?;
        registry
            .views
            .get(id)
            .filter(|(b, _)| b.owner == owner)
            .map(|(_, v)| v.clone())
    }

    pub(crate) fn browser_views(&self, owner: &str) -> Vec<UserAppView> {
        self.list(owner)
            .into_iter()
            .filter(|v| v.browser.is_some())
            .collect()
    }

    pub(crate) fn begin_browser_pump(&self, owner: &str) -> bool {
        self.registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pumping
            .insert(owner.into())
    }

    /// End the pump atomically with observing the last hosted view disappear;
    /// a concurrent new open can then claim the next pump without losing it.
    pub(crate) fn keep_browser_pumping(&self, owner: &str) -> bool {
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        if registry
            .views
            .values()
            .any(|(b, v)| b.owner == owner && v.browser.is_some())
        {
            return true;
        }
        registry.pumping.remove(owner);
        false
    }

    pub(crate) fn close(&self, owner: &str, id: &str) -> bool {
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        let closed = registry.remove_view(owner, id).is_some();
        drop(registry);
        if closed {
            self.changed.notify_waiters();
        }
        closed
    }

    /// Removing the hosted Tab and returning its intent are one transition.
    pub(crate) fn suspend(&self, owner: &str, id: &str) -> bool {
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        let Some(binding) = registry.remove_view(owner, id) else {
            return false;
        };
        let pending = registry.restore.entry(owner.into()).or_default();
        if pending.len() < 32 {
            pending.push(binding.installation);
        }
        drop(registry);
        self.changed.notify_waiters();
        true
    }

    /// Installations to reopen: hosted instances, then pending restores.
    #[cfg(test)]
    pub(crate) fn restore_intent(&self, owner: &str) -> Vec<String> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        registry.intent(owner)
    }

    /// A save commits before shutdown can retire the registry. After shutdown
    /// a late restore/pump cannot replace the durable file with an empty list.
    pub(crate) fn save_restore_intent(&self, owner: &str, save: impl FnOnce(Vec<String>)) {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        if !registry.stopped {
            save(registry.intent(owner));
        }
    }

    /// Seed restore intent persisted by an earlier kernel (first use only).
    pub(crate) fn seed_restore(&self, owner: &str, installations: Vec<String>) {
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        if registry.restore.contains_key(owner) {
            return;
        }
        // An earlier kernel saved one entry per open (repeat opens added Tabs):
        // an App is restored as one Tab.
        let mut pending: Vec<String> = Vec::new();
        for installation in installations {
            if pending.len() < 32 && !pending.contains(&installation) {
                pending.push(installation);
            }
        }
        // First use may load disk after an explicit browser binding. Account
        // for each hosted Tab once here; later bindings consume their entry.
        for installation in registry.hosted(owner) {
            if let Some(index) = pending.iter().position(|app| app == &installation) {
                pending.remove(index);
            }
        }
        registry.restore.insert(owner.into(), pending);
    }

    pub(crate) fn has_restore(&self, owner: &str) -> Option<bool> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        registry
            .restore
            .get(owner)
            .map(|pending| !pending.is_empty())
    }

    /// One restore pass per owner at a time, at most every five seconds.
    pub(crate) fn begin_restore(&self, owner: &str) -> bool {
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        if registry.stopped
            || registry.restoring.get(owner).is_some_and(|(running, at)| {
                *running || at.elapsed() < std::time::Duration::from_secs(5)
            })
        {
            return false;
        }
        registry
            .restoring
            .insert(owner.into(), (true, std::time::Instant::now()));
        true
    }

    pub(crate) fn end_restore(&self, owner: &str) {
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((running, _)) = registry.restoring.get_mut(owner) {
            *running = false;
        }
    }

    /// Snapshot a pass without withdrawing its uncompleted durable intent.
    pub(crate) fn restore_batch(&self, owner: &str) -> Vec<String> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        if registry.stopped {
            return Vec::new();
        }
        registry.pending(owner)
    }

    /// A permanently unavailable installation no longer has restore intent.
    pub(crate) fn settle_restore(&self, owner: &str, installation: &str) {
        self.registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .settle_restore(owner, installation);
    }

    pub(crate) fn record_change(&self, owner: &str) {
        *self
            .registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cursors
            .entry(owner.into())
            .or_default() += 1;
        self.changed.notify_waiters();
    }

    pub(crate) fn shutdown(&self) -> Vec<String> {
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        registry.stopped = true;
        let records = std::mem::take(&mut registry.views);
        for (binding, _) in records.values() {
            *registry.cursors.entry(binding.owner.clone()).or_default() += 1;
        }
        registry.browser_targets.clear();
        registry.pumping.clear();
        drop(registry);
        self.changed.notify_waiters();
        records.into_keys().collect()
    }

    pub(crate) fn cursor(&self, owner: &str) -> u64 {
        self.registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cursors
            .get(owner)
            .copied()
            .unwrap_or(0)
    }

    pub(crate) async fn wait(&self, owner: &str, after: u64) {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.cursor(owner) != after {
                return;
            }
            notified.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_update_rebinds_a_view_to_the_new_generation_for_its_owner_only() {
        let views = UserAppViews::default();
        let view = views.open("alice", binding("alice"), "a").unwrap();
        let before = views.cursor("alice");
        assert!(!views.set_generation("bob", &view.view_id, 9));
        assert!(views.set_generation("alice", &view.view_id, 9));
        let (bound, listed) = views.get("alice", &view.view_id).unwrap();
        assert_eq!((bound.generation, listed.generation.as_str()), (9, "9"));
        assert_eq!(views.cursor("alice"), before + 1);
    }
    #[test]
    fn shutdown_revokes_instances_and_refuses_new_open() {
        let views = UserAppViews::default();
        let view = views.open("alice", binding("alice"), "a").unwrap();
        assert_eq!(views.shutdown(), vec![view.view_id.clone()]);
        assert!(views.get("alice", &view.view_id).is_none());
        assert!(views.open("alice", binding("alice"), "a").is_err());
    }
    /// #992 review F3: an App the owner reopens by hand after its browser went
    /// away is not restored a second time, nor saved twice.
    #[test]
    fn an_explicit_reopen_settles_the_pending_restore() {
        let views = UserAppViews::default();
        let hosted = |views: &UserAppViews| {
            let view = views.open("alice", binding("alice"), "a").unwrap();
            views
                .bind_browser(
                    "alice",
                    &view.view_id,
                    format!("target-{}", view.view_id),
                    crate::local::UserAppViewBrowser {
                        tab_id: format!("host-tab-{}", view.view_id),
                        generation: 1,
                    },
                )
                .unwrap();
            view
        };
        let first = hosted(&views);
        assert!(views.suspend("alice", &first.view_id));
        hosted(&views);
        assert_eq!(views.restore_intent("alice"), vec!["todo".to_string()]);
        assert!(views.restore_batch("alice").is_empty());
    }
    /// MP-08/MP-10/MP-11 #992: settle before a close or another browser stop.
    #[test]
    fn explicit_reopen_close_or_stop_does_not_resurrect_saved_tabs() {
        for stop in [false, true] {
            let views = UserAppViews::default();
            views.seed_restore("alice", vec!["todo".into(), "todo".into()]);
            let view = views.open("alice", binding("alice"), "a").unwrap();
            views.bind_browser(
                "alice",
                &view.view_id,
                "target".into(),
                crate::local::UserAppViewBrowser {
                    tab_id: "host-tab-a".into(),
                    generation: 1,
                },
            );
            // The saved duplicate collapsed into one Tab, which the explicit open settles.
            assert_eq!(views.restore_intent("alice"), vec!["todo"]);
            if stop {
                views.suspend("alice", &view.view_id);
            } else {
                views.close("alice", &view.view_id);
            }
            let expected = if stop { vec!["todo"] } else { vec![] };
            assert_eq!(views.restore_intent("alice"), expected);
            assert_eq!(views.restore_batch("alice"), expected);
        }
    }
    #[test]
    fn mp08_a_save_finishes_before_shutdown_and_late_saves_are_refused() {
        let views = UserAppViews::default();
        views.seed_restore("alice", vec!["todo".into(), "documents".into()]);
        let (entered, received) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let saving = views.clone();
        let save = std::thread::spawn(move || {
            saving.save_restore_intent("alice", |intent| {
                assert_eq!(intent, vec!["todo", "documents"]);
                entered.send(()).unwrap();
                released.recv().unwrap();
            })
        });
        received.recv().unwrap();
        let stopping = views.clone();
        let (stopped, result) = std::sync::mpsc::channel();
        let shutdown = std::thread::spawn(move || {
            stopping.shutdown();
            stopped.send(()).unwrap();
        });
        assert!(
            result
                .recv_timeout(std::time::Duration::from_millis(100))
                .is_err(),
            "shutdown waits for the admitted save"
        );
        release.send(()).unwrap();
        save.join().unwrap();
        shutdown.join().unwrap();
        views.save_restore_intent("alice", |_| panic!("late save must not run"));
    }
    #[test]
    fn mp08_an_explicit_open_and_close_fences_an_older_restore_open() {
        let views = UserAppViews::default();
        views.seed_restore("alice", vec!["todo".into()]);
        let restoring = views.open("alice", binding("alice"), "a").unwrap();
        let explicit = views.open("alice", binding("alice"), "a").unwrap();
        let browser = crate::local::UserAppViewBrowser {
            tab_id: "host-tab-explicit".into(),
            generation: 1,
        };
        views
            .bind_browser(
                "alice",
                &explicit.view_id,
                "target-e".into(),
                browser.clone(),
            )
            .unwrap();
        views.close("alice", &explicit.view_id);
        assert!(views
            .bind_restoring_browser("alice", &restoring.view_id, "target-r".into(), browser)
            .is_none());
        assert!(views.restore_intent("alice").is_empty());
    }
    fn binding(owner: &str) -> AppViewBinding {
        AppViewBinding {
            owner: owner.into(),
            installation: "todo".into(),
            generation: 1,
            panel: Default::default(),
            logical_tab: None,
        }
    }
    #[test]
    fn browser_target_lookup_is_scoped_before_selecting_a_view() {
        let views = UserAppViews::default();
        for owner in ["alice", "bob"] {
            let view = views.open(owner, binding(owner), "a").unwrap();
            views
                .bind_browser(
                    owner,
                    &view.view_id,
                    "same-cdp-id".into(),
                    crate::local::UserAppViewBrowser {
                        tab_id: format!("host-tab-{owner}"),
                        generation: 1,
                    },
                )
                .unwrap();
        }
        for owner in ["alice", "bob"] {
            assert_eq!(
                views
                    .browser_view(owner, "same-cdp-id")
                    .unwrap()
                    .browser
                    .unwrap()
                    .tab_id,
                format!("host-tab-{owner}")
            );
        }
        assert!(views.browser_view("eve", "same-cdp-id").is_none());
    }

    #[test]
    fn restore_intent_saved_with_repeat_opens_restores_one_tab_per_app() {
        let views = UserAppViews::default();
        views.seed_restore(
            "alice",
            ["todo", "todo", "docs", "todo", "docs"]
                .map(String::from)
                .to_vec(),
        );
        assert_eq!(views.restore_batch("alice"), vec!["todo", "docs"]);
    }

    #[test]
    fn hosted_finds_only_the_owners_live_tab_of_that_installation() {
        let views = UserAppViews::default();
        let view = views.open("alice", binding("alice"), "a").unwrap();
        assert!(views.hosted("alice", "todo").is_none(), "no Tab yet");
        let browser = crate::local::UserAppViewBrowser {
            tab_id: "host-tab-a".into(),
            generation: 1,
        };
        views
            .bind_browser("alice", &view.view_id, "target".into(), browser)
            .unwrap();
        assert_eq!(
            views.hosted("alice", "todo").unwrap().1.view_id,
            view.view_id
        );
        assert!(views.hosted("bob", "todo").is_none());
        assert!(views.hosted("alice", "docs").is_none());
        views.close("alice", &view.view_id);
        assert!(views.hosted("alice", "todo").is_none());
    }

    #[test]
    fn hosted_binding_is_owner_scoped_and_close_revokes_the_pump() {
        let views = UserAppViews::default();
        let view = views.open("alice", binding("alice"), "a").unwrap();
        let browser = crate::local::UserAppViewBrowser {
            tab_id: "host-tab-a".into(),
            generation: 1,
        };
        assert!(views
            .bind_browser("bob", &view.view_id, "target".into(), browser.clone())
            .is_none());
        assert!(views
            .bind_browser("alice", &view.view_id, "target".into(), browser)
            .is_some());
        assert!(views.browser_view("bob", "target").is_none());
        assert!(views.browser_target("bob", &view.view_id).is_none());
        assert!(views.begin_browser_pump("alice"));
        assert!(!views.begin_browser_pump("alice"));
        assert!(views.keep_browser_pumping("alice"));
        views.close("alice", &view.view_id);
        assert!(!views.keep_browser_pumping("alice"));
        assert!(views.browser_view("alice", "target").is_none());
        assert!(
            views.begin_browser_pump("alice"),
            "new open can acquire the pump after close"
        );
    }
    #[tokio::test]
    async fn owner_scope_lifecycle_limits_and_subscription() {
        let views = UserAppViews::default();
        let view = views.open("alice", binding("alice"), "a").unwrap();
        assert!(views.get("bob", &view.view_id).is_none());
        assert!(!views.close("bob", &view.view_id));
        assert!(views.list("bob").is_empty());
        assert_eq!(views.cursor("bob"), 0);
        let waiter = {
            let views = views.clone();
            tokio::spawn(async move { views.wait("alice", 1).await })
        };
        views.close("alice", &view.view_id);
        tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .unwrap()
            .unwrap();
        assert!(views.get("alice", &view.view_id).is_none());
        for _ in 0..32 {
            views.open("alice", binding("alice"), "a").unwrap();
        }
        assert_eq!(
            views.open("alice", binding("alice"), "a"),
            Err(AppRequestErrorCode::LimitExceeded)
        );
    }

    #[test]
    fn mp08_browser_loss_keeps_app_tab_intent_and_close_ends_it() {
        let views = UserAppViews::default();
        let browser = |n| crate::local::UserAppViewBrowser {
            tab_id: format!("host-tab-{n}"),
            generation: 1,
        };
        let kept = views.open("alice", binding("alice"), "a").unwrap();
        let closed = views.open("alice", binding("alice"), "a").unwrap();
        views.bind_browser("alice", &kept.view_id, "t1".into(), browser(1));
        views.bind_browser("alice", &closed.view_id, "t2".into(), browser(2));
        assert_eq!(views.restore_intent("alice").len(), 2);
        assert!(views.close("alice", &closed.view_id));
        assert!(!views.suspend("bob", &kept.view_id));
        assert!(views.suspend("alice", &kept.view_id));
        assert!(views.get("alice", &kept.view_id).is_none());
        assert_eq!(
            views.restore_intent("alice"),
            vec![kept.installation_id.clone()]
        );
        assert_eq!(views.has_restore("alice"), Some(true));
        // A seed from disk never replaces live intent.
        views.seed_restore("alice", vec!["other".into()]);
        assert!(views.begin_restore("alice"));
        assert!(!views.begin_restore("alice"), "one pass at a time");
        let pending = views.restore_batch("alice");
        assert_eq!(pending, vec![kept.installation_id.clone()]);
        assert_eq!(views.restore_intent("alice"), pending);
        views.end_restore("alice");
        assert!(!views.begin_restore("alice"), "retries wait five seconds");
        assert_eq!(views.restore_intent("bob"), Vec::<String>::new());
    }
}
