//! MP-08/MP-11: focus or an owner-requested prompt grants resources; task
//! lifetime retains that exact grant until idle, expiry or revocation.
use super::browser_controller_process::BrowserCancellation;
use crate::local::{UserDomainGrant, UserDomainNotice, UserDomainResource};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Owner decision 2026-10-07: grants last 8 hours by default, 24 hours at most.
pub(crate) const DEFAULT_GRANT_LIFETIME: Duration = Duration::from_secs(8 * 3600);
pub(crate) const MAX_GRANT_LIFETIME: Duration = Duration::from_secs(24 * 3600);
const MAX_TRANSFER_RESOURCES: usize = 64;

/// MP-08/MP-11: why a grant exists. Agents can never mint one without a cause.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GrantCause {
    Focus,
    Prompt(String),
    Delegated(String),
}

pub(crate) struct Grant {
    pub(crate) epoch: Arc<BrowserCancellation>,
    pub(crate) resources: BTreeSet<UserDomainResource>,
    pub(crate) subscriptions: BTreeMap<String, String>,
    pub(crate) subscription_owner: String,
    subscription_deadlines: BTreeMap<String, Instant>,
    pub(crate) cause: GrantCause,
    /// None for focus: the owner's live act retires through idle or revoke.
    pub(crate) expires_at: Option<Instant>,
    expires_at_ms: Option<u64>,
    session: String,
    since_ms: u64,
    idle_since: Option<Instant>,
    idle_since_ms: Option<u64>,
}

impl Grant {
    fn new(cause: GrantCause, lifetime: Option<Duration>) -> Self {
        let lifetime = lifetime.map(|lifetime| lifetime.min(MAX_GRANT_LIFETIME));
        let now_ms = crate::session::unix_epoch_ms();
        Self {
            epoch: Arc::new(BrowserCancellation::default()),
            resources: BTreeSet::new(),
            subscriptions: BTreeMap::new(),
            subscription_deadlines: BTreeMap::new(),
            subscription_owner: format!("grant-{:032x}", rand::random::<u128>()),
            cause,
            expires_at: lifetime.map(|lifetime| Instant::now() + lifetime),
            expires_at_ms: lifetime
                .map(|lifetime| now_ms.saturating_add(lifetime.as_millis() as u64)),
            session: String::new(),
            since_ms: now_ms,
            idle_since: Some(Instant::now()),
            idle_since_ms: Some(now_ms),
        }
    }
    fn live(&self, now: Instant) -> bool {
        self.expires_at.is_none_or(|expires_at| now < expires_at)
    }
}

pub(crate) struct GrantRetirement {
    pub(crate) agent: String,
    pub(crate) epoch: Arc<BrowserCancellation>,
    pub(crate) scope: String,
    pub(crate) subscriptions: Vec<String>,
}

#[derive(Default)]
pub(crate) struct UserDomainAccess {
    focus: BTreeMap<String, String>,
    grants: BTreeMap<(String, String), Grant>,
    cursors: BTreeMap<String, u64>,
    /// MP-11: revokes per (owner, agent); agent "" counts owner-wide revokes.
    fences: BTreeMap<(String, String), u64>,
    notices: BTreeMap<String, UserDomainNotice>,
}

