//! MP-08/MP-09/MP-10/MP-11 A10: one supervisor, two receipt destinations.
use super::*;
use crate::durable_state::agent_lifecycle::{
    self as ledger, LeasedProcessMutation, Operation, Outcome,
};

impl KernelRuntimeState {
    pub(super) fn apply_agent_process_receipt(
        &self,
        op: Operation,
    ) -> Result<Outcome, DaemonError> {
        let id = match &op {
            Operation::ProcessStarted { id, .. }
            | Operation::ProcessMatched { id, .. }
            | Operation::ProcessExited { id, .. } => id,
            _ => return self.owned.durable_state_store.agent_lifecycle(op),
        };
        let store = &self.owned.durable_state_store;
        let Some(mut receipt) = store.leased_process_receipt(id)? else {
            return store.agent_lifecycle(op);
        };
        let revision = receipt.revision;
        match op {
            Operation::ProcessStarted { pid, now, .. } => {
                if receipt.wake.state != "starting" || pid <= 1 {
                    return Err(ledger::error("leased process start is not current"));
                }
                receipt.process_start_ticks = Some(
                    super::agent_process_group::birth(pid)
                        .map_err(|_| ledger::error("leased process start identity unavailable"))?,
                );
                receipt.wake.pid = Some(pid);
                receipt.wake.verified_at_ms = Some(now);
                receipt.wake.state = "running".into();
            }
            Operation::ProcessMatched { line, now, .. } => {
                if receipt.wake.state != "running" || receipt.wake.matched_at_ms.is_some() {
                    return Ok(Outcome::Wakes(vec![]));
                }
                receipt.matched_line = Some(line);
                receipt.wake.matched_at_ms = Some(now);
            }
            Operation::ProcessExited {
                exit_code, tail, ..
            } => {
                receipt.wake.state = if exit_code.is_none() {
                    "lost"
                } else if receipt.wake.state == "cancelling" {
                    "cancelled"
                } else {
                    "exited"
                }
                .into();
                receipt.wake.exit_code = exit_code;
                receipt.output_tail = tail;
            }
            _ => unreachable!(),
        }
        store.agent_lifecycle(Operation::LeasedProcess(LeasedProcessMutation::Update {
            id: receipt.wake.id.clone(),
            expected_revision: revision,
            receipt,
        }))?;
        // Only the home applies occurrences and commits task settlement.
        Ok(Outcome::Wakes(vec![]))
    }
}
