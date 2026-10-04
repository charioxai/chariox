//! Kernel-owned, ephemeral user-domain instances. No Session or Room exists
//! for these identifiers. Owner checks are repeated for every lookup/close.
use super::app_views::AppViewBinding;
use crate::local::{AppRequestErrorCode, UserAppView};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Registry {
    views: BTreeMap<String, (AppViewBinding, UserAppView)>,
    cursors: HashMap<String, u64>,
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

    pub(crate) fn close(&self, owner: &str, id: &str) -> bool {
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        if !registry
            .views
            .get(id)
            .is_some_and(|(b, _)| b.owner == owner)
        {
            return false;
        }
        registry.views.remove(id);
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
    fn binding(owner: &str) -> AppViewBinding {
        AppViewBinding {
            owner: owner.into(),
            installation: "todo".into(),
            generation: 1,
            panel: Default::default(),
            logical_tab: None,
        }
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
