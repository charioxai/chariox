//! MP-08 / MP-10 / MP-11: shared push/poll delivery ownership and receipts.
use super::*;
use crate::transport::relay_peer::RelayProjectionDelivery;

pub(crate) fn receipt_key(
    session: &str,
    agent: &str,
    worker: &str,
    run: &str,
    delivery: &RelayProjectionDelivery,
) -> String {
    format!(
        "{session}:{agent}:remote:{worker}:{}:{}:{run}",
        delivery.execution_lease_id, delivery.leased_agent_id
    )
}

impl RemoteLeaseRuntime<'_> {
    pub(crate) fn drain_retained_leased_runtime_projection(
        &mut self,
        leased_agent_id: &str,
        provider_run_id: &str,
        pump_output: bool,
        replay_settled_completion: bool,
    ) -> Result<Option<(String, RelayPeerEvent)>, DaemonError> {
        // Recheck the normal run/lease fence even when returning retained bytes.
        self.leased_provider_output_session(leased_agent_id, provider_run_id)?;
        let agent = self.app.leased_agents.get(leased_agent_id).ok_or_else(|| {
            DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.into(),
            }
        })?;
        let key = format!(
            "{}:{}:{provider_run_id}",
            agent.backing_session_id, agent.backing_agent_id
        );
        if let Some(event) = self
            .app
            .operational_history_store()
            .pending_leased_projection_delivery(&key)?
        {
            let lease = self
                .app
                .execution_leases
                .get(&agent.lease_id)
                .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                    lease_id: agent.lease_id.clone(),
                })?;
            return Ok(Some((lease.home_kernel_id.clone(), event)));
        }
        self.build_leased_runtime_projection(
            leased_agent_id,
            provider_run_id,
            pump_output,
            replay_settled_completion,
            true,
        )
    }

    pub(crate) fn acknowledge_leased_runtime_projection(
        &mut self,
        leased_agent_id: &str,
        provider_run_id: &str,
        delivery: &RelayProjectionDelivery,
    ) -> Result<bool, DaemonError> {
        self.leased_provider_output_session(leased_agent_id, provider_run_id)?;
        let agent = self.app.leased_agents.get(leased_agent_id).ok_or_else(|| {
            DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.into(),
            }
        })?;
        if agent.lease_id != delivery.execution_lease_id || agent.id != delivery.leased_agent_id {
            return Ok(false);
        }
        let key = format!(
            "{}:{}:{provider_run_id}",
            agent.backing_session_id, agent.backing_agent_id
        );
        self.app
            .operational_history_store()
            .acknowledge_leased_projection_delivery(&key, delivery.sequence)
    }
}

// The terminal producer already bounds individual records. Page the projection
// as well: a long transcript must not become a multi-megabyte consuming reply.
// Prompts precede output; settlement is carried only by the last page.
pub(super) fn pages(
    event: RelayPeerEvent,
    receipt: RelayProjectionDelivery,
) -> Vec<RelayPeerEvent> {
    let RelayPeerEvent::LeasedRuntimeProjection {
        home_session_id,
        home_agent_id,
        provider_run_id,
        provider_run,
        prompts,
        output_chunks,
        notices,
        completions,
        ..
    } = event;
    let mut result = Vec::new();
    let mut sequence = receipt.sequence;
    let mut page = || {
        let delivery = RelayProjectionDelivery {
            sequence,
            ..receipt.clone()
        };
        sequence += 1;
        RelayPeerEvent::LeasedRuntimeProjection {
            home_session_id: home_session_id.clone(),
            home_agent_id: home_agent_id.clone(),
            provider_run_id: provider_run_id.clone(),
            provider_run: provider_run.clone(),
            delivery: Some(delivery),
            prompts: vec![],
            output_chunks: vec![],
            notices: vec![],
            completions: vec![],
        }
    };
    let mut current = page();
    // Keep indivisible protocol records intact; output producers enforce their
    // existing record bounds. The budget counts encoded JSON, not raw Vec bytes.
    let mut bytes = serde_json::to_vec(&current)
        .expect("projection serializes")
        .len();
    for item in prompts
        .into_iter()
        .map(Item::Prompt)
        .chain(output_chunks.into_iter().map(Item::Output))
        .chain(notices.into_iter().map(Item::Notice))
        .chain(completions.into_iter().map(Item::Completion))
    {
        let size = item.json_len();
        if bytes + size + 64 > 128 * 1024 && !empty(&current) {
            result.push(current);
            current = page();
            bytes = serde_json::to_vec(&current)
                .expect("projection serializes")
                .len();
        }
        bytes += size + 1;
        let RelayPeerEvent::LeasedRuntimeProjection {
            prompts,
            output_chunks,
            notices,
            completions,
            ..
        } = &mut current;
        match item {
            Item::Prompt(value) => prompts.push(value),
            Item::Output(value) => output_chunks.push(value),
            Item::Notice(value) => notices.push(value),
            Item::Completion(value) => completions.push(value),
        }
    }
    result.push(current);
    result
}

fn empty(event: &RelayPeerEvent) -> bool {
    let RelayPeerEvent::LeasedRuntimeProjection {
        prompts,
        output_chunks,
        notices,
        completions,
        ..
    } = event;
    prompts.is_empty() && output_chunks.is_empty() && notices.is_empty() && completions.is_empty()
}

enum Item {
    Prompt(RelayProjectedPrompt),
    Output(RelayProjectedOutputChunk),
    Notice(String),
    Completion(RelayProjectedCompletion),
}
impl Item {
    fn json_len(&self) -> usize {
        match self {
            Self::Prompt(value) => serde_json::to_vec(value),
            Self::Output(value) => serde_json::to_vec(value),
            Self::Notice(value) => serde_json::to_vec(value),
            Self::Completion(value) => serde_json::to_vec(value),
        }
        .expect("projection item serializes")
        .len()
    }
}
