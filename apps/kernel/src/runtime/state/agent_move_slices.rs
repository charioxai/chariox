//! Slice membership follows the durable execution commit, including recovery and retries.
use super::*;
use crate::agent_move::HomeMoveOperation;

impl KernelRuntimeState {
    pub(super) async fn finish_agent_move_slice_membership(
        &self,
        operation: &HomeMoveOperation,
    ) -> Result<(), DaemonError> {
        let forward = operation.target_ref.is_some();
        let worker = if forward {
            Some(operation.identity.target_kernel_id.as_str())
        } else {
            operation
                .source_remote
                .as_ref()
                .map(|source| source.worker_kernel_id.as_str())
        };
        let slice = worker
            .and_then(|worker| self.owned.slice_store.resolve_by_worker_kernel_ref(worker))
            .or_else(|| {
                operation
                    .source_remote
                    .as_ref()
                    .filter(|_| !forward)
                    .and_then(|source| {
                        self.owned
                            .slice_store
                            .resolve_by_worker_kernel_ref(&source.worker_machine_id)
                    })
            });
        let Some(slice) = slice else {
            return Ok(());
        };
        let agent_id = &operation.identity.home_agent_id;
        if forward {
            if !slice.agent_ids.contains(agent_id) {
                self.with_external_command_authority(None)
                    .attach_slice_agent(&slice.id, &operation.identity.home_session_id, agent_id)
                    .await?;
            }
        } else if slice.agent_ids.contains(agent_id) {
            let slice = self.owned.slice_store.detach_agent(
                &slice.id,
                agent_id,
                crate::session::unix_epoch_ms(),
            )?;
            self.owned.durable_state_store.append_event(
                "slice.updated",
                Some(slice.id.clone()),
                serde_json::json!({"slice":slice}),
            )?;
        }
        Ok(())
    }
}
