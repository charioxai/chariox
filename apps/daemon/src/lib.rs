pub mod app;
pub mod attachment;
pub mod config;
pub mod error;
pub mod provider;
pub mod session;

pub use app::DaemonApp;
pub use config::DaemonConfig;
pub use error::DaemonError;

#[cfg(test)]
mod tests {
    use super::attachment::{AttachRequest, AttachmentMode, ClientCapabilityLevel};
    use super::provider::LaunchProviderRequest;
    use super::session::CreateSessionRequest;
    use super::{DaemonApp, DaemonConfig, DaemonError};

    #[test]
    fn daemon_app_bootstrap_wires_runtime_services() {
        let config = DaemonConfig::for_tests();
        let app = DaemonApp::bootstrap(config.clone()).expect("bootstrap should succeed");

        assert_eq!(app.config(), &config);
        assert_eq!(app.sessions().active_session_count(), 0);
        assert!(app.attachments().list_events().is_empty());
        assert_eq!(app.providers().registry().registered_adapter_count(), 1);
        assert_eq!(
            app.startup_message(),
            "arroba daemon daemon-test ready on machine machine-test"
        );
    }

    #[test]
    fn daemon_config_rejects_empty_identifiers() {
        let error = DaemonApp::bootstrap(DaemonConfig::new("", "machine-local", "miguel"))
            .expect_err("empty daemon id should be rejected");

        match error {
            DaemonError::InvalidConfig { field, .. } => assert_eq!(field, "daemon_id"),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn ending_session_via_app_removes_runtime_attachments() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = app
            .sessions_mut()
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");

        let controller = app
            .attach(AttachRequest::new(
                session.id(),
                "client-a",
                ClientCapabilityLevel::FullTerminal,
                AttachmentMode::Controller,
            ))
            .expect("controller should attach");

        let ended = app
            .end_session(session.id())
            .expect("session should end cleanly through the app");

        assert_eq!(ended.id(), session.id());
        assert!(app.attachments().get_attachment(controller.id()).is_err());
    }

    #[test]
    fn launching_provider_via_app_marks_session_active() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = app
            .sessions_mut()
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");

        let run = app
            .launch_provider(LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            ))
            .expect("provider run should launch");

        let session = app
            .sessions()
            .get_session(session.id())
            .expect("session should still exist");

        assert_eq!(session.active_provider_run_id(), Some(run.id()));
    }
}
