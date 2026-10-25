//! MP-08 / MP-10 / MP-11: the latest turn of each agent one session subscription has been sent.
//! A client draws the turn from this event instead of reading the history outline again.
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use crate::local::SessionHistoryOutlineTurn;
use crate::runtime::projection::{AgentRuntimeActivity, SessionSnapshotProjection};
use crate::runtime::router::CommandRouter;
use crate::transport::kernel_protocol::KernelEvent;

/// At most one `agent_turn_changed` per agent in this time, except the one that ends a turn.
pub(crate) const AGENT_TURN_COALESCE: Duration = Duration::from_millis(50);
/// A turn's end reaches history a little after the signal that announces it; look again this often...
const SETTLE_RETRY: Duration = Duration::from_millis(50);
/// ...and this many times before the turn is taken as still open.
const SETTLE_RETRIES: u8 = 40;

struct Dirty {
    ends_turn: bool,
    retries_left: u8,
    not_before: Option<Instant>,
}

#[derive(Default)]
pub(crate) struct AgentTurnFeed {
    sent: BTreeMap<String, SessionHistoryOutlineTurn>,
    sent_at: BTreeMap<String, Instant>,
    dirty: BTreeMap<String, Dirty>,
    /// `None` until the first snapshot; the view's own read covers everything before it.
    visible: Option<BTreeSet<String>>,
    activity: Option<BTreeMap<String, AgentRuntimeActivity>>,
}

impl AgentTurnFeed {
    /// An agent produced output or completed a message: its turn may have changed.
    pub(crate) fn mark_changed(&mut self, agent_id: &str) {
        self.dirty.entry(agent_id.to_string()).or_insert(Dirty {
            ends_turn: false,
            retries_left: 0,
            not_before: None,
        });
    }

    /// An agent's turn ended or is about to: send it now, with its final status.
    pub(crate) fn mark_ended(&mut self, agent_id: &str) {
        self.dirty.insert(
            agent_id.to_string(),
            Dirty {
                ends_turn: true,
                retries_left: SETTLE_RETRIES,
                not_before: None,
            },
        );
    }

    pub(crate) fn observe_snapshot(&mut self, snapshot: &SessionSnapshotProjection) {
        self.visible = Some(
            snapshot
                .session
                .agents()
                .iter()
                .map(|agent| agent.id().to_string())
                .collect(),
        );
        let changed = self.activity.as_ref().map(|previous| {
            snapshot
                .agent_activity
                .iter()
                .filter(|(agent_id, activity)| previous.get(*agent_id) != Some(*activity))
                .map(|(agent_id, activity)| {
                    let ended = previous
                        .get(agent_id)
                        .is_some_and(|before| before.active_turn.is_some())
                        && activity.active_turn.is_none();
                    (agent_id.clone(), ended)
                })
                .collect::<Vec<_>>()
        });
        for (agent_id, ended) in changed.unwrap_or_default() {
            if ended {
                self.mark_ended(&agent_id);
            } else {
                self.mark_changed(&agent_id);
            }
        }
        self.activity = Some(snapshot.agent_activity.clone());
    }

    /// When a held turn is due to be sent.
    pub(crate) fn next_due(&self) -> Option<Instant> {
        self.dirty
            .iter()
            .map(|(agent_id, dirty)| self.due_at(agent_id, dirty).unwrap_or_else(Instant::now))
            .min()
    }

    /// `None`: due now.
    fn due_at(&self, agent_id: &str, dirty: &Dirty) -> Option<Instant> {
        let coalesced = (!dirty.ends_turn)
            .then(|| {
                self.sent_at
                    .get(agent_id)
                    .map(|at| *at + AGENT_TURN_COALESCE)
            })
            .flatten();
        [coalesced, dirty.not_before].into_iter().flatten().max()
    }

    /// The turns that changed since they were last sent, each as the history outline reads it now.
    pub(crate) async fn flush(
        &mut self,
        router: &CommandRouter,
        session_id: &str,
    ) -> Vec<KernelEvent> {
        let now = Instant::now();
        let due = self
            .dirty
            .iter()
            .filter(|(agent_id, dirty)| self.due_at(agent_id, dirty).is_none_or(|at| at <= now))
            .map(|(agent_id, _)| agent_id.clone())
            .collect::<Vec<_>>();
        let mut events = Vec::new();
        for agent_id in due {
            let Some(mut dirty) = self.dirty.remove(&agent_id) else {
                continue;
            };
            if !self
                .visible
                .as_ref()
                .is_some_and(|visible| visible.contains(&agent_id))
            {
                continue;
            }
            let read = match router.read_agent_latest_turn(session_id, &agent_id).await {
                Ok(Some(read)) => read,
                Ok(None) => continue,
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.runtime_transport",
                        "failed to read an agent's latest turn for its subscription",
                        serde_json::json!({
                            "session_id": session_id,
                            "agent_id": agent_id,
                            "error": error.to_string(),
                        }),
                    );
                    continue;
                }
            };
            let still_open = dirty.ends_turn
                && read.turn.lifecycle == crate::local::SessionHistoryOutlineTurnLifecycle::Open
                && dirty.retries_left > 0;
            if self.sent.get(&agent_id) != Some(&read.turn) {
                self.sent.insert(agent_id.clone(), read.turn.clone());
                self.sent_at.insert(agent_id.clone(), Instant::now());
                events.push(KernelEvent::AgentTurnChanged {
                    session_id: session_id.to_string(),
                    agent_id: agent_id.clone(),
                    history_revision: read.history_revision,
                    turn: Box::new(read.turn),
                });
            }
            if still_open {
                dirty.retries_left -= 1;
                dirty.not_before = Some(Instant::now() + SETTLE_RETRY);
                self.dirty.insert(agent_id, dirty);
            }
        }
        events
    }
}
