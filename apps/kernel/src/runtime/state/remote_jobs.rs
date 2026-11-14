//! Remote placement settings belong to the kernel, independently of any client.
use super::*;
use crate::local::{OffloadPolicy, RemoteJobPolicy};
use std::sync::Mutex as StdMutex;

static POLICY_WRITE: StdMutex<()> = StdMutex::new(());
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Policies {
    sessions: BTreeMap<String, OffloadPolicy>,
    agents: BTreeMap<String, OffloadPolicy>,
}
impl Policies {
    fn get(&self, room: &str, agent: Option<&str>) -> RemoteJobPolicy {
        let selected = agent.and_then(|a| self.agents.get(a)).copied();
        let room_policy = self.sessions.get(room).copied();
        RemoteJobPolicy {
            inherited_policy: if agent.is_some() {
                room_policy.unwrap_or_default()
            } else {
                OffloadPolicy::Off
            },
            policy: if agent.is_some() {
                selected
            } else {
                room_policy
            },
            effective_policy: selected.or(room_policy).unwrap_or_default(),
            source: if selected.is_some() {
                "agent"
            } else if room_policy.is_some() {
                "session"
            } else {
                "project"
            }
            .into(),
        }
    }
    fn load(path: &Path) -> Result<Self, DaemonError> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| policy_error(e.to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(policy_error(e.to_string())),
        }
    }
}
fn policy_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "remote job policy",
        message: message.into(),
    }
}
impl KernelRuntimeState {
    fn remote_policy_path(&self) -> PathBuf {
        self.owned
            .config_projection
            .snapshot()
            .private_runtime_state_root()
            .join("remote-job-policies.json")
    }
    fn policy_subject(
        &self,
        room: &str,
        agent: Option<&str>,
        caller: &str,
    ) -> Result<(), DaemonError> {
        self.ensure_room_control(room, caller)?;
        if let Some(id) = agent {
            let subject = self.owned.agent_store.get_agent(id)?;
            if subject.session_id() != room || subject.archived() {
                return Err(policy_error("the agent is not active in this Room"));
            }
        }
        Ok(())
    }
    pub(crate) fn remote_job_policy(
        &self,
        room: &str,
        agent: Option<&str>,
        caller: &str,
    ) -> Result<RemoteJobPolicy, DaemonError> {
        self.policy_subject(room, agent, caller)?;
        Ok(Policies::load(&self.remote_policy_path())?.get(room, agent))
    }
    pub(crate) fn set_remote_job_policy(
        &self,
        room: &str,
        agent: Option<&str>,
        caller: &str,
        policy: Option<OffloadPolicy>,
    ) -> Result<RemoteJobPolicy, DaemonError> {
        self.authorize_current_external_command()?;
        self.policy_subject(room, agent, caller)?;
        let _write = POLICY_WRITE.lock().unwrap_or_else(|e| e.into_inner());
        let path = self.remote_policy_path();
        let mut policies = Policies::load(&path)?;
        let (map, key) = agent.map_or((&mut policies.sessions, room), |agent| {
            (&mut policies.agents, agent)
        });
        match policy {
            Some(policy) => {
                map.insert(key.into(), policy);
            }
            None => {
                map.remove(key);
            }
        }
        let bytes = serde_json::to_vec(&policies).map_err(|e| policy_error(e.to_string()))?;
        crate::config::write_private_file(&path, &bytes)
            .map_err(|e| policy_error(e.to_string()))?;
        Ok(policies.get(room, agent))
    }
    pub(super) fn effective_remote_jobs_policy(
        &self,
        room: &str,
        agent: &str,
    ) -> Result<OffloadPolicy, DaemonError> {
        let session = self.owned.session_store.get_session(room)?;
        self.policy_subject(room, Some(agent), session.owner_user_id())?;
        Ok(Policies::load(&self.remote_policy_path())?
            .get(room, Some(agent))
            .effective_policy)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_job_policy_inherits_and_persists() {
        let dir = crate::test_support::TestWorktree::new("remote-job-policy");
        let path = dir.path().join("policies.json");
        let mut policies = Policies::load(&path).unwrap();
        assert_eq!(
            policies.get("room", Some("agent")).effective_policy,
            OffloadPolicy::Off
        );
        policies
            .sessions
            .insert("room".into(), OffloadPolicy::WhenBusy);
        policies
            .agents
            .insert("agent".into(), OffloadPolicy::AlwaysHeavy);
        crate::config::write_private_file(&path, &serde_json::to_vec(&policies).unwrap()).unwrap();
        let mut reloaded = Policies::load(&path).unwrap();
        assert_eq!(
            reloaded.get("room", Some("agent")).effective_policy,
            OffloadPolicy::AlwaysHeavy
        );
        reloaded.agents.remove("agent");
        assert_eq!(
            reloaded.get("room", Some("agent")).effective_policy,
            OffloadPolicy::WhenBusy
        );
    }
}
