//! Remote placement settings belong to the kernel, independently of any client.
use super::*;
use crate::local::{OffloadPolicy, RemoteJobPolicy};
use std::sync::Mutex as StdMutex;

static POLICY_WRITE: StdMutex<()> = StdMutex::new(());
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Policies {
    #[serde(default)]
    projects: BTreeMap<String, OffloadPolicy>,
    sessions: BTreeMap<String, OffloadPolicy>,
    agents: BTreeMap<String, OffloadPolicy>,
}
/// What a policy is read or written for; the closest one set wins: agent, then session, then project.
#[derive(Clone, Copy)]
enum PolicyScope<'a> {
    Project,
    Session,
    Agent(&'a str),
}
impl<'a> PolicyScope<'a> {
    fn of(agent: Option<&'a str>, project: bool) -> Result<Self, DaemonError> {
        match (agent, project) {
            (Some(_), true) => Err(policy_error(
                "a policy is for the project or for an agent, not both",
            )),
            (Some(agent), false) => Ok(Self::Agent(agent)),
            (None, true) => Ok(Self::Project),
            (None, false) => Ok(Self::Session),
        }
    }
}
impl Policies {
    fn get(&self, room: &str, project: &str, scope: PolicyScope) -> RemoteJobPolicy {
        let project_policy = self.projects.get(project).copied();
        let room_policy = self.sessions.get(room).copied();
        let agent_policy = match scope {
            PolicyScope::Agent(agent) => self.agents.get(agent).copied(),
            _ => None,
        };
        let (policy, inherited) = match scope {
            PolicyScope::Project => (project_policy, None),
            PolicyScope::Session => (room_policy, project_policy),
            PolicyScope::Agent(_) => (agent_policy, room_policy.or(project_policy)),
        };
        let effective = match scope {
            PolicyScope::Project => project_policy,
            PolicyScope::Session => room_policy.or(project_policy),
            PolicyScope::Agent(_) => agent_policy.or(room_policy).or(project_policy),
        };
        let source = match scope {
            PolicyScope::Agent(_) if agent_policy.is_some() => "agent",
            PolicyScope::Agent(_) | PolicyScope::Session if room_policy.is_some() => "session",
            // Unset everywhere is the project's default: clients show it as "As project".
            _ => "project",
        };
        RemoteJobPolicy {
            inherited_policy: inherited.unwrap_or_default(),
            policy,
            effective_policy: effective.unwrap_or_default(),
            source: source.into(),
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
        scope: PolicyScope,
        caller: &str,
    ) -> Result<String, DaemonError> {
        self.ensure_room_control(room, caller)?;
        if let PolicyScope::Agent(id) = scope {
            let subject = self.owned.agent_store.get_agent(id)?;
            if subject.session_id() != room || subject.archived() {
                return Err(policy_error("the agent is not active in this Room"));
            }
        }
        Ok(self
            .owned
            .session_store
            .get_session(room)?
            .project_id()
            .to_owned())
    }
    pub(crate) fn remote_job_policy(
        &self,
        room: &str,
        agent: Option<&str>,
        project: bool,
        caller: &str,
    ) -> Result<RemoteJobPolicy, DaemonError> {
        let scope = PolicyScope::of(agent, project)?;
        let project = self.policy_subject(room, scope, caller)?;
        Ok(Policies::load(&self.remote_policy_path())?.get(room, &project, scope))
    }
    pub(crate) fn set_remote_job_policy(
        &self,
        room: &str,
        agent: Option<&str>,
        project: bool,
        caller: &str,
        policy: Option<OffloadPolicy>,
    ) -> Result<RemoteJobPolicy, DaemonError> {
        self.authorize_current_external_command()?;
        let scope = PolicyScope::of(agent, project)?;
        let project = self.policy_subject(room, scope, caller)?;
        let _write = POLICY_WRITE.lock().unwrap_or_else(|e| e.into_inner());
        let path = self.remote_policy_path();
        let mut policies = Policies::load(&path)?;
        let (map, key) = match scope {
            PolicyScope::Project => (&mut policies.projects, project.as_str()),
            PolicyScope::Session => (&mut policies.sessions, room),
            PolicyScope::Agent(agent) => (&mut policies.agents, agent),
        };
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
        Ok(policies.get(room, &project, scope))
    }
    pub(super) fn effective_remote_jobs_policy(
        &self,
        room: &str,
        agent: &str,
    ) -> Result<OffloadPolicy, DaemonError> {
        let session = self.owned.session_store.get_session(room)?;
        let scope = PolicyScope::Agent(agent);
        let project = self.policy_subject(room, scope, session.owner_user_id())?;
        Ok(Policies::load(&self.remote_policy_path())?
            .get(room, &project, scope)
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
        let get =
            |policies: &Policies| policies.get("room", "project", PolicyScope::Agent("agent"));
        let mut policies = Policies::load(&path).unwrap();
        assert_eq!(get(&policies).effective_policy, OffloadPolicy::Off);
        assert_eq!(get(&policies).source, "project");
        policies
            .projects
            .insert("project".into(), OffloadPolicy::AlwaysHeavy);
        assert_eq!(get(&policies).effective_policy, OffloadPolicy::AlwaysHeavy);
        assert_eq!(get(&policies).source, "project");
        policies
            .sessions
            .insert("room".into(), OffloadPolicy::WhenBusy);
        assert_eq!(get(&policies).effective_policy, OffloadPolicy::WhenBusy);
        assert_eq!(get(&policies).source, "session");
        policies.agents.insert("agent".into(), OffloadPolicy::Off);
        crate::config::write_private_file(&path, &serde_json::to_vec(&policies).unwrap()).unwrap();
        let mut reloaded = Policies::load(&path).unwrap();
        let closest = get(&reloaded);
        assert_eq!(closest.effective_policy, OffloadPolicy::Off);
        assert_eq!(closest.source, "agent");
        assert_eq!(closest.inherited_policy, OffloadPolicy::WhenBusy);
        reloaded.agents.remove("agent");
        assert_eq!(get(&reloaded).effective_policy, OffloadPolicy::WhenBusy);
        reloaded.sessions.remove("room");
        assert_eq!(get(&reloaded).effective_policy, OffloadPolicy::AlwaysHeavy);
        let project_view = reloaded.get("room", "project", PolicyScope::Project);
        assert_eq!(project_view.policy, Some(OffloadPolicy::AlwaysHeavy));
    }
}
