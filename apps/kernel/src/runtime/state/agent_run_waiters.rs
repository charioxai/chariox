//! A blocked `chariox.agents.run` call is the consumer of its workers' completions.
//!
//! One consumer per completion, decided by an atomic take: the call that still waits on a
//! worker gets its end, and anyone else is told through `child_finished`. The map is in memory
//! on purpose: a restart ends the call with its turn, and the restart already tells the parent
//! through the event path.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use tokio::sync::oneshot;

use crate::durable_state::agent_lifecycle::AgentTaskExecution;
use crate::provider::TurnOutcome;

pub(crate) struct WorkerEnd {
    pub task: AgentTaskExecution,
    pub outcome: TurnOutcome,
    pub deadline_reached: bool,
}

#[derive(Default, Clone)]
pub(crate) struct AgentRunWaiters(Arc<Mutex<BTreeMap<String, oneshot::Sender<WorkerEnd>>>>);

impl AgentRunWaiters {
    fn map(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, oneshot::Sender<WorkerEnd>>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The call waits for the end of the worker's task.
    pub(crate) fn register(&self, worker: &str) -> oneshot::Receiver<WorkerEnd> {
        let (sender, receiver) = oneshot::channel();
        self.map().insert(worker.to_string(), sender);
        receiver
    }

    /// The call stops waiting: a later end of the worker goes to the event path.
    pub(crate) fn release(&self, worker: &str) {
        self.map().remove(worker);
    }

    /// Hands the end to the call waiting on the worker; false when nobody waits any more.
    pub(crate) fn claim(&self, worker: &str, end: WorkerEnd) -> bool {
        let sender = self.map().remove(worker);
        sender.is_some_and(|sender| sender.send(end).is_ok())
    }
}