impl UserDomainAccess {
    pub(crate) fn focus(&mut self, user: &str, agent: Option<&str>) {
        if self.focused(user) == agent
            && agent.is_none_or(|agent| {
                self.grant(user, agent)
                    .is_ok_and(|grant| grant.cause == GrantCause::Focus)
            })
        {
            return;
        }
        if let Some(agent) = agent {
            self.focus.insert(user.into(), agent.into());
            let key = (user.to_string(), agent.to_string());
            if self
                .grants
                .get(&key)
                .is_some_and(|grant| !grant.live(Instant::now()))
            {
                self.revoke(user, Some(agent));
                self.focus.insert(user.into(), agent.into());
            }
            let grant = self
                .grants
                .entry(key)
                .or_insert_with(|| Grant::new(GrantCause::Focus, None));
            // MP-08/MP-11: explicit owner focus supersedes prompt/delegation
            // expiry while preserving resources, subscriptions and the epoch.
            grant.cause = GrantCause::Focus;
            grant.expires_at = None;
            grant.expires_at_ms = None;
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
            .filter(|grant| grant.live(Instant::now()))
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
    /// MP-08/MP-11: an owner-requested prompt grants the agent its own grant
    /// without focus. Nothing is claimed; only tabs it opens join the grant.
    /// An existing live grant keeps its cause, resources and expiry.
    pub(crate) fn request(
        &mut self,
        user: &str,
        agent: &str,
        prompt_id: &str,
        lifetime: Duration,
    ) -> bool {
        if self.grant(user, agent).is_ok() {
            return false;
        }
        if self.grants.contains_key(&(user.into(), agent.into())) {
            self.revoke(user, Some(agent));
        }
        self.grants.insert(
            (user.into(), agent.into()),
            Grant::new(GrantCause::Prompt(prompt_id.into()), Some(lifetime)),
        );
        self.changed(user);
        true
    }
    /// MP-08/MP-11: a parent hands its direct child an explicit subset of its
    /// own granted resources. The child grant cannot outlive or widen it.
    pub(crate) fn transfer(
        &mut self,
        user: &str,
        parent: &str,
        child: &str,
        resources: &[UserDomainResource],
    ) -> Result<(), String> {
        let source = self.grant(user, parent)?;
        if resources.is_empty()
            || resources.len() > MAX_TRANSFER_RESOURCES
            || resources.iter().any(|r| !source.resources.contains(r))
        {
            return Err("MP-08: not_granted: transfer must name resources the caller holds".into());
        }
        let expires_at = source.expires_at;
        let expires_at_ms = source.expires_at_ms;
        let session = source.session.clone();
        let cause = GrantCause::Delegated(parent.into());
        if let Ok(existing) = self.grant(user, child) {
            if existing.cause != cause {
                return Err(
                    "MP-08: not_requested: child already holds its own user-domain grant".into(),
                );
            }
        } else {
            self.revoke(user, Some(child));
            let mut grant = Grant::new(cause, None);
            grant.expires_at = expires_at;
            grant.expires_at_ms = expires_at_ms;
            grant.session = session;
            self.grants.insert((user.into(), child.into()), grant);
        }
        let grant = self.grants.get_mut(&(user.into(), child.into())).unwrap();
        grant.resources.extend(resources.iter().cloned());
        self.changed(user);
        Ok(())
    }
    pub(crate) fn next_expiry(&self, idle_window: Duration) -> Option<Instant> {
        self.grants
            .values()
            .flat_map(|grant| {
                grant
                    .expires_at
                    .into_iter()
                    .chain(grant.idle_since.map(|since| since + idle_window))
                    .chain(grant.subscription_deadlines.values().copied())
            })
            .min()
    }
    pub(crate) fn due_retention(
        &self,
        now: Instant,
        idle_window: Duration,
    ) -> Vec<(String, String)> {
        self.grants
            .iter()
            .filter(|(_, grant)| {
                !grant.live(now)
                    || grant
                        .idle_since
                        .is_some_and(|since| now.saturating_duration_since(since) >= idle_window)
            })
            .map(|(key, _)| key.clone())
            .collect()
    }
    pub(crate) fn expired_subscriptions(&mut self, now: Instant) -> Vec<(String, Vec<String>)> {
        let mut expired = Vec::new();
        for ((user, _), grant) in &mut self.grants {
            let ids = grant
                .subscription_deadlines
                .iter()
                .filter(|(_, deadline)| now >= **deadline)
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            for id in &ids {
                grant.subscription_deadlines.remove(id);
                grant.subscriptions.remove(id);
            }
            if !ids.is_empty() {
                expired.push((user.clone(), ids));
            }
        }
        expired
    }
    /// MP-11: keep exact old scopes until a replacing mutation has retired them.
    pub(crate) fn retirement_snapshot(&self, user: &str, agent: &str) -> Vec<GrantRetirement> {
        self.revocation_set(user, Some(agent))
            .into_iter()
            .filter_map(|key| {
                self.grants.get(&key).map(|grant| GrantRetirement {
                    agent: key.1,
                    epoch: grant.epoch.clone(),
                    scope: grant.subscription_owner.clone(),
                    subscriptions: grant.subscriptions.keys().cloned().collect(),
                })
            })
            .collect()
    }
    pub(crate) fn raw_epoch(&self, user: &str, agent: &str) -> Option<Arc<BrowserCancellation>> {
        self.grants
            .get(&(user.into(), agent.into()))
            .map(|grant| grant.epoch.clone())
    }
    /// Also returns expired grants: their subscriptions still need closing.
    pub(crate) fn subscription_owner(&self, user: &str, agent: &str) -> Option<String> {
        self.grants
            .get(&(user.into(), agent.into()))
            .map(|grant| grant.subscription_owner.clone())
    }
    /// MP-08/MP-11: holders whose absolute lifetime has elapsed.
    #[cfg(test)]
    pub(crate) fn due(&self, now: Instant) -> Vec<(String, String)> {
        self.grants
            .iter()
            .filter(|(_, grant)| !grant.live(now))
            .map(|(key, _)| key.clone())
            .collect()
    }
    /// MP-08/MP-11: explicit open is an owner-authorized creation. It grants
    /// only the resulting tab; it cannot claim unrelated existing resources.
    pub(crate) fn opened_tab(
        &mut self,
        user: &str,
        agent: &str,
        tab_id: &str,
    ) -> Result<(), String> {
        if matches!(self.grant(user, agent)?.cause, GrantCause::Delegated(_))
            && self.focused(user) != Some(agent)
        {
            return Err(NOT_REQUESTED_OPEN.into());
        }
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
    /// MP-08/MP-11: the holders a revoke removes, including grants delegated
    /// from them, transitively.
    pub(crate) fn revocation_set(&self, user: &str, agent: Option<&str>) -> Vec<(String, String)> {
        let mut holders: Vec<_> = self
            .grants
            .keys()
            .filter(|(owner, id)| owner == user && agent.is_none_or(|agent| id == agent))
            .cloned()
            .collect();
        let mut index = 0;
        while index < holders.len() {
            let parent = holders[index].1.clone();
            for (key, grant) in &self.grants {
                if key.0 == user
                    && grant.cause == GrantCause::Delegated(parent.clone())
                    && !holders.contains(key)
                {
                    holders.push(key.clone());
                }
            }
            index += 1;
        }
        holders
    }
    pub(crate) fn revoke(&mut self, user: &str, agent: Option<&str>) -> Vec<String> {
        let holders = self.revocation_set(user, agent);
        let target = (user.to_string(), agent.unwrap_or_default().to_string());
        for key in holders.iter().chain([&target]) {
            *self.fences.entry(key.clone()).or_default() += 1;
        }
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
        // MP-11: even an empty explicit revoke invalidates pending acquisition.
        self.changed(user);
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
    /// MP-11: fences a pending acquisition for this owner and agent. Only a
    /// revoke reaching this agent (direct, cascaded or owner-wide) moves it;
    /// other holders' grant changes do not.
    pub(crate) fn acquisition_fence(&self, user: &str, agent: &str) -> u64 {
        let fence = |agent: &str| {
            self.fences
                .get(&(user.to_string(), agent.to_string()))
                .copied()
                .unwrap_or(0)
        };
        fence("") + fence(agent)
    }
    pub(crate) fn cursor(&self, user: &str) -> u64 {
        self.cursors.get(user).copied().unwrap_or(0)
    }
    fn changed(&mut self, user: &str) {
        *self.cursors.entry(user.into()).or_default() += 1;
    }
    pub(crate) fn snapshot(
        &self,
        user: &str,
        kernel: &str,
        idle_window: Duration,
    ) -> serde_json::Value {
        let now = Instant::now();
        let grants: Vec<_> = self.grants.iter().filter(|((owner, _), grant)| owner == user && grant.live(now)).map(|((_, agent), grant)| UserDomainGrant {
            agent_id: agent.clone(), session_id: grant.session.clone(), kernel_id: kernel.into(),
            resources: grant.resources.iter().cloned().collect(), since_ms: grant.since_ms,
            focused: self.focused(user) == Some(agent.as_str()),
            idle_since_ms: grant.idle_since_ms, idle_timeout_seconds: idle_window.as_secs(),
            expiry_rule: "retained during active turn or pending wake; expires after fully idle window, absolute lifetime, session/agent end or parent revocation".into(),
            prompt_id: match &grant.cause { GrantCause::Prompt(id) => Some(id.clone()), _ => None },
            delegated_by_agent_id: match &grant.cause { GrantCause::Delegated(id) => Some(id.clone()), _ => None },
            expires_at_ms: grant.expires_at_ms,
        }).collect();
        serde_json::json!({"event":"user_domain_grants_changed","cursor":self.cursor(user),"grants":grants,"notice":self.notices.get(user)})
    }
}

pub(crate) const NOT_REQUESTED_LOAD: &str =
    "MP-08: not_requested: user-domain access requires the user's request in this turn or focus; ask the user";
pub(crate) const NOT_REQUESTED_OPEN: &str =
    "MP-08: not_requested: delegated user-domain grants cannot acquire new resources";

#[cfg(test)]
mod tests {
    use super::*;
    fn tab(id: &str) -> UserDomainResource {
        UserDomainResource::BrowserTab { tab_id: id.into() }
    }
    /// MP-08/MP-11 (#922 round 3): owner focus supersedes a timed cause
    /// without retiring the grant's resources, subscriptions or epoch.
    #[test]
    fn capability_review_round3_focus_preserves_requested_and_delegated_grants() {
        for agent in ["parent", "child"] {
            let mut access = UserDomainAccess::default();
            access.request("owner", "parent", "prompt", DEFAULT_GRANT_LIFETIME);
            access.opened_tab("owner", "parent", "tab").unwrap();
            access
                .transfer("owner", "parent", "child", &[tab("tab")])
                .unwrap();
            access.subscribe("owner", agent, "stream", "tab").unwrap();
            let grant = access.grant("owner", agent).unwrap();
            let deadline = grant.expires_at.unwrap();
            let epoch = grant.epoch.clone();
            let scope = grant.subscription_owner.clone();

            access.focus("owner", Some(agent));
            access.focus("owner", Some(agent));
            let grant = access.grant("owner", agent).unwrap();
            assert_eq!(grant.cause, GrantCause::Focus);
            assert!(grant.expires_at.is_none());
            assert!(grant.expires_at_ms.is_none());
            assert!(Arc::ptr_eq(&grant.epoch, &epoch));
            assert!(!epoch.requested());
            assert_eq!(grant.subscription_owner, scope);
            assert_eq!(grant.resources, BTreeSet::from([tab("tab")]));
            assert_eq!(grant.subscriptions.get("stream"), Some(&"tab".into()));
            assert!(!access
                .due(deadline + Duration::from_secs(1))
                .contains(&("owner".into(), agent.into())));
            assert_eq!(access.focused("owner"), Some(agent));
            if agent == "child" {
                access.revoke("owner", Some("parent"));
                assert!(access.grant("owner", agent).is_ok());
                assert_eq!(access.focused("owner"), Some(agent));
            }
        }
    }
    /// MP-08 (#922 review 4): focus is the owner's live act; its grant has
    /// no absolute lifetime and retires only through idle, revoke or end.
    #[test]
    fn capability_focus_grants_have_no_absolute_lifetime() {
        let mut access = UserDomainAccess::default();
        access.focus("owner", Some("focused"));
        let later = Instant::now() + MAX_GRANT_LIFETIME + Duration::from_secs(60);
        assert!(access.due(later).is_empty());
        assert!(access
            .due_retention(later, MAX_GRANT_LIFETIME * 2)
            .is_empty());
        let snapshot = access.snapshot("owner", "kernel", Duration::from_secs(60));
        assert!(snapshot["grants"][0].get("expires_at_ms").is_none());
        assert_eq!(access.focused("owner"), Some("focused"));
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
    fn capability_prompt_grant_transfer_subset_and_cascading_revoke() {
        let mut access = UserDomainAccess::default();
        assert!(access.request("owner", "parent", "prompt-1", DEFAULT_GRANT_LIFETIME));
        assert!(!access.request("owner", "parent", "prompt-2", DEFAULT_GRANT_LIFETIME));
        assert_eq!(
            access.grant("owner", "parent").unwrap().cause,
            GrantCause::Prompt("prompt-1".into())
        );
        assert!(access
            .claim("owner", "parent", tab("existing"), false)
            .is_err());
        access.opened_tab("owner", "parent", "opened").unwrap();
        assert!(access
            .transfer("owner", "parent", "child", &[tab("existing")])
            .is_err());
        assert!(access.transfer("owner", "parent", "child", &[]).is_err());
        access
            .transfer("owner", "parent", "child", &[tab("opened")])
            .unwrap();
        let child = access.grant("owner", "child").unwrap();
        assert_eq!(child.cause, GrantCause::Delegated("parent".into()));
        assert_eq!(
            child.expires_at,
            access.grant("owner", "parent").unwrap().expires_at
        );
        assert!(access.opened_tab("owner", "child", "widened").is_err());
        access
            .transfer("owner", "child", "grandchild", &[tab("opened")])
            .unwrap();
        access.focus("owner", Some("other"));
        assert!(access.transfer("owner", "other", "child", &[]).is_err());
        let epoch = access.grant("owner", "grandchild").unwrap().epoch.clone();
        access.revoke("owner", Some("parent"));
        assert!(epoch.requested(), "revocation cascades to delegated grants");
        assert!(access.grant("owner", "child").is_err());
        assert!(access.grant("owner", "grandchild").is_err());
        assert!(access.grant("owner", "other").is_ok());
    }
    #[test]
    fn capability_replacing_expired_child_retires_its_old_scope_and_descendants() {
        let mut access = UserDomainAccess::default();
        access.request("owner", "parent", "request", DEFAULT_GRANT_LIFETIME);
        access.opened_tab("owner", "parent", "tab").unwrap();
        access
            .transfer("owner", "parent", "child", &[tab("tab")])
            .unwrap();
        access
            .subscribe("owner", "child", "old-stream", "tab")
            .unwrap();
        access
            .transfer("owner", "child", "grandchild", &[tab("tab")])
            .unwrap();
        let retired = access.retirement_snapshot("owner", "child");
        assert_eq!(retired.len(), 2);
        access
            .grants
            .get_mut(&("owner".into(), "child".into()))
            .unwrap()
            .expires_at = Some(Instant::now());
        access
            .transfer("owner", "parent", "child", &[tab("tab")])
            .unwrap();
        assert!(retired.iter().all(|old| old.epoch.requested()));
        assert!(retired
            .iter()
            .any(|old| old.subscriptions == ["old-stream"]));
        assert!(access.grant("owner", "grandchild").is_err());
        assert!(retired
            .iter()
            .all(|old| old.scope != access.subscription_owner("owner", "child").unwrap()));
    }
    #[test]
    fn capability_expired_grants_refuse_use_and_are_due() {
        let mut access = UserDomainAccess::default();
        access.request("owner", "agent", "prompt", MAX_GRANT_LIFETIME * 2);
        let expires = access.grant("owner", "agent").unwrap().expires_at;
        assert!(expires.is_some_and(|expires| expires <= Instant::now() + MAX_GRANT_LIFETIME));
        access
            .grants
            .get_mut(&("owner".into(), "agent".into()))
            .unwrap()
            .expires_at = Some(Instant::now());
        assert!(access.grant("owner", "agent").is_err());
        assert_eq!(
            access.due(Instant::now()),
            [("owner".into(), "agent".into())]
        );
        assert!(access.request("owner", "agent", "fresh", DEFAULT_GRANT_LIFETIME));
        assert!(access.grant("owner", "agent").is_ok());
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
