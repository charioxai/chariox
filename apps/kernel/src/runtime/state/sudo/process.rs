//! Shell callers use OS ancestry, never process-group/session membership.
use super::*;
use crate::runtime::kernel_access::process::{ancestry, ProcessIdentity};

impl KernelRuntimeState {
    pub(crate) fn sudo_for_peer(
        &self,
        peer: &ProcessIdentity,
    ) -> Result<KernelSudoTurn, DaemonError> {
        let chain = ancestry(peer).map_err(|_| error("provider process identity changed"))?;
        let mut roots = self
            .owned
            .provider_process_tracking
            .read()
            .processes
            .values()
            .filter(|process| process.endpoint_mode == crate::provider::AgentEndpointMode::Managed)
            .flat_map(|process| {
                // The endpoint root is fixed at runtime initialization, never
                // inferred from a requesting descendant during an elevated turn.
                [
                    process.identity.clone(),
                    process
                        .endpoint_identity
                        .clone()
                        .filter(ProcessIdentity::alive),
                ]
                .into_iter()
                .flatten()
                .map(|identity| (identity, process.owner_provider_run_ids.clone()))
            })
            .collect::<Vec<_>>();
        // Claude's stream-JSON child is owned by the provider actor, including
        // when its runtime state is leased for prompt I/O or restarted.
        roots.extend(
            self.owned
                .provider_store
                .read()
                .launched_process_identities()
                .into_iter()
                .map(|(run_id, identity)| (identity, vec![run_id])),
        );
        // The nearest launched provider is the boundary. Shared servers fail closed.
        let (root_index, owners) = chain
            .iter()
            .enumerate()
            .find_map(|(index, identity)| {
                roots
                    .iter()
                    .find(|(launched, _)| {
                        // Wrapper exec preserves the OS start identity, unlike PID reuse.
                        launched.pid == identity.pid
                            && launched.uid == identity.uid
                            && launched.start == identity.start
                    })
                    .map(|(_, owners)| (index, owners))
            })
            .ok_or_else(|| error("peer is outside a tracked provider process tree"))?;
        let [run_id] = owners.as_slice() else {
            return Err(error("sudo requires one dedicated provider process"));
        };
        let turn = self.sudo_for_provider_run(run_id)?;
        let session = self.owned.session_store.get_session(&turn.session_id)?;
        let born_after_cutoff = self
            .owned
            .prompt_state_owner
            .sudo_bound_process_cutoff(
                &session,
                &turn.agent_id,
                &turn.entry_id,
                turn.prompt_id.as_deref().unwrap_or_default(),
            )
            .is_some_and(|cutoff| {
                chain[..root_index].iter().all(|identity| {
                    crate::runtime::kernel_access::process::born_after(identity, cutoff)
                })
            });
        born_after_cutoff
            .then_some(turn)
            .ok_or_else(|| error("this provider turn has no sudo authority"))
    }

    pub(crate) fn sudo_peer_live(&self, id: &str, peer: &ProcessIdentity) -> bool {
        self.sudo_for_peer(peer)
            .is_ok_and(|turn| turn.entry_id == id)
    }
}
