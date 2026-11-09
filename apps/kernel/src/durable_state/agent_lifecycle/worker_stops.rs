//! MP-08 / MP-10 / MP-11 A10-S04: a worker run the home ended the turn for stays
//! a stored obligation until the worker stops it, across home restarts.
use super::*;
use serde::Deserialize;

/// The exact worker run the home gave up on without knowing it stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct UnstoppedWorkerRun {
    pub binding: crate::agent::RemoteAgentBinding,
    pub room_id: String,
    pub agent_id: String,
    pub home_prompt_id: String,
    pub worker_provider_run_id: String,
    /// Failed attempts to tell the worker to stop it.
    pub attempts: u32,
    pub next_attempt_ms: u64,
}

impl UnstoppedWorkerRun {
    fn key(&self) -> (&str, &str) {
        (
            &self.binding.execution_lease_id,
            &self.worker_provider_run_id,
        )
    }
}

#[derive(Clone)]
pub(crate) enum WorkerStopMutation {
    /// Keeps the attempts of an obligation that is already stored.
    Open(UnstoppedWorkerRun),
    Retry {
        run: UnstoppedWorkerRun,
        next_attempt_ms: u64,
    },
    Discharge(UnstoppedWorkerRun),
}

pub(super) fn initialize(db: &Connection) -> Result<(), DaemonError> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS agent_unstopped_worker_runs(lease_id TEXT NOT NULL,run_id TEXT NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(lease_id,run_id));",
    )
    .map_err(sql)
}

pub(super) fn apply(
    tx: &Transaction<'_>,
    mutation: WorkerStopMutation,
) -> Result<Outcome, DaemonError> {
    match mutation {
        WorkerStopMutation::Open(run) => {
            tx.execute(
                "INSERT INTO agent_unstopped_worker_runs VALUES(?1,?2,?3) ON CONFLICT DO NOTHING",
                params![run.key().0, run.key().1, encode(&run)?],
            )
            .map_err(sql)?;
        }
        WorkerStopMutation::Retry {
            mut run,
            next_attempt_ms,
        } => {
            run.attempts += 1;
            run.next_attempt_ms = next_attempt_ms;
            tx.execute(
                "UPDATE agent_unstopped_worker_runs SET payload=?3 WHERE lease_id=?1 AND run_id=?2",
                params![run.key().0, run.key().1, encode(&run)?],
            )
            .map_err(sql)?;
        }
        WorkerStopMutation::Discharge(run) => {
            tx.execute(
                "DELETE FROM agent_unstopped_worker_runs WHERE lease_id=?1 AND run_id=?2",
                params![run.key().0, run.key().1],
            )
            .map_err(sql)?;
        }
    }
    Ok(Outcome::Saved)
}

impl DurableKernelStateStore {
    pub(crate) fn unstopped_worker_runs(&self) -> Result<Vec<UnstoppedWorkerRun>, DaemonError> {
        let db = self.lock_connection("agent.unstopped_worker_runs")?;
        let mut q = db
            .prepare("SELECT payload FROM agent_unstopped_worker_runs")
            .map_err(sql)?;
        let rows = q.query_map([], |r| r.get::<_, String>(0)).map_err(sql)?;
        rows.map(|r| decode(&r.map_err(sql)?)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::Fixture;
    use super::*;

    fn run() -> UnstoppedWorkerRun {
        UnstoppedWorkerRun {
            binding: crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker".into(),
                worker_machine_id: "machine".into(),
                execution_lease_id: "lease".into(),
                leased_agent_id: "leased".into(),
                active_worker_provider_run_id: Some("run".into()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: None,
                worker_worktree_id: None,
            },
            room_id: "room".into(),
            agent_id: "agent".into(),
            home_prompt_id: "prompt".into(),
            worker_provider_run_id: "run".into(),
            attempts: 0,
            next_attempt_ms: 0,
        }
    }

    #[test]
    fn an_open_obligation_keeps_its_attempts_until_it_is_discharged() {
        let f = Fixture::new();
        let apply = |mutation| {
            f.store
                .agent_lifecycle(Operation::WorkerStop(mutation))
                .unwrap()
        };
        apply(WorkerStopMutation::Open(run()));
        apply(WorkerStopMutation::Retry {
            run: run(),
            next_attempt_ms: 90_000,
        });
        apply(WorkerStopMutation::Open(run()));
        let stored = f.store.unstopped_worker_runs().unwrap();
        assert_eq!(
            stored
                .iter()
                .map(|r| (r.attempts, r.next_attempt_ms))
                .collect::<Vec<_>>(),
            [(1, 90_000)],
            "{stored:?}"
        );
        apply(WorkerStopMutation::Discharge(run()));
        assert!(f.store.unstopped_worker_runs().unwrap().is_empty());
    }
}
