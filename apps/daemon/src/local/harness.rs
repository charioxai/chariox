use std::env;
use std::thread;
use std::time::{Duration, Instant};

use crate::app::DaemonApp;
use crate::attachment::{AttachmentMode, ClientCapabilityLevel};
use crate::error::DaemonError;
use crate::local::{
    AttachToSessionRequest, EndSessionRequest, LaunchProviderRunRequest, LocalDaemonRequest,
    LocalDaemonResponse, PumpTerminalOutputRequest, ResizeTerminalRequest,
    SendTerminalInputRequest,
};
use crate::session::CreateSessionRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalHarnessReport {
    pub session_id: String,
    pub controller_attachment_id: String,
    pub observer_attachment_id: String,
    pub provider_run_id: String,
    pub output_preview: String,
}

pub fn run_local_harness(app: &mut DaemonApp) -> Result<LocalHarnessReport, DaemonError> {
    let session = match app.handle_local_request(LocalDaemonRequest::CreateSession(
        CreateSessionRequest::new("workspace-harness", "worktree-harness"),
    ))? {
        LocalDaemonResponse::SessionCreated { session } => session,
        _ => unreachable!("create-session must return SessionCreated"),
    };

    let controller = match app.handle_local_request(LocalDaemonRequest::AttachToSession(
        AttachToSessionRequest {
            session_id: session.id().to_string(),
            client_id: "client-controller".to_string(),
            capability_level: ClientCapabilityLevel::FullTerminal,
            mode: AttachmentMode::Controller,
        },
    ))? {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => unreachable!("attach must return SessionAttached"),
    };

    let observer = match app.handle_local_request(LocalDaemonRequest::AttachToSession(
        AttachToSessionRequest {
            session_id: session.id().to_string(),
            client_id: "client-observer".to_string(),
            capability_level: ClientCapabilityLevel::InteractiveStructured,
            mode: AttachmentMode::Observer,
        },
    ))? {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => unreachable!("attach must return SessionAttached"),
    };

    let provider_run = match app.handle_local_request(LocalDaemonRequest::LaunchProviderRun(
        LaunchProviderRunRequest {
            session_id: session.id().to_string(),
            adapter_key: "dev-stub".to_string(),
            provider: "claude-code".to_string(),
            account_profile: "default".to_string(),
            model: "sonnet".to_string(),
        },
    ))? {
        LocalDaemonResponse::ProviderRunLaunched { provider_run } => provider_run,
        _ => unreachable!("launch must return ProviderRunLaunched"),
    };

    let _ =
        app.handle_local_request(LocalDaemonRequest::ResizeTerminal(ResizeTerminalRequest {
            session_id: session.id().to_string(),
            cols: 100,
            rows: 30,
        }))?;

    let _ = app.handle_local_request(LocalDaemonRequest::SendTerminalInput(
        SendTerminalInputRequest {
            session_id: session.id().to_string(),
            attachment_id: controller.id().to_string(),
            bytes: b"harness smoke\n".to_vec(),
        },
    ))?;

    let output_preview = wait_for_output(app, session.id())?;

    let _ = app.handle_local_request(LocalDaemonRequest::EndSession(EndSessionRequest {
        session_id: session.id().to_string(),
    }))?;

    Ok(LocalHarnessReport {
        session_id: session.id().to_string(),
        controller_attachment_id: controller.id().to_string(),
        observer_attachment_id: observer.id().to_string(),
        provider_run_id: provider_run.id().to_string(),
        output_preview,
    })
}

fn wait_for_output(app: &mut DaemonApp, session_id: &str) -> Result<String, DaemonError> {
    let timeout_ms = env::var("ARROBA_HARNESS_OUTPUT_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(2_000);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        let response = app.handle_local_request(LocalDaemonRequest::PumpTerminalOutput(
            PumpTerminalOutputRequest {
                session_id: session_id.to_string(),
            },
        ))?;

        if let LocalDaemonResponse::TerminalOutput { records } = response {
            if !records.is_empty() {
                let combined = records
                    .into_iter()
                    .flat_map(|record| record.bytes)
                    .collect::<Vec<u8>>();
                return Ok(String::from_utf8_lossy(&combined).into_owned());
            }
        }

        if Instant::now() >= deadline {
            return Err(DaemonError::LocalHarnessTimeout {
                session_id: session_id.to_string(),
                timeout_ms,
            });
        }

        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use crate::{DaemonApp, DaemonConfig};

    use super::run_local_harness;

    #[test]
    fn local_harness_exercises_managed_session_flow() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");

        let report = run_local_harness(&mut app).expect("local harness should succeed");

        assert!(!report.output_preview.is_empty());
        assert!(report.output_preview.contains("harness smoke"));
        assert_eq!(app.terminal().input_records().len(), 1);
        assert!(!app.terminal().output_records().is_empty());
    }
}
