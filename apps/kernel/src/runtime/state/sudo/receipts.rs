use super::*;

impl KernelRuntimeState {
    pub(super) fn audit_sudo(
        &self,
        turn: &KernelSudoTurn,
        outcome: &str,
    ) -> Result<(), DaemonError> {
        if let Some(run) = turn.provider_run_id.as_deref() {
            self.owned
                .provider_run_projection
                .catalog_changes()
                .invalidate(run);
        }
        self.owned.durable_state_store.append_event(
            "kernel_access.sudo",
            Some(turn.entry_id.clone()),
            serde_json::json!({"outcome": outcome, "turn": turn}),
        )?;
        Ok(())
    }
}
