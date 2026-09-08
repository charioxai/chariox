//! One poller per pending agent. A Deferred result retains its original clock;
//! calling remember again cannot renew the deadline or create another task.
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::Instant;

#[derive(Debug, Clone, Default)]
pub(in crate::runtime::state) struct PendingPollers(Arc<Mutex<BTreeMap<String, Arc<()>>>>);
pub(in crate::runtime::state) struct PendingPoller {
    owner: PendingPollers,
    agent: String,
    identity: Arc<()>,
    deadline: Instant,
    next_attempt: Instant,
    interval: Duration,
}
impl PendingPollers {
    /// Call with the pending-map lock held. Finishing must release under the
    /// same map lock, so a concurrent remember cannot lose its wakeup.
    pub(in crate::runtime::state) fn claim(&self, agent: &str) -> Option<PendingPoller> {
        self.claim_with_clock(
            agent,
            Instant::now(),
            Duration::from_secs(120),
            Duration::from_millis(500),
        )
    }
    fn claim_with_clock(
        &self,
        agent: &str,
        now: Instant,
        budget: Duration,
        interval: Duration,
    ) -> Option<PendingPoller> {
        let mut owners = self.0.lock().expect("pending poller mutex poisoned");
        if owners.contains_key(agent) {
            return None;
        }
        let identity = Arc::new(());
        owners.insert(agent.to_owned(), identity.clone());
        Some(PendingPoller {
            owner: self.clone(),
            agent: agent.to_owned(),
            identity,
            deadline: now + budget,
            next_attempt: now,
            interval,
        })
    }
}
impl PendingPoller {
    pub(in crate::runtime::state) async fn next(&mut self) -> bool {
        tokio::time::sleep_until(self.next_attempt.min(self.deadline)).await;
        self.admit_at(Instant::now())
    }
    fn admit_at(&mut self, now: Instant) -> bool {
        if now < self.next_attempt || now >= self.deadline {
            return false;
        }
        self.next_attempt = now + self.interval;
        true
    }
    pub(in crate::runtime::state) fn release(&mut self) {
        let mut owners = self.owner.0.lock().expect("pending poller mutex poisoned");
        if owners
            .get(&self.agent)
            .is_some_and(|identity| Arc::ptr_eq(identity, &self.identity))
        {
            owners.remove(&self.agent);
        }
    }
}
impl Drop for PendingPoller {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_deferrals_coalesce_keep_backoff_and_exhaust_the_original_deadline() {
        let owners = PendingPollers::default();
        let start = Instant::now();
        let mut poller = owners
            .claim_with_clock(
                "agent",
                start,
                Duration::from_millis(75),
                Duration::from_millis(25),
            )
            .unwrap();
        assert!(poller.admit_at(start));
        let deadline = poller.deadline;
        for _ in 0..100 {
            assert!(owners.claim("agent").is_none());
        }
        assert_eq!(poller.deadline, deadline);
        assert!(!poller.admit_at(start + Duration::from_millis(24)));
        assert!(poller.admit_at(start + Duration::from_millis(25)));
        assert!(!poller.admit_at(start + Duration::from_millis(49)));
        assert!(poller.admit_at(start + Duration::from_millis(50)));
        assert!(!poller.admit_at(deadline));
        assert!(!poller.admit_at(deadline + Duration::from_secs(1)));
        assert!(owners.claim("agent").is_none());
        poller.release();
        let replacement = owners.claim("agent").unwrap();
        drop(poller); // old Drop must not release the replacement owner
        assert!(owners.claim("agent").is_none());
        drop(replacement);
        assert!(owners.claim("agent").is_some());
    }

    #[tokio::test]
    async fn repeated_idle_attempts_have_an_actual_delay() {
        let owners = PendingPollers::default();
        let mut poller = owners
            .claim_with_clock(
                "agent",
                Instant::now(),
                Duration::from_secs(120),
                Duration::from_millis(25),
            )
            .unwrap();
        let before = Instant::now();
        assert!(poller.admit_at(before));
        assert!(poller.next().await);
        assert!(Instant::now() - before >= Duration::from_millis(15));
    }
}
