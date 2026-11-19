//! The machines agents acquired without asking the owner: each one's class, rate, start and end.
//! A row is held before the request reaches the service, so the owner's limit on machines at the
//! same time is one compare-and-set; the decision is in `runtime/state/machine_acquisition/without_asking.rs`.
use rusqlite::{params, Connection, Transaction};

use super::{DurableKernelStateStore, DurableWriteOperation};
use crate::error::DaemonError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AcquiredState {
    /// Admitted against the limit; the service has not accepted it yet.
    Held,
    Running {
        allocation_ref: String,
    },
    Ended {
        allocation_ref: String,
        ended_ms: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AcquiredMachine {
    /// The request's decision id, so a retry of the same request finds its row.
    pub(crate) id: String,
    pub(crate) room_id: String,
    pub(crate) agent_id: String,
    pub(crate) class_id: String,
    pub(crate) rate_micros: u64,
    pub(crate) currency: String,
    pub(crate) started_ms: u64,
    pub(crate) state: AcquiredState,
}

#[derive(Debug)]
pub(crate) enum AcquiredMachineWrite {
    /// Inserts `machine` as held while fewer than `max_live` rows are held or running.
    Hold {
        machine: AcquiredMachine,
        max_live: u32,
    },
    /// A held row the service accepted starts now, unless its allocation is already recorded (the
    /// service replayed an earlier request): one machine is one row.
    Start {
        id: String,
        allocation_ref: String,
        started_ms: u64,
    },
    /// A held row the service refused leaves the ledger.
    Drop { id: String },
    /// A running allocation ended.
    End {
        allocation_ref: String,
        ended_ms: u64,
    },
}

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS durable_acquired_machines (
            id TEXT PRIMARY KEY NOT NULL,
            room_id TEXT NOT NULL, agent_id TEXT NOT NULL, class_id TEXT NOT NULL,
            rate_micros INTEGER NOT NULL, currency TEXT NOT NULL,
            state TEXT NOT NULL CHECK (state IN ('held', 'running', 'ended')),
            allocation_ref TEXT CHECK ((state = 'held') = (allocation_ref IS NULL)),
            started_ms INTEGER NOT NULL,
            ended_ms INTEGER CHECK ((state = 'ended') = (ended_ms IS NOT NULL))
        );",
    )
}

impl DurableKernelStateStore {
    /// Every machine that is held or running, and every one that ended at or after `ended_since_ms`.
    pub(crate) fn acquired_machines(
        &self,
        ended_since_ms: u64,
    ) -> Result<Vec<AcquiredMachine>, DaemonError> {
        let connection = self.lock_connection("acquired_machines.read")?;
        let mut statement = connection
            .prepare(
                "SELECT id, room_id, agent_id, class_id, rate_micros, currency, state, allocation_ref,
                    started_ms, ended_ms
                FROM durable_acquired_machines WHERE state != 'ended' OR ended_ms >= ?1
                ORDER BY started_ms, id",
            )
            .map_err(|error| state_error(error.to_string()))?;
        let rows = statement
            .query_map(params![ended_since_ms as i64], |row| {
                let state = match (
                    row.get::<_, String>(6)?.as_str(),
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<i64>>(9)?,
                ) {
                    ("held", None, None) => AcquiredState::Held,
                    ("running", Some(allocation_ref), None) => {
                        AcquiredState::Running { allocation_ref }
                    }
                    ("ended", Some(allocation_ref), Some(ended_ms)) => AcquiredState::Ended {
                        allocation_ref,
                        ended_ms: ended_ms as u64,
                    },
                    _ => return Err(rusqlite::Error::InvalidQuery),
                };
                Ok(AcquiredMachine {
                    id: row.get(0)?,
                    room_id: row.get(1)?,
                    agent_id: row.get(2)?,
                    class_id: row.get(3)?,
                    rate_micros: row.get::<_, i64>(4)? as u64,
                    currency: row.get(5)?,
                    started_ms: row.get::<_, i64>(8)? as u64,
                    state,
                })
            })
            .map_err(|error| state_error(error.to_string()))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| state_error(error.to_string()))
    }

    /// Applies `write` when the ledger is in the state it expects; false when it is not.
    pub(crate) fn write_acquired_machine(
        &self,
        write: AcquiredMachineWrite,
    ) -> Result<bool, DaemonError> {
        self.writer
            .execute(DurableWriteOperation::AcquiredMachine(write))
            .map(|changed| changed > 0)
    }
}

