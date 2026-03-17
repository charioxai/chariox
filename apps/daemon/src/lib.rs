pub mod app;
pub mod attachment;
pub mod config;
pub mod error;
pub mod provider;
pub mod pty;
pub mod session;
pub mod terminal;

pub use app::DaemonApp;
pub use config::DaemonConfig;
pub use error::DaemonError;

#[cfg(test)]
mod tests {
    use super::attachment::{AttachRequest, AttachmentMode, ClientCapabilityLevel};
    use super::provider::LaunchProviderRequest;
    use super::session::CreateSessionRequest;
    use super::{DaemonApp, DaemonConfig, DaemonError};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn daemon_app_bootstrap_wires_runtime_services() {
        let config = DaemonConfig::for_tests();
        let app = DaemonApp::bootstrap(config.clone()).expect("bootstrap should succeed");

        assert_eq!(app.config(), &config);
        assert_eq!(app.sessions().active_session_count(), 0);
        assert!(app.attachments().list_events().is_empty());
        assert_eq!(app.providers().registry().registered_adapter_count(), 1);
        assert!(app.terminal().input_records().is_empty());
        assert!(app.terminal().output_records().is_empty());
        assert_eq!(
            app.startup_message(),
            "arroba daemon daemon-test ready on machine machine-test"
        );
    }

    #[test]
    fn daemon_config_rejects_empty_identifiers() {
        let error = match DaemonApp::bootstrap(DaemonConfig::new("", "machine-local", "miguel")) {
            Ok(_) => panic!("empty daemon id should be rejected"),
            Err(error) => error,
        };

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

    #[test]
    fn terminal_flow_writes_input_resizes_and_fans_out_output() {
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
        let observer = app
            .attach(AttachRequest::new(
                session.id(),
                "client-b",
                ClientCapabilityLevel::InteractiveStructured,
                AttachmentMode::Observer,
            ))
            .expect("observer should attach");

        let run = app
            .launch_provider(LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            ))
            .expect("provider run should launch");

        app.resize_terminal(session.id(), 90, 24)
            .expect("terminal resize should succeed");
        app.send_terminal_input(session.id(), controller.id(), b"fanout test\n")
            .expect("controller input should reach provider PTY");

        thread::sleep(Duration::from_millis(100));

        let records = app
            .pump_terminal_output(session.id())
            .expect("terminal output should fan out");

        assert!(!records.is_empty());
        assert_eq!(app.terminal().input_records().len(), 1);
        assert!(records
            .iter()
            .all(|record| record.provider_run_id == run.id()));
        assert!(records.iter().all(|record| {
            record
                .recipient_attachment_ids
                .contains(&controller.id().to_string())
                && record
                    .recipient_attachment_ids
                    .contains(&observer.id().to_string())
        }));
        let combined = records
            .into_iter()
            .flat_map(|record| record.bytes)
            .collect::<Vec<u8>>();
        let combined = String::from_utf8_lossy(&combined);
        assert!(combined.contains("fanout test"));
    }
}
