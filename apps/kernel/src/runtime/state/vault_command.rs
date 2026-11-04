//! MP-08 / MP-09 / MP-10 / MP-11 VAUDIT S2: Vault capture from a command and
//! Vault use as a command's environment or stdin. The kernel moves the value;
//! the model only ever holds an opaque credential_id.
use super::vault_use::VaultAgentContext;
use super::*;
use crate::runtime::kernel_access::error;

mod capture;
mod files;
mod remove;
mod use_env;

pub(in crate::runtime::state) use capture::{
    capture_binding_schema, committed as committed_capture, service_host, Captured,
};
pub(crate) use capture::{capture_command_spec, VAULT_CAPTURE_COMMAND};
pub(crate) use files::{capture_file_spec, write_file_spec, VAULT_CAPTURE_FILE, VAULT_WRITE_FILE};
pub(crate) use remove::{remove_capture_spec, VAULT_REMOVE_CAPTURE};
pub(crate) use use_env::{use_env_spec, VAULT_USE_ENV};

/// Codex gives a chariox tool call 300 s; a command stops well before that.
const DEFAULT_TIMEOUT_SECONDS: u64 = 120;
const MAX_TIMEOUT_SECONDS: u64 = 240;

fn command_timeout(seconds: Option<u64>) -> Result<Duration, DaemonError> {
    match seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS) {
        seconds @ 1..=MAX_TIMEOUT_SECONDS => Ok(Duration::from_secs(seconds)),
        _ => Err(error(format!(
            "timeout_seconds must be 1-{MAX_TIMEOUT_SECONDS}"
        ))),
    }
}

pub(in crate::runtime::state) fn validate_capture_binding(
    run: &crate::provider::RuntimeProviderRun,
    binding: &crate::config::VaultUseBinding,
) -> Result<(), DaemonError> {
    use crate::config::VaultUseTarget;
    binding.target.validate()?;
    match &binding.target {
        VaultUseTarget::CommandEnvironment { .. } | VaultUseTarget::CommandStdin { .. } => Ok(()),
        VaultUseTarget::File { path } if files::new_workspace_file(run, path)?.to_str() == Some(path.as_str()) => Ok(()),
        _ => Err(error("capture use_binding must name one command environment, stdin or canonical workspace file destination")),
    }
}

impl KernelRuntimeState {
    fn recheck_vault_command_context(
        &self,
        context: &VaultAgentContext,
    ) -> Result<(), DaemonError> {
        self.recheck_vault_agent(context)?;
        self.watched_process_run(context.run.id())?;
        Ok(())
    }
}

/// Runs the command off the async runtime; it may take minutes.
async fn run_command(
    state: &KernelRuntimeState,
    context: VaultAgentContext,
    command: super::agent_process_watch::KernelCommand,
) -> Result<super::agent_process_watch::KernelCommandOutput, DaemonError> {
    let state = state.clone();
    tokio::task::spawn_blocking(move || {
        state.recheck_vault_command_context(&context)?;
        let run = state.watched_process_run(context.run.id())?;
        state.run_kernel_command(&run, command)
    })
    .await
    .map_err(|_| error("the command runner stopped unexpectedly"))?
}
