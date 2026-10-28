//! Session-scoped read model: only launch-owned observations reach clients.
use super::{samples_for_key, ProcessSample};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum SessionProcessOwner {
    Agent(String),
    Terminal(String),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SessionProcess {
    pub owner: SessionProcessOwner,
    #[serde(flatten)]
    pub process: ProcessSample,
}

pub(crate) fn for_session(
    session_id: &str,
    providers: &crate::provider::ProviderProcessServiceStore,
    terminals: &crate::room_terminal::RoomTerminalService,
) -> Vec<SessionProcess> {
    let mut rows = Vec::new();
    for (owner, key) in keys_for_session(session_id, providers, terminals) {
        rows.extend(
            samples_for_key(&key)
                .into_iter()
                .map(|process| SessionProcess {
                    owner: owner.clone(),
                    process,
                }),
        );
    }
    rows.sort_by(|a, b| a.process.pid.cmp(&b.process.pid));
    rows.dedup_by_key(|row| row.process.pid);
    rows
}

pub(crate) fn keys_for_session(
    session_id: &str,
    providers: &crate::provider::ProviderProcessServiceStore,
    terminals: &crate::room_terminal::RoomTerminalService,
) -> Vec<(SessionProcessOwner, String)> {
    let mut keys = providers
        .list_runs()
        .into_iter()
        .filter(|run| run.session_id() == session_id)
        .filter_map(|run| {
            run.agent_instance_id().map(|agent| {
                (
                    SessionProcessOwner::Agent(agent.to_string()),
                    run.id().to_string(),
                )
            })
        })
        .collect::<Vec<_>>();
    keys.extend(terminals.process_keys(session_id));
    keys
}
