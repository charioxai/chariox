use std::collections::BTreeMap;

use crate::error::DaemonError;
use crate::session::SessionService;

use super::{LaunchProviderRequest, ProviderRegistry, ProviderRunState, RuntimeProviderRun};

#[derive(Debug, Clone)]
pub struct ProviderProcessService {
    registry: ProviderRegistry,
    runs: BTreeMap<String, RuntimeProviderRun>,
    next_run_number: u64,
}

impl ProviderProcessService {
    pub fn new() -> Self {
        Self {
            registry: ProviderRegistry::new(),
            runs: BTreeMap::new(),
            next_run_number: 0,
        }
    }

    pub fn registry(&self) -> &ProviderRegistry {
        &self.registry
    }

    pub fn launch_run(
        &mut self,
        sessions: &mut SessionService,
        request: LaunchProviderRequest,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let active_run_id = sessions
            .get_session(&request.session_id)?
            .active_provider_run_id()
            .map(str::to_owned);

        if let Some(active_run_id) = active_run_id.as_deref() {
            self.park_run(sessions, &request.session_id, active_run_id)?;
        }

        let adapter = self.registry.resolve(&request.adapter_key).ok_or_else(|| {
            DaemonError::ProviderAdapterNotFound {
                adapter_key: request.adapter_key.clone(),
            }
        })?;

        let run_id = self.next_run_id();
        let launch_result = adapter.launch(&request)?;
        let mut run = RuntimeProviderRun::new(run_id.clone(), &request, launch_result);
        run.mark_running();

        self.runs.insert(run_id.clone(), run.clone());
        sessions.set_active_provider_run(&request.session_id, Some(run_id))?;

        Ok(run)
    }

    pub fn park_run(
        &mut self,
        sessions: &mut SessionService,
        session_id: &str,
        run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let session = sessions.get_session(session_id)?;

        if session.active_provider_run_id() != Some(run_id) {
            return Err(DaemonError::InconsistentActiveProviderRun {
                session_id: session_id.to_string(),
                active_provider_run_id: session.active_provider_run_id().map(str::to_owned),
                requested_provider_run_id: run_id.to_string(),
            });
        }

        let run_snapshot = self.get_run(run_id)?;

        if run_snapshot.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: run_id.to_string(),
            });
        }

        if run_snapshot.state() != ProviderRunState::Running {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: run_id.to_string(),
                state: run_snapshot.state(),
                operation: "park",
            });
        }

        let adapter = self.adapter_for(run_snapshot.adapter_key())?;
        adapter.park(&run_snapshot);

        let run = self.get_run_mut(run_id)?;
        run.mark_parked();
        sessions.set_active_provider_run(session_id, None)?;

        Ok(run.clone())
    }

    pub fn resume_run(
        &mut self,
        sessions: &mut SessionService,
        session_id: &str,
        run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let active_run_id = sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_owned);

        if let Some(active_run_id) = active_run_id.as_deref() {
            if active_run_id != run_id {
                self.park_run(sessions, session_id, active_run_id)?;
            }
        }

        let run_snapshot = self.get_run(run_id)?;

        if run_snapshot.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: run_id.to_string(),
            });
        }

        if run_snapshot.state() != ProviderRunState::Parked {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: run_id.to_string(),
                state: run_snapshot.state(),
                operation: "resume",
            });
        }

        let adapter = self.adapter_for(run_snapshot.adapter_key())?;
        adapter.resume(&run_snapshot);

        let run = self.get_run_mut(run_id)?;
        run.mark_running();
        sessions.set_active_provider_run(session_id, Some(run_id.to_string()))?;

        Ok(run.clone())
    }

    pub fn terminate_run(
        &mut self,
        sessions: &mut SessionService,
        session_id: &str,
        run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let active_run_id = sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_owned);
        let run_snapshot = self.get_run(run_id)?;

        if run_snapshot.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: run_id.to_string(),
            });
        }

        if run_snapshot.state() == ProviderRunState::Ended {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: run_id.to_string(),
                state: run_snapshot.state(),
                operation: "terminate",
            });
        }

        let adapter = self.adapter_for(run_snapshot.adapter_key())?;
        adapter.terminate(&run_snapshot);

        let run = self.get_run_mut(run_id)?;
        run.mark_ended();

        if active_run_id.as_deref() == Some(run_id) {
            sessions.set_active_provider_run(session_id, None)?;
        }

        Ok(run.clone())
    }

    pub fn get_run(&self, run_id: &str) -> Result<RuntimeProviderRun, DaemonError> {
        self.runs
            .get(run_id)
            .cloned()
            .ok_or_else(|| DaemonError::ProviderRunNotFound {
                provider_run_id: run_id.to_string(),
            })
    }

    pub fn terminate_session_runs(
        &mut self,
        sessions: &mut SessionService,
        session_id: &str,
    ) -> Result<Vec<RuntimeProviderRun>, DaemonError> {
        let run_ids: Vec<String> = self
            .runs
            .values()
            .filter(|run| run.session_id() == session_id && run.state() != ProviderRunState::Ended)
            .map(|run| run.id().to_string())
            .collect();

        let mut terminated_runs = Vec::with_capacity(run_ids.len());

        for run_id in run_ids {
            terminated_runs.push(self.terminate_run(sessions, session_id, &run_id)?);
        }

        Ok(terminated_runs)
    }

    fn get_run_mut(&mut self, run_id: &str) -> Result<&mut RuntimeProviderRun, DaemonError> {
        self.runs
            .get_mut(run_id)
            .ok_or_else(|| DaemonError::ProviderRunNotFound {
                provider_run_id: run_id.to_string(),
            })
    }

    fn adapter_for(
        &self,
        adapter_key: &str,
    ) -> Result<&'static dyn super::ProviderAdapter, DaemonError> {
        self.registry
            .resolve(adapter_key)
            .ok_or_else(|| DaemonError::ProviderAdapterNotFound {
                adapter_key: adapter_key.to_string(),
            })
    }

    fn next_run_id(&mut self) -> String {
        self.next_run_number += 1;
        format!("provider-run-{}", self.next_run_number)
    }
}