pub(super) fn apply(
    transaction: &Transaction<'_>,
    write: &AcquiredMachineWrite,
) -> rusqlite::Result<u64> {
    let changed = match write {
        AcquiredMachineWrite::Hold { machine, max_live } => transaction.execute(
            "INSERT INTO durable_acquired_machines
                (id, room_id, agent_id, class_id, rate_micros, currency, state, started_ms)
            SELECT ?1, ?2, ?3, ?4, ?5, ?6, 'held', ?7
            WHERE (SELECT COUNT(*) FROM durable_acquired_machines WHERE state != 'ended') < ?8
            ON CONFLICT (id) DO NOTHING",
            params![
                machine.id,
                machine.room_id,
                machine.agent_id,
                machine.class_id,
                machine.rate_micros as i64,
                machine.currency,
                machine.started_ms as i64,
                max_live
            ],
        )?,
        AcquiredMachineWrite::Start {
            id,
            allocation_ref,
            started_ms,
        } => transaction.execute(
            "UPDATE durable_acquired_machines
            SET state = 'running', allocation_ref = ?2, started_ms = ?3
            WHERE id = ?1 AND state = 'held'
                AND NOT EXISTS (SELECT 1 FROM durable_acquired_machines WHERE allocation_ref = ?2)",
            params![id, allocation_ref, *started_ms as i64],
        )?,
        AcquiredMachineWrite::Drop { id } => transaction.execute(
            "DELETE FROM durable_acquired_machines WHERE id = ?1 AND state = 'held'",
            params![id],
        )?,
        AcquiredMachineWrite::End {
            allocation_ref,
            ended_ms,
        } => transaction.execute(
            "UPDATE durable_acquired_machines SET state = 'ended', ended_ms = MAX(started_ms, ?2)
            WHERE allocation_ref = ?1 AND state = 'running'",
            params![allocation_ref, *ended_ms as i64],
        )?,
    };
    Ok(changed as u64)
}

fn state_error(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "acquired_machines",
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acquired_machine(id: &str) -> AcquiredMachine {
        AcquiredMachine {
            id: id.into(),
            room_id: "room".into(),
            agent_id: "agent".into(),
            class_id: "small".into(),
            rate_micros: 1_000,
            currency: "EUR".into(),
            started_ms: 10,
            state: AcquiredState::Held,
        }
    }

    fn hold(store: &DurableKernelStateStore, id: &str) -> bool {
        store
            .write_acquired_machine(AcquiredMachineWrite::Hold {
                machine: acquired_machine(id),
                max_live: 2,
            })
            .unwrap()
    }

    #[test]
    fn the_limit_counts_held_and_running_rows_and_an_end_frees_a_place_after_a_restart() {
        let directory = crate::test_support::TestTempDir::new("acquired-machines");
        let path = directory.path().join("kernel.db");
        {
            let store = DurableKernelStateStore::open(path.clone()).unwrap();
            assert!(hold(&store, "a"));
            assert!(!hold(&store, "a"), "a retry holds no second place");
            assert!(hold(&store, "b"));
            assert!(!hold(&store, "c"), "two at the same time is the limit");
            assert!(store
                .write_acquired_machine(AcquiredMachineWrite::Start {
                    id: "a".into(),
                    allocation_ref: "alloc-a".into(),
                    started_ms: 20,
                })
                .unwrap());
            assert!(store
                .write_acquired_machine(AcquiredMachineWrite::Drop { id: "b".into() })
                .unwrap());
            assert!(
                !store
                    .write_acquired_machine(AcquiredMachineWrite::Drop { id: "a".into() })
                    .unwrap(),
                "a started machine is never dropped"
            );
        }
        let store = DurableKernelStateStore::open(path).unwrap();
        assert!(hold(&store, "c"));
        assert!(!hold(&store, "d"));
        assert!(store
            .write_acquired_machine(AcquiredMachineWrite::End {
                allocation_ref: "alloc-a".into(),
                ended_ms: 80,
            })
            .unwrap());
        assert!(hold(&store, "d"));
        assert!(
            !store
                .write_acquired_machine(AcquiredMachineWrite::Start {
                    id: "d".into(),
                    allocation_ref: "alloc-a".into(),
                    started_ms: 90,
                })
                .unwrap(),
            "a replayed allocation is not recorded twice"
        );
        let acquired = store.acquired_machines(0).unwrap();
        assert_eq!(acquired.len(), 3);
        let ended = acquired.iter().find(|machine| machine.id == "a").unwrap();
        assert_eq!(
            ended.state,
            AcquiredState::Ended {
                allocation_ref: "alloc-a".into(),
                ended_ms: 80
            }
        );
        assert_eq!(ended.started_ms, 20);
        assert!(store
            .acquired_machines(81)
            .unwrap()
            .iter()
            .all(|r| r.id != "a"));
    }
}
