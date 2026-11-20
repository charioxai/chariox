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
    pub public_answer: Option<serde_json::Value>,
}

#[derive(Default, Clone)]
pub(crate) struct AgentRunWaiters(
    Arc<Mutex<BTreeMap<String, oneshot::Sender<WorkerEnd>>>>,
    #[cfg(test)] pause_test::ClaimPause,
);

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

    /// The call stops waiting: a later end of the worker goes to the event path. Returns the end
    /// already sent to it. The channel is closed before it is drained, so a claim that took the
    /// sender earlier either sent before the close (returned here) or fails and takes the event
    /// path; the end is never sent into a receiver nobody reads.
    pub(crate) fn abandon(
        &self,
        worker: &str,
        end: &mut oneshot::Receiver<WorkerEnd>,
    ) -> Option<WorkerEnd> {
        self.map().remove(worker);
        end.close();
        let sent = end.try_recv().ok();
        #[cfg(test)]
        self.1.let_claim_send();
        sent
    }

    /// Hands the end to the call waiting on the worker; false when nobody waits any more.
    pub(crate) async fn claim(&self, worker: &str, end: WorkerEnd) -> bool {
        let sender = self.map().remove(worker);
        #[cfg(test)]
        let held = self.1.hold_claim().await;
        let sent = sender.is_some_and(|sender| sender.send(end).is_ok());
        #[cfg(test)]
        if let Some(sent_signal) = held {
            let _ = sent_signal.send(());
        }
        sent
    }
}

#[cfg(test)]
impl AgentRunWaiters {
    /// The next claim stops after it took its sender and resumes when the call abandons the
    /// worker, right after it drained its channel; `reached` fires at the stop.
    pub(crate) fn pause_next_claim(&self) -> oneshot::Receiver<()> {
        self.1.arm()
    }

    pub(crate) fn discard_paused_claim(&self) {
        self.1.discard();
    }
}

#[cfg(test)]
mod pause_test {
    use std::sync::{mpsc, Arc, Mutex};
    use tokio::sync::oneshot;

    struct Held {
        reached: oneshot::Sender<()>,
        resume: oneshot::Receiver<()>,
        sent: mpsc::Sender<()>,
    }

    struct Releasing {
        resume: oneshot::Sender<()>,
        sent: mpsc::Receiver<()>,
    }

    #[derive(Default, Clone)]
    pub(super) struct ClaimPause {
        held: Arc<Mutex<Option<Held>>>,
        releasing: Arc<Mutex<Option<Releasing>>>,
    }

    impl ClaimPause {
        pub(super) fn arm(&self) -> oneshot::Receiver<()> {
            let (reached, reached_rx) = oneshot::channel();
            let (resume_tx, resume) = oneshot::channel();
            let (sent_tx, sent_rx) = mpsc::channel();
            *self.held.lock().unwrap() = Some(Held {
                reached,
                resume,
                sent: sent_tx,
            });
            *self.releasing.lock().unwrap() = Some(Releasing {
                resume: resume_tx,
                sent: sent_rx,
            });
            reached_rx
        }

        /// Stops an armed claim until the abandoning call lets it go; returns what tells that
        /// call the claim has sent.
        pub(super) async fn hold_claim(&self) -> Option<mpsc::Sender<()>> {
            let held = self.held.lock().unwrap().take()?;
            let _ = held.reached.send(());
            let _ = held.resume.await;
            Some(held.sent)
        }

        pub(super) fn discard(&self) {
            self.releasing.lock().unwrap().take();
        }

        /// Called by the abandoning call: lets the held claim send and waits until it has.
        pub(super) fn let_claim_send(&self) {
            let releasing = self.releasing.lock().unwrap().take();
            if let Some(releasing) = releasing {
                let _ = releasing.resume.send(());
                releasing
                    .sent
                    .recv_timeout(std::time::Duration::from_secs(10))
                    .expect("the held claim sent");
            }
        }
    }
}
