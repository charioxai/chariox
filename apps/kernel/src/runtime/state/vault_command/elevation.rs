//! The one gate for Vault capture and use. Today a live sudo window elevates;
//! the project mandate (VAUDIT S1) joins it here as mandate-or-sudo.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum VaultElevationKind {
    Capture,
    Use,
}

pub(super) struct VaultElevation {
    pub(super) turn: KernelSudoTurn,
    pub(super) run: crate::provider::RuntimeProviderRun,
}

impl KernelRuntimeState {
    pub(super) fn vault_command_context(
        &self,
        token: &str,
        kind: VaultElevationKind,
    ) -> Result<VaultElevation, DaemonError> {
        let operation = match kind {
            VaultElevationKind::Capture => "Vault capture",
            VaultElevationKind::Use => "Vault use",
        };
        let turn =
            self.vault_elevation(super::super::sudo::VaultCaller::AuthToken(token), operation)?;
        let run = turn
            .provider_run_id
            .as_deref()
            .ok_or_else(|| error("the sudo window has no running provider turn"))
            .and_then(|run| self.watched_process_run(run))?;
        Ok(VaultElevation { turn, run })
    }
}
