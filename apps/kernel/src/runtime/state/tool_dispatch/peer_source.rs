//! Where a peer address points: the agent a follower names with `{room?, agent}` when it
//! subscribes to an agent outside its own room. Only agents of rooms the same owner has qualify.
use super::*;

/// A resolved peer: the room it lives in and its agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::runtime::state) struct SourceRef {
    pub(in crate::runtime::state) room: String,
    pub(in crate::runtime::state) agent_id: String,
    pub(in crate::runtime::state) alias: Option<String>,
}

/// Why an address names no peer. The reasons say what to do and never confirm that a room of
/// another owner, or an agent hidden from the caller, exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::runtime::state) enum PeerRefusal {
    NoSuchRoom,
    NoSuchAgent { reference: String },
    Ambiguous { reference: String },
}

impl std::fmt::Display for PeerRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchRoom => f.write_str("that room is not one of yours: name a room you own"),
            Self::NoSuchAgent { reference } => write!(
                f,
                "no agent `{reference}` in that room: name an agent by its alias, ref or id"
            ),
            Self::Ambiguous { reference } => write!(
                f,
                "`{reference}` names more than one agent in that room: use its id"
            ),
        }
    }
}

impl KernelRuntimeState {
    /// The peer `agent` of `room`, for the caller `owner`. A sub-agent is never addressed from
    /// outside its team, so it resolves exactly like an agent that does not exist.
    // The cross-room subscribe path calls this once the coordinator folds it with the message resolver.
    #[allow(dead_code)]
    pub(in crate::runtime::state) fn resolve_peer_source(
        &self,
        owner: &str,
        room: &str,
        agent: &str,
    ) -> Result<SourceRef, PeerRefusal> {
        if self
            .owned
            .session_store
            .session_owner_user_id(room)
            .as_deref()
            != Some(owner)
        {
            return Err(PeerRefusal::NoSuchRoom);
        }
        let reference = agent.trim().trim_start_matches('@');
        let agents = self.session_agents(room);
        let mut matches = agents.iter().filter(|candidate| {
            !candidate.archived()
                && candidate.parent_agent_id().is_none()
                && (candidate.id() == reference
                    || candidate.agent_ref() == reference
                    || candidate
                        .alias()
                        .is_some_and(|alias| alias.trim().eq_ignore_ascii_case(reference)))
        });
        let found = matches.next().filter(|_| !reference.is_empty());
        let Some(found) = found else {
            return Err(PeerRefusal::NoSuchAgent {
                reference: reference.to_owned(),
            });
        };
        if matches.next().is_some() {
            return Err(PeerRefusal::Ambiguous {
                reference: reference.to_owned(),
            });
        }
        Ok(SourceRef {
            room: room.to_owned(),
            agent_id: found.id().to_owned(),
            alias: found.alias().map(str::to_owned),
        })
    }
}
