use std::collections::BTreeMap;
use std::process::Command;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use super::agent::{CreateAgentRequest, GitWorktreePlacement};
use super::app::RemoteLeaseRuntime;
use super::attachment::{AttachRequest, ClientCapabilityLevel};
use super::provider::{LaunchProviderRequest, ProviderResumeState};
use super::session::{CreateSessionRequest, PromptStatus, PromptSubmissionOutcome, SessionStatus};
use super::terminal::TerminalOutputKind;
use super::transport::relay_peer::{
    RelayPeerEvent, RelayProjectedCompletion, RelayProjectedOutputChunk,
};
use super::{DaemonApp, DaemonConfig, DaemonError};

static CURRENT_DIR_LOCK: Mutex<()> = Mutex::new(());

fn run_test_git(cwd: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git should run");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn wait_for_terminal_output(
    app: &mut DaemonApp,
    session_id: &str,
    attachment_id: &str,
) -> Vec<super::terminal::TerminalOutputRecord> {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);

    loop {
        let records = crate::app::provider_output::pump_terminal_output_for_attachment(
            app,
            session_id,
            attachment_id,
        )
        .expect("terminal output should fan out");
        if !records.is_empty() {
            return records;
        }

        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for terminal output"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

mod app_lifecycle;
mod capability_boundaries;
mod provider_sessions;
mod remote_leases;
