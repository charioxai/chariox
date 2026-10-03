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
            .filter_map(|process| {
                process
                    .identity
                    .clone()
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
        let turns = self
            .owned
            .sudo_turns
            .lock()
            .expect("access state poisoned")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        turns
            .into_iter()
            .find(|turn| {
                turn.provider_run_id.as_deref() == Some(run_id.as_str())
                    && self.sudo_live(turn)
                    && self
                        .owned
                        .sudo_process_cutoffs
                        .lock()
                        .expect("sudo process cutoffs poisoned")
                        .get(&turn.entry_id)
                        .is_some_and(|cutoff| {
                            chain[..root_index].iter().all(|identity| {
                                crate::runtime::kernel_access::process::born_after(
                                    identity, *cutoff,
                                )
                            })
                        })
            })
            .ok_or_else(|| error("this provider turn has no sudo authority"))
    }

    pub(crate) fn sudo_peer_live(&self, id: &str, peer: &ProcessIdentity) -> bool {
        self.sudo_for_peer(peer)
            .is_ok_and(|turn| turn.entry_id == id)
    }
}
