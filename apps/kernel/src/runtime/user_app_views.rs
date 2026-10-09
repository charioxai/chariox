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
}

#[derive(Clone, Default)]
pub(crate) struct UserAppViews {
    registry: Arc<Mutex<Registry>>,
    changed: Arc<tokio::sync::Notify>,
}

impl UserAppViews {
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
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        let (binding, view) = registry.views.get_mut(id)?;
        if binding.owner != owner || view.browser.is_some() {
            return None;
        }
        view.browser = Some(browser);
        let view = view.clone();
        registry.browser_targets.insert(id.into(), target);
        *registry.cursors.entry(owner.into()).or_default() += 1;
        drop(registry);
        self.changed.notify_waiters();
        Some(view)
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
        if registry.views.get(id).is_none_or(|(b, _)| b.owner != owner) {
            return false;
        }
        registry.views.remove(id);
        registry.browser_targets.remove(id);
        *registry.cursors.entry(owner.into()).or_default() += 1;
        drop(registry);
        self.changed.notify_waiters();
        true
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
    fn shutdown_revokes_instances_and_refuses_new_open() {
        let views = UserAppViews::default();
        let view = views.open("alice", binding("alice"), "a").unwrap();
        assert_eq!(views.shutdown(), vec![view.view_id.clone()]);
        assert!(views.get("alice", &view.view_id).is_none());
        assert!(views.open("alice", binding("alice"), "a").is_err());
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
}
