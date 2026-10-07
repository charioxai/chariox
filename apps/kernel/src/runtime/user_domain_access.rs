//! MP-08/MP-11: focus claims resources; task lifetime retains that exact grant.
use super::browser_controller_process::BrowserCancellation;
use crate::local::{UserDomainGrant, UserDomainNotice, UserDomainResource};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(crate) struct Grant {
    pub(crate) epoch: Arc<BrowserCancellation>,
    pub(crate) resources: BTreeSet<UserDomainResource>,
    pub(crate) subscriptions: BTreeMap<String, String>,
    pub(crate) subscription_owner: String,
    subscription_deadlines: BTreeMap<String, Instant>,
    session: String,
    since_ms: u64,
    idle_since: Option<Instant>,
    idle_since_ms: Option<u64>,
}

#[derive(Default)]
pub(crate) struct UserDomainAccess {
    focus: BTreeMap<String, String>,
    grants: BTreeMap<(String, String), Grant>,
    cursors: BTreeMap<String, u64>,
    notices: BTreeMap<String, UserDomainNotice>,
}

impl UserDomainAccess {
    pub(crate) fn focus(&mut self, user: &str, agent: Option<&str>) {
        if self.focused(user) == agent {
            return;
        }
        if let Some(agent) = agent {
            self.focus.insert(user.into(), agent.into());
            self.grants
                .entry((user.into(), agent.into()))
                .or_insert_with(|| Grant {
                    epoch: Arc::new(BrowserCancellation::default()),
                    resources: BTreeSet::new(),
                    subscriptions: BTreeMap::new(),
                    subscription_deadlines: BTreeMap::new(),
                    subscription_owner: format!("grant-{:032x}", rand::random::<u128>()),
                    session: String::new(),
                    since_ms: crate::session::unix_epoch_ms(),
                    idle_since: Some(Instant::now()),
                    idle_since_ms: Some(crate::session::unix_epoch_ms()),
                });
        } else {
            self.focus.remove(user);
        }
        self.changed(user);
    }
    pub(crate) fn focused(&self, user: &str) -> Option<&str> {
        self.focus.get(user).map(String::as_str)
    }
    pub(crate) fn grant(&self, user: &str, agent: &str) -> Result<&Grant, String> {
        self.grants
            .get(&(user.into(), agent.into()))
            .ok_or_else(|| {
                "MP-08: not_granted: user-domain access expired or revoked; focus this agent again"
                    .into()
            })
    }
    pub(crate) fn claim(
        &mut self,
        user: &str,
        agent: &str,
        resource: UserDomainResource,
        sensitive: bool,
    ) -> Result<(), String> {
        self.grant(user, agent)?;
        let focused = self.focused(user) == Some(agent);
        if sensitive && !focused {
            return Err("MP-11: sensitive_requires_focus: sensitive user-domain action requires focus or human approval; focus this agent".into());
        }
        let grant = self
            .grants
            .get_mut(&(user.into(), agent.into()))
            .ok_or("MP-08: not_granted: user-domain grant revoked")?;
        if !grant.resources.contains(&resource) {
            if !focused {
                return Err(
                    "MP-08: not_focused_agent: new user-domain resource requires focus; focus this agent".into(),
                );
            }
            if grant.resources.len() >= 1024 {
                return Err(
                    "MP-08: user-domain resource limit reached; revoke unused grants".into(),
                );
            }
            grant.resources.insert(resource);
            self.changed(user);
        }
        Ok(())
    }
    /// MP-08/MP-11: explicit open is an owner-authorized creation. It grants
    /// only the resulting tab; it cannot claim unrelated existing resources.
    pub(crate) fn opened_tab(
        &mut self,
        user: &str,
        agent: &str,
        tab_id: &str,
    ) -> Result<(), String> {
        self.grant(user, agent)?;
        let grant = self.grants.get_mut(&(user.into(), agent.into())).unwrap();
        let resource = UserDomainResource::BrowserTab {
            tab_id: tab_id.into(),
        };
        if !grant.resources.contains(&resource) {
            if grant.resources.len() >= 1024 {
                return Err(
                    "MP-08: user-domain resource limit reached; revoke unused grants".into(),
                );
            }
            grant.resources.insert(resource);
            self.changed(user);
        }
        Ok(())
    }
    pub(crate) fn bind(
        &mut self,
        user: &str,
        agent: &str,
        session: &str,
        busy: bool,
        (now, now_ms): (Instant, u64),
        idle_window: Duration,
    ) -> bool {
        let Some(grant) = self.grants.get_mut(&(user.into(), agent.into())) else {
            return false;
        };
        let changed = grant.session != session || busy == grant.idle_since.is_some();
        grant.session = session.into();
        if busy {
            grant.idle_since = None;
            grant.idle_since_ms = None;
        } else if grant.idle_since.is_none() {
            grant.idle_since = Some(now);
            grant.idle_since_ms = Some(now_ms);
        }
        if grant
            .idle_since
            .is_some_and(|since| now.saturating_duration_since(since) >= idle_window)
        {
            self.revoke(user, Some(agent));
            return true;
        }
        if changed {
            self.changed(user);
        }
        false
    }
    pub(crate) fn holders(&self) -> Vec<(String, String)> {
        self.grants.keys().cloned().collect()
    }
    pub(crate) fn revoke(&mut self, user: &str, agent: Option<&str>) -> Vec<String> {
        let holders: Vec<_> = self
            .grants
            .keys()
            .filter(|(owner, id)| owner == user && agent.is_none_or(|agent| id == agent))
            .cloned()
            .collect();
        let mut subscriptions = Vec::new();
        for key in &holders {
            if let Some(grant) = self.grants.remove(key) {
                grant.epoch.request_cancel();
                subscriptions.extend(grant.subscriptions.into_keys());
            }
            if self.focused(user) == Some(key.1.as_str()) {
                self.focus.remove(user);
            }
        }
        if !holders.is_empty() {
            self.changed(user);
        }
        subscriptions
    }
    pub(crate) fn subscribe(
        &mut self,
        user: &str,
        agent: &str,
        id: &str,
        tab: &str,
    ) -> Result<(), String> {
        let grant = self
            .grants
            .get_mut(&(user.into(), agent.into()))
            .ok_or("MP-08: not_granted: revoked subscription")?;
        if grant.subscriptions.len() >= 16 {
            return Err("MP-08: user-domain subscription limit reached".into());
        }
        grant.subscriptions.insert(id.into(), tab.into());
        grant
            .subscription_deadlines
            .insert(id.into(), Instant::now() + Duration::from_secs(60));
        Ok(())
    }
    pub(crate) fn unsubscribe(&mut self, user: &str, agent: &str, id: &str) {
        if let Some(grant) = self.grants.get_mut(&(user.into(), agent.into())) {
            grant.subscriptions.remove(id);
            grant.subscription_deadlines.remove(id);
        }
    }
    pub(crate) fn prune_subscriptions(&mut self, user: &str, agent: &str, now: Instant) {
        if let Some(grant) = self.grants.get_mut(&(user.into(), agent.into())) {
            grant
                .subscription_deadlines
                .retain(|_, deadline| now < *deadline);
            grant
                .subscriptions
                .retain(|id, _| grant.subscription_deadlines.contains_key(id));
        }
    }
    pub(crate) fn subscription_polled(&mut self, user: &str, agent: &str, id: &str) {
        if let Some(grant) = self.grants.get_mut(&(user.into(), agent.into())) {
            if let Some(deadline) = grant.subscription_deadlines.get_mut(id) {
                *deadline = Instant::now() + Duration::from_secs(60);
            }
        }
    }
    pub(crate) fn acted(&mut self, user: &str, agent: &str, resource: UserDomainResource) {
        if self.focused(user) != Some(agent) {
            self.notices.insert(
                user.into(),
                UserDomainNotice {
                    agent_id: agent.into(),
                    resource,
                    at_ms: crate::session::unix_epoch_ms(),
                },
            );
            self.changed(user);
        }
    }
    pub(crate) fn cursor(&self, user: &str) -> u64 {
        self.cursors.get(user).copied().unwrap_or(0)
    }
    pub(crate) fn changed(&mut self, user: &str) {
        *self.cursors.entry(user.into()).or_default() += 1;
    }
    pub(crate) fn snapshot(
        &self,
        user: &str,
        kernel: &str,
        idle_window: Duration,
    ) -> serde_json::Value {
        let grants: Vec<_> = self.grants.iter().filter(|((owner, _), _)| owner == user).map(|((_, agent), grant)| UserDomainGrant {
            agent_id: agent.clone(), session_id: grant.session.clone(), kernel_id: kernel.into(),
            resources: grant.resources.iter().cloned().collect(), since_ms: grant.since_ms,
            focused: self.focused(user) == Some(agent.as_str()),
            idle_since_ms: grant.idle_since_ms, idle_timeout_seconds: idle_window.as_secs(),
            expiry_rule: "retained during active turn or pending wake; expires after fully idle window or session/agent end".into(),
        }).collect();
        serde_json::json!({"event":"user_domain_grants_changed","cursor":self.cursor(user),"grants":grants,"notice":self.notices.get(user)})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tab(id: &str) -> UserDomainResource {
        UserDomainResource::BrowserTab { tab_id: id.into() }
    }
    #[test]
    fn mdaccess_focus_retains_only_claimed_resources_and_sensitive_requires_focus() {
        let mut access = UserDomainAccess::default();
        access.focus("owner", Some("first"));
        access.claim("owner", "first", tab("a"), false).unwrap();
        let epoch = access.grant("owner", "first").unwrap().epoch.clone();
        access.focus("owner", Some("second"));
        assert!(!epoch.requested());
        assert!(access.claim("owner", "first", tab("a"), false).is_ok());
        assert!(matches!(
            crate::error::HostFailure::from(
                access.claim("owner", "first", tab("b"), false).unwrap_err()
            ),
            crate::error::HostFailure::Refused(
                crate::error::UserDomainRefusalReason::NotFocusedAgent
            )
        ));
        assert!(matches!(
            crate::error::HostFailure::from(
                access.claim("owner", "first", tab("a"), true).unwrap_err()
            ),
            crate::error::HostFailure::Refused(
                crate::error::UserDomainRefusalReason::SensitiveRequiresFocus
            )
        ));
        assert!(access.claim("owner", "second", tab("b"), false).is_ok());
        assert!(matches!(
            crate::error::HostFailure::from(
                access
                    .claim("stranger", "first", tab("a"), false)
                    .unwrap_err()
            ),
            crate::error::HostFailure::Refused(crate::error::UserDomainRefusalReason::NotGranted)
        ));
    }
    #[test]
    fn mdaccess_explicit_open_grants_only_its_created_tab() {
        let mut access = UserDomainAccess::default();
        access.focus("owner", Some("first"));
        access.focus("owner", Some("second"));
        access.opened_tab("owner", "first", "created").unwrap();
        assert!(access
            .claim("owner", "first", tab("created"), false)
            .is_ok());
        assert!(access
            .claim("owner", "first", tab("unrelated"), false)
            .unwrap_err()
            .contains("not_focused_agent"));
        assert_eq!(access.grant("owner", "first").unwrap().resources.len(), 1);
        access.revoke("owner", Some("first"));
        assert!(access
            .opened_tab("owner", "first", "late")
            .unwrap_err()
            .contains("not_granted"));
    }
    #[test]
    fn mdaccess_subscription_expiry_reclaims_slots_and_poll_renews() {
        let mut access = UserDomainAccess::default();
        access.focus("owner", Some("agent"));
        for i in 0..16 {
            access
                .subscribe("owner", "agent", &i.to_string(), "tab")
                .unwrap();
        }
        assert!(access.subscribe("owner", "agent", "new", "tab").is_err());
        let initial = access
            .grant("owner", "agent")
            .unwrap()
            .subscription_deadlines["0"];
        access.subscription_polled("owner", "agent", "0");
        assert!(
            access
                .grant("owner", "agent")
                .unwrap()
                .subscription_deadlines["0"]
                >= initial
        );
        access.unsubscribe("owner", "agent", "0");
        access.subscribe("owner", "agent", "new", "tab").unwrap();
        access.prune_subscriptions("owner", "agent", Instant::now() + Duration::from_secs(61));
        assert!(access
            .grant("owner", "agent")
            .unwrap()
            .subscriptions
            .is_empty());
        access.subscribe("owner", "agent", "fresh", "tab").unwrap();
    }
    #[test]
    fn mdaccess_yield_and_wake_retain_grant_idle_expires_and_revoke_cancels_subscribers() {
        let mut access = UserDomainAccess::default();
        let now = Instant::now();
        let window = Duration::from_secs(1800);
        access.focus("owner", Some("first"));
        let epoch = access.grant("owner", "first").unwrap().epoch.clone();
        access.subscribe("owner", "first", "stream", "a").unwrap();
        access.bind("owner", "first", "session", true, (now, 1), window);
        access.focus("owner", Some("second"));
        // The kernel reports pending wake as busy, even with no executing turn.
        access.bind(
            "owner",
            "first",
            "session",
            true,
            (now + window * 2, 2),
            window,
        );
        assert!(!epoch.requested());
        let idle = now + window * 3;
        assert!(!access.bind("owner", "first", "session", false, (idle, 3), window));
        assert!(!access.bind(
            "owner",
            "first",
            "session",
            false,
            (idle + window - Duration::from_millis(1), 4),
            window
        ));
        assert!(access.bind(
            "owner",
            "first",
            "session",
            false,
            (idle + window, 5),
            window
        ));
        assert!(epoch.requested());
        assert!(access.grant("owner", "first").is_err());
        let second = access.grant("owner", "second").unwrap().epoch.clone();
        access
            .subscribe("owner", "second", "idle-stream", "b")
            .unwrap();
        assert_eq!(access.revoke("owner", Some("second")), ["idle-stream"]);
        assert!(second.requested());
        access.focus("owner", Some("second"));
        assert!(!Arc::ptr_eq(
            &second,
            &access.grant("owner", "second").unwrap().epoch
        ));
    }
}