impl Default for ProviderProcessService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::config::DaemonConfig;
    use crate::session::{CreateSessionRequest, SessionService, SessionStatus};

    use super::{LaunchProviderRequest, ProviderProcessService, ProviderRunState};

    fn sessions() -> SessionService {
        SessionService::new(&DaemonConfig::for_tests())
    }

    fn launch_request(session_id: &str, model: &str) -> LaunchProviderRequest {
        LaunchProviderRequest::new(session_id, "dev-stub", "claude-code", "default", model)
    }

    #[test]
    fn launches_the_first_provider_run() {
        let mut sessions = sessions();
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let mut providers = ProviderProcessService::new();

        let run = providers
            .launch_run(&mut sessions, launch_request(session.id(), "sonnet"))
            .expect("provider run should launch");
        let session = sessions
            .get_session(session.id())
            .expect("session should exist");

        assert_eq!(run.id(), "provider-run-1");
        assert_eq!(run.state(), ProviderRunState::Running);
        assert_eq!(run.adapter_key(), "dev-stub");
        assert_eq!(session.active_provider_run_id(), Some(run.id()));
        assert_eq!(session.status(), SessionStatus::Active);
    }

    #[test]
    fn parks_existing_run_when_new_run_becomes_active() {
        let mut sessions = sessions();
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let mut providers = ProviderProcessService::new();

        let first = providers
            .launch_run(&mut sessions, launch_request(session.id(), "sonnet"))
            .expect("first run should launch");
        let second = providers
            .launch_run(&mut sessions, launch_request(session.id(), "opus"))
            .expect("second run should launch");

        let first = providers
            .get_run(first.id())
            .expect("first run should still exist");
        let session = sessions
            .get_session(session.id())
            .expect("session should exist");

        assert_eq!(first.state(), ProviderRunState::Parked);
        assert_eq!(second.state(), ProviderRunState::Running);
        assert_eq!(session.active_provider_run_id(), Some(second.id()));
    }

    #[test]
    fn rejects_inconsistent_active_run_state() {
        let mut sessions = sessions();
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let mut providers = ProviderProcessService::new();

        sessions
            .set_active_provider_run(session.id(), Some("missing-run".to_string()))
            .expect("session active run can be set for this invariant test");

        let error = providers
            .launch_run(&mut sessions, launch_request(session.id(), "sonnet"))
            .expect_err("launch should reject inconsistent active run state");

        match error {
            crate::DaemonError::ProviderRunNotFound { provider_run_id } => {
                assert_eq!(provider_run_id, "missing-run");
            }
            other => panic!("unexpected error: {other}"),
        }
    }
}
