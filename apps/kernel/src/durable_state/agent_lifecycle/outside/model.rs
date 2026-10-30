//! MP-08 / MP-09 / MP-10 / MP-11: recipient and subscription state.
use super::*;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Recipient {
    SessionAgent {
        session_id: String,
        agent_id: String,
    },
    Outside {
        subscription_id: String,
    },
}
impl Recipient {
    pub(in crate::durable_state::agent_lifecycle) fn address(&self) -> (&str, &str) {
        match self {
            Self::SessionAgent {
                session_id,
                agent_id,
            } => (session_id, agent_id),
            // Room IDs are nonempty. Outside rows cannot be selected by a Room agent.
            Self::Outside { subscription_id } => ("", subscription_id),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Part {
    pub id: String,
    pub home: bool,
    pub targets: Vec<EventTarget>,
    pub ack: u64,
    pub floor: u64,
    pub latest: u64,
    pub ended: Option<String>,
    #[serde(default)]
    pub ended_at_ms: Option<u64>,
    pub remote_cursors: BTreeMap<String, u64>,
    #[serde(default)]
    pub cleanup_targets: Vec<EventTarget>,
    #[serde(default)]
    pub run_states: BTreeMap<String, crate::local::EventRunState>,
    /// The last settled turn each followed agent's holder has been told of.
    #[serde(default)]
    pub settled_prompts: BTreeMap<String, String>,
    #[serde(default)]
    pub kernel_health: BTreeMap<String, bool>,
    /// The inbox rows at the last count, so retention counts again only when appends since
    /// then could have reached the limit.
    #[serde(default)]
    pub counted: Option<CountedInbox>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct CountedInbox {
    /// `latest` when the rows were counted; a later row has a higher sequence.
    pub at: u64,
    pub rows: u64,
}
impl Part {
    pub(super) fn new(id: String, home: bool) -> Self {
        Self {
            id,
            home,
            targets: Vec::new(),
            ack: 0,
            floor: 0,
            latest: 0,
            ended: None,
            ended_at_ms: None,
            remote_cursors: BTreeMap::new(),
            cleanup_targets: Vec::new(),
            run_states: BTreeMap::new(),
            settled_prompts: BTreeMap::new(),
            kernel_health: BTreeMap::new(),
            counted: None,
        }
    }
    pub(crate) fn subscription(&self) -> EventSubscription {
        EventSubscription {
            subscription_id: self.id.clone(),
            targets: self.targets.clone(),
            kernels: Vec::new(),
            acknowledged_cursor: self.ack,
            ended: self.ended.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Mutation {
    KernelHealth {
        id: String,
        kernel: String,
        online: bool,
        reason: String,
    },
    RunStates {
        states: Vec<RunState>,
    },
    #[cfg(test)]
    Bound {
        id: String,
        limit: usize,
    },
    Subscribe {
        baseline: Vec<RunState>,
        /// Decisions already open when the subscription is made. Each reaches
        /// the holder as the `pending` event it missed, once, for the targets
        /// this call adds.
        open_decisions: Vec<OpenDecision>,
        id: String,
        home: bool,
        targets: Vec<EventTarget>,
    },
    Unsubscribe {
        id: String,
        targets: Vec<EventTarget>,
        all: bool,
        reason: String,
    },
    Publish {
        occurrence: String,
        event: Box<OutsideEvent>,
    },
    Import {
        id: String,
        kernel: String,
        events: Vec<OutsideEvent>,
    },
    Ack {
        id: String,
        cursor: u64,
    },
    Restart {
        keep: Vec<String>,
    },
    CleanupAck {
        id: String,
        targets: Vec<EventTarget>,
    },
    Identity {
        kernel: String,
        machine: String,
    },
}

impl Mutation {
    /// Whether committing this changes which sessions and agents the subscriptions follow.
    pub(in crate::durable_state::agent_lifecycle) fn changes_targets(&self) -> bool {
        match self {
            Self::Subscribe { .. } | Self::Unsubscribe { .. } | Self::Restart { .. } => true,
            Self::Publish { event, .. } => super::followed::removes_target(event),
            Self::Import { events, .. } => events.iter().any(super::followed::removes_target),
            Self::KernelHealth { .. }
            | Self::RunStates { .. }
            | Self::Ack { .. }
            | Self::CleanupAck { .. }
            | Self::Identity { .. } => false,
            #[cfg(test)]
            Self::Bound { .. } => false,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RunState {
    pub session_id: String,
    pub agent_id: String,
    pub state: crate::local::EventRunState,
    /// The agent's last settled turn and how it ended, while it is not working.
    pub turn: Option<SettledTurn>,
}

#[derive(Debug, Clone)]
pub(crate) struct SettledTurn {
    pub prompt_id: String,
    pub turn_id: String,
    pub outcome: crate::provider::TurnOutcome,
}

/// A decision that was open before a holder followed its target.
#[derive(Debug, Clone)]
pub(crate) struct OpenDecision {
    pub session_id: String,
    pub agent_id: Option<String>,
    pub interaction_id: String,
    pub summary: String,
    pub value: crate::local::OutsideEventValue,
}
