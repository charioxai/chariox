use std::env;
use std::thread;
use std::time::{Duration, Instant};

use crate::app::DaemonApp;
use crate::attachment::ClientCapabilityLevel;
use crate::error::DaemonError;
use crate::local::LocalDaemonClient;
use crate::local::{
    AttachToSessionRequest, EndSessionRequest, GetSessionStateRequest,
    GetWaitingRoomPublicSnapshotRequest, LaunchProviderRunRequest, LocalDaemonRequest,
    LocalDaemonResponse, PumpTerminalOutputRequest, ResizeTerminalRequest, SubmitPromptRequest,
};
use crate::session::CreateSessionRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalHarnessReport {
    pub session_id: String,
    pub prompt_attachment_id: String,
    pub second_attachment_id: String,
    pub provider_run_id: String,
    pub waiting_room_snapshot_schema_version: u32,
    pub waiting_room_public_agent_count: usize,
    pub output_preview: String,
}

pub fn run_local_harness(app: DaemonApp) -> Result<LocalHarnessReport, DaemonError> {
    let client = LocalDaemonClient::new(app)?;

    let session = match client.send(LocalDaemonRequest::CreateSession(
        CreateSessionRequest::new("workspace-harness", "worktree-harness"),
    ))? {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => unreachable!("create-session must return SessionCreated"),
    };

    let prompt_source = match client.send(LocalDaemonRequest::AttachToSession(
        AttachToSessionRequest {
            session_id: session.id().to_string(),
            client_id: "client-primary".to_string(),
            capability_level: ClientCapabilityLevel::FullTerminal,
        },
    ))? {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => unreachable!("attach must return SessionAttached"),
    };

    let second = match client.send(LocalDaemonRequest::AttachToSession(
        AttachToSessionRequest {
            session_id: session.id().to_string(),
            client_id: "client-secondary".to_string(),
            capability_level: ClientCapabilityLevel::InteractiveStructured,
        },
    ))? {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => unreachable!("attach must return SessionAttached"),
    };

    let provider_run = match client.send(LocalDaemonRequest::LaunchProviderRun(
        LaunchProviderRunRequest {
            session_id: session.id().to_string(),
            agent_id: None,
            adapter_key: "dev-stub".to_string(),
            provider: "claude-code".to_string(),
            account_profile: "default".to_string(),
            model: "sonnet".to_string(),
            variant: None,
            structured_endpoint: None,
            provider_session_id: None,
            native_tui: false,
        },
    ))? {
        LocalDaemonResponse::ProviderRunLaunched { provider_run }
        | LocalDaemonResponse::ProviderRunLaunchAccepted { provider_run } => provider_run,
        _ => unreachable!("launch must return ProviderRunLaunched"),
    };

    wait_for_provider_run_ready(&client, session.id(), provider_run.id())?;

    let _ = client.send(LocalDaemonRequest::ResizeTerminal(ResizeTerminalRequest {
        session_id: session.id().to_string(),
        cols: 100,
        rows: 30,
    }))?;

    let _ = client.send(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session.id().to_string(),
        attachment_id: prompt_source.id().to_string(),
        target_agent_id: None,
        prompt: "harness smoke\n".to_string(),
        attachments: Vec::new(),
    }))?;

    let output_preview = wait_for_output(&client, session.id(), prompt_source.id())?;
    let (waiting_room_snapshot_schema_version, waiting_room_public_agent_count) = match client
        .send(LocalDaemonRequest::GetWaitingRoomPublicSnapshot(
            GetWaitingRoomPublicSnapshotRequest,
        ))? {
        LocalDaemonResponse::WaitingRoomPublicSnapshot { snapshot } => {
            let public_session = snapshot
                .sessions
                .iter()
                .find(|candidate| candidate.id == session.id())
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session.id().to_string(),
                })?;
            (snapshot.schema_version, public_session.agents.len())
        }
        _ => unreachable!("public snapshot must return WaitingRoomPublicSnapshot"),
    };

    let _ = client.send(LocalDaemonRequest::EndSession(EndSessionRequest {
        session_id: session.id().to_string(),
    }))?;

    Ok(LocalHarnessReport {
        session_id: session.id().to_string(),
        prompt_attachment_id: prompt_source.id().to_string(),
        second_attachment_id: second.id().to_string(),
        provider_run_id: provider_run.id().to_string(),
        waiting_room_snapshot_schema_version,
        waiting_room_public_agent_count,
        output_preview,
    })
}

fn wait_for_provider_run_ready(
    client: &LocalDaemonClient,
    session_id: &str,
    provider_run_id: &str,
) -> Result<(), DaemonError> {
    let timeout_ms = env::var("ARROBA_HARNESS_PROVIDER_LAUNCH_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(2_000);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        let response = client.send(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session_id.to_string(),
            },
        ))?;

        if let LocalDaemonResponse::SessionState { session, .. } = response {
            if session.active_provider_run_id() == Some(provider_run_id) {
                return Ok(());
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

fn wait_for_output(
    client: &LocalDaemonClient,
    session_id: &str,
    attachment_id: &str,
) -> Result<String, DaemonError> {
    let timeout_ms = env::var("ARROBA_HARNESS_OUTPUT_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(2_000);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        let response = client.send(LocalDaemonRequest::PumpTerminalOutput(
            PumpTerminalOutputRequest {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
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
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");

        let report = run_local_harness(app).expect("local harness should succeed");

        assert!(!report.output_preview.is_empty());
        assert!(report.output_preview.contains("harness smoke"));
    }
}
