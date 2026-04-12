use serde::{Deserialize, Serialize};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::RuntimeProviderRun;
use crate::session::{unix_epoch_ms, RuntimeSession};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionMetadata {
    pub projection_version: u64,
    pub last_event_id: u64,
    pub generated_at_ms: u64,
}

impl ProjectionMetadata {
    pub fn new(projection_version: u64, last_event_id: u64) -> Self {
        Self {
            projection_version,
            last_event_id,
            generated_at_ms: unix_epoch_ms(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSnapshotProjection {
    pub metadata: ProjectionMetadata,
    pub session: RuntimeSession,
    pub provider_run: Option<RuntimeProviderRun>,
}

impl SessionSnapshotProjection {
    pub fn from_daemon_app(
        app: &mut DaemonApp,
        session_id: &str,
        last_event_id: u64,
    ) -> Result<Self, DaemonError> {
        let mut session = app.sessions().get_session(session_id)?;
        let agents = app.agents().get_session_agents(session_id);
        session.set_agents(agents);
        app.project_session_runtime_view(&mut session);
        let provider_run = session
            .active_provider_run_id()
            .and_then(|provider_run_id| app.providers().get_run(provider_run_id).ok());
        Ok(Self {
            metadata: ProjectionMetadata::new(1, last_event_id),
            session,
            provider_run,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::kernel::projection::SessionSnapshotProjection;
    use crate::session::CreateSessionRequest;
    use crate::{DaemonApp, DaemonConfig};

    #[test]
    fn session_snapshot_projection_includes_metadata_and_agents() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");

        let projection = SessionSnapshotProjection::from_daemon_app(&mut app, session.id(), 42)
            .expect("projection should build");

        assert_eq!(projection.metadata.projection_version, 1);
        assert_eq!(projection.metadata.last_event_id, 42);
        assert_eq!(projection.session.id(), session.id());
        assert_eq!(projection.session.agents().len(), 1);
    }
}
