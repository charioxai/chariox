//! The machines agents acquired without asking the owner: one row per agent request, with its
//! class, rate, start and end. Every write is a compare-and-set on the row's state, so concurrent
//! requests, a lost answer, a cancel and a stale list can never move a row the wrong way; the
//! decisions are in `runtime/state/machine_acquisition/without_asking.rs`.
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use super::{DurableKernelStateStore, DurableWriteOperation};
use crate::error::DaemonError;

/// Where one acquisition is in its life. A row is created `Pending` and leaves the ledger only
/// when it is certain no machine exists: an answer to another attempt proves nothing about the
/// first, so once a request is `Uncertain` only the service's machine or a fenced job cleanup
/// settles it.
///
/// | state | counts toward the maximum | costs | moves to, on |
/// |---|---|---|---|
/// | (none) | | | `Pending`: admitted while fewer than the maximum rows are live. |
/// | `Pending` | yes | from its start | `Running`: the service accepted it. Removed: the service refused the attempt that admitted it, that attempt was never sent, or job cleanup confirmed no machine exists. `Uncertain`: its answer was lost, or a retry of the same request goes ahead. |
/// | `Uncertain` | yes | from its start | `Running`: a retry's answer or job cleanup returns its machine. Removed: job cleanup confirmed no machine exists. A retry that is refused or never sent leaves it uncertain; a retry takes no new place. |
/// | `Running` | yes | yes | `Deleting`: the service says it is being deleted. `Ended`: the service confirms it deleted, or a list sent while it was running no longer has it. |
/// | `Deleting` | yes | yes | `Ended`, as from `Running`. |
/// | `Ended` | no | until its end | nothing. |
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AcquiredState {
    /// Sent to the service by the attempt that admitted it, which applies its answer (unless the
    /// kernel restarted before it came).
    Pending,
    /// More than one attempt may have reached the service, or an answer was lost: a machine may
    /// exist that no refusal can rule out.
    Uncertain,
    Running {
        allocation_ref: String,
    },
    Deleting {
        allocation_ref: String,
    },
    Ended {
        allocation_ref: String,
        ended_ms: u64,
    },
}

impl AcquiredState {
    /// Whether the machine may exist and be charged, so it holds a place.
    pub(crate) fn is_live(&self) -> bool {
        !matches!(self, Self::Ended { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AcquiredMachine {
    /// The agent and its request_id: a retry of the same request finds this row.
    pub(crate) agent_id: String,
    pub(crate) request_id: String,
    /// The job whose machine this is, so its cleanup can settle the row.
    pub(crate) job_ref: Option<String>,
    pub(crate) room_id: String,
    pub(crate) class_id: String,
    pub(crate) rate_micros: u64,
    pub(crate) currency: String,
    /// When the request was sent: an uncertain answer may hide a machine charged from then.
    pub(crate) started_ms: u64,
    pub(crate) state: AcquiredState,
}

/// The row a write names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AcquisitionKey {
    Request {
        agent_id: String,
        request_id: String,
    },
    Job(String),
}

#[derive(Debug)]
pub(crate) enum AcquiredMachineWrite {
    /// Inserts `machine` as pending while fewer than `max_live` rows are live.
    Admit {
        machine: AcquiredMachine,
        max_live: u32,
    },
    /// A pending or uncertain row the service accepted runs. When the allocation is already another
    /// row's (the service replayed an earlier request), this row leaves instead: one machine is one
    /// row.
    Accept {
        key: AcquisitionKey,
        allocation_ref: String,
    },
    /// A pending or uncertain row whose answer may have been lost, or which a retry goes ahead
    /// with, becomes uncertain.
    Uncertain { key: AcquisitionKey },
    /// A pending row whose only attempt was refused or never sent leaves the ledger. An uncertain
    /// row stays.
    Withdraw { key: AcquisitionKey },
    /// A pending or uncertain row for which a fenced job cleanup confirmed no machine exists leaves
    /// the ledger.
    Remove { key: AcquisitionKey },
    /// A running allocation is being deleted.
    Deleting { allocation_ref: String },
    /// A running or deleting allocation's deletion is confirmed.
    End {
        allocation_ref: String,
        ended_ms: u64,
    },
}

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS durable_acquired_machines (
            agent_id TEXT NOT NULL, request_id TEXT NOT NULL, job_ref TEXT UNIQUE,
            room_id TEXT NOT NULL, class_id TEXT NOT NULL,
            rate_micros INTEGER NOT NULL, currency TEXT NOT NULL,
            state TEXT NOT NULL
                CHECK (state IN ('pending', 'uncertain', 'running', 'deleting', 'ended')),
            allocation_ref TEXT UNIQUE
                CHECK ((state IN ('pending', 'uncertain')) = (allocation_ref IS NULL)),
            started_ms INTEGER NOT NULL,
            ended_ms INTEGER CHECK ((state = 'ended') = (ended_ms IS NOT NULL)),
            PRIMARY KEY (agent_id, request_id)
        );",
    )
}

const COLUMNS: &str = "agent_id, request_id, job_ref, room_id, class_id, rate_micros, currency,
    state, allocation_ref, started_ms, ended_ms";

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AcquiredMachine> {
    let state = match (
        row.get::<_, String>(7)?.as_str(),
        row.get::<_, Option<String>>(8)?,
        row.get::<_, Option<i64>>(10)?,
    ) {
        ("pending", None, None) => AcquiredState::Pending,
        ("uncertain", None, None) => AcquiredState::Uncertain,
        ("running", Some(allocation_ref), None) => AcquiredState::Running { allocation_ref },
        ("deleting", Some(allocation_ref), None) => AcquiredState::Deleting { allocation_ref },
        ("ended", Some(allocation_ref), Some(ended_ms)) => AcquiredState::Ended {
            allocation_ref,
            ended_ms: ended_ms as u64,
        },
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(AcquiredMachine {
        agent_id: row.get(0)?,
        request_id: row.get(1)?,
        job_ref: row.get(2)?,
        room_id: row.get(3)?,
        class_id: row.get(4)?,
        rate_micros: row.get::<_, i64>(5)? as u64,
        currency: row.get(6)?,
        started_ms: row.get::<_, i64>(9)? as u64,
        state,
    })
}

impl DurableKernelStateStore {
    /// Every live machine, and every one that ended at or after `ended_since_ms`.
    pub(crate) fn acquired_machines(
        &self,
        ended_since_ms: u64,
    ) -> Result<Vec<AcquiredMachine>, DaemonError> {
        let connection = self.lock_connection("acquired_machines.read")?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {COLUMNS} FROM durable_acquired_machines
                WHERE state != 'ended' OR ended_ms >= ?1 ORDER BY started_ms, agent_id, request_id"
            ))
            .map_err(|error| state_error(error.to_string()))?;
        let rows = statement
            .query_map(params![ended_since_ms as i64], read_row)
            .map_err(|error| state_error(error.to_string()))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| state_error(error.to_string()))
    }

    /// The row of one agent request, or of the machine `allocation_ref`.
    pub(crate) fn acquired_machine(
        &self,
        find: AcquiredMachineLookup<'_>,
    ) -> Result<Option<AcquiredMachine>, DaemonError> {
        let connection = self.lock_connection("acquired_machines.read")?;
        let (filter, values): (&str, Vec<&str>) = match find {
            AcquiredMachineLookup::Request {
                agent_id,
                request_id,
            } => (
                "agent_id = ?1 AND request_id = ?2",
                vec![agent_id, request_id],
            ),
            AcquiredMachineLookup::Allocation(allocation_ref) => {
                ("allocation_ref = ?1", vec![allocation_ref])
            }
        };
        connection
            .query_row(
                &format!("SELECT {COLUMNS} FROM durable_acquired_machines WHERE {filter}"),
                rusqlite::params_from_iter(values),
                read_row,
            )
            .optional()
            .map_err(|error| state_error(error.to_string()))
    }

    /// Applies `write` when the row is in the state it expects; false when it is not.
    pub(crate) fn write_acquired_machine(
        &self,
        write: AcquiredMachineWrite,
    ) -> Result<bool, DaemonError> {
        self.writer
            .execute(DurableWriteOperation::AcquiredMachine(write))
            .map(|changed| changed > 0)
    }
}

/// How to find one row.
pub(crate) enum AcquiredMachineLookup<'a> {
    Request {
        agent_id: &'a str,
        request_id: &'a str,
    },
    Allocation(&'a str),
}

/// The (agent_id, request_id) of the row `key` names, if it exists.
fn resolve(
    transaction: &Transaction<'_>,
    key: &AcquisitionKey,
) -> rusqlite::Result<Option<(String, String)>> {
    match key {
        AcquisitionKey::Request {
            agent_id,
            request_id,
        } => Ok(Some((agent_id.clone(), request_id.clone()))),
        AcquisitionKey::Job(job_ref) => transaction
            .query_row(
                "SELECT agent_id, request_id FROM durable_acquired_machines WHERE job_ref = ?1",
                params![job_ref],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional(),
    }
}

pub(super) fn apply(
    transaction: &Transaction<'_>,
    write: &AcquiredMachineWrite,
) -> rusqlite::Result<u64> {
    let changed = match write {
        AcquiredMachineWrite::Admit { machine, max_live } => transaction.execute(
            "INSERT INTO durable_acquired_machines
                (agent_id, request_id, job_ref, room_id, class_id, rate_micros, currency, state,
                    started_ms)
            SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?8
            WHERE (SELECT COUNT(*) FROM durable_acquired_machines WHERE state != 'ended') < ?9
            ON CONFLICT DO NOTHING",
            params![
                machine.agent_id,
                machine.request_id,
                machine.job_ref,
                machine.room_id,
                machine.class_id,
                machine.rate_micros as i64,
                machine.currency,
                machine.started_ms as i64,
                max_live
            ],
        )?,
        AcquiredMachineWrite::Accept {
            key,
            allocation_ref,
        } => {
            let Some((agent_id, request_id)) = resolve(transaction, key)? else {
                return Ok(0);
            };
            let started = transaction.execute(
                "UPDATE durable_acquired_machines SET state = 'running', allocation_ref = ?3
                WHERE agent_id = ?1 AND request_id = ?2 AND state IN ('pending', 'uncertain')
                    AND NOT EXISTS (SELECT 1 FROM durable_acquired_machines WHERE allocation_ref = ?3)",
                params![agent_id, request_id, allocation_ref],
            )?;
            if started == 0 {
                transaction.execute(
                    "DELETE FROM durable_acquired_machines
                    WHERE agent_id = ?1 AND request_id = ?2 AND state IN ('pending', 'uncertain')",
                    params![agent_id, request_id],
                )?;
            }
            started
        }
        AcquiredMachineWrite::Uncertain { key } => {
            let Some((agent_id, request_id)) = resolve(transaction, key)? else {
                return Ok(0);
            };
            transaction.execute(
                "UPDATE durable_acquired_machines SET state = 'uncertain'
                WHERE agent_id = ?1 AND request_id = ?2 AND state IN ('pending', 'uncertain')",
                params![agent_id, request_id],
            )?
        }
        AcquiredMachineWrite::Withdraw { key } => {
            let Some((agent_id, request_id)) = resolve(transaction, key)? else {
                return Ok(0);
            };
            transaction.execute(
                "DELETE FROM durable_acquired_machines
                WHERE agent_id = ?1 AND request_id = ?2 AND state = 'pending'",
                params![agent_id, request_id],
            )?
        }
        AcquiredMachineWrite::Remove { key } => {
            let Some((agent_id, request_id)) = resolve(transaction, key)? else {
                return Ok(0);
            };
            transaction.execute(
                "DELETE FROM durable_acquired_machines
                WHERE agent_id = ?1 AND request_id = ?2 AND state IN ('pending', 'uncertain')",
                params![agent_id, request_id],
            )?
        }
        AcquiredMachineWrite::Deleting { allocation_ref } => transaction.execute(
            "UPDATE durable_acquired_machines SET state = 'deleting'
            WHERE allocation_ref = ?1 AND state = 'running'",
            params![allocation_ref],
        )?,
        AcquiredMachineWrite::End {
            allocation_ref,
            ended_ms,
        } => transaction.execute(
            "UPDATE durable_acquired_machines SET state = 'ended', ended_ms = MAX(started_ms, ?2)
            WHERE allocation_ref = ?1 AND state IN ('running', 'deleting')",
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

    fn request(request_id: &str) -> AcquisitionKey {
        AcquisitionKey::Request {
            agent_id: "agent".into(),
            request_id: request_id.into(),
        }
    }

    fn admit(store: &DurableKernelStateStore, request_id: &str, job_ref: Option<&str>) -> bool {
        store
            .write_acquired_machine(AcquiredMachineWrite::Admit {
                machine: AcquiredMachine {
                    agent_id: "agent".into(),
                    request_id: request_id.into(),
                    job_ref: job_ref.map(str::to_owned),
                    room_id: "room".into(),
                    class_id: "small".into(),
                    rate_micros: 1_000,
                    currency: "EUR".into(),
                    started_ms: 10,
                    state: AcquiredState::Pending,
                },
                max_live: 2,
            })
            .unwrap()
    }

    fn write(store: &DurableKernelStateStore, write: AcquiredMachineWrite) -> bool {
        store.write_acquired_machine(write).unwrap()
    }

    fn state_of(store: &DurableKernelStateStore, request_id: &str) -> Option<AcquiredState> {
        store
            .acquired_machine(AcquiredMachineLookup::Request {
                agent_id: "agent",
                request_id,
            })
            .unwrap()
            .map(|machine| machine.state)
    }

    #[test]
    fn every_transition_is_a_compare_and_set_and_only_an_end_frees_a_place_after_a_restart() {
        let directory = crate::test_support::TestTempDir::new("acquired-machines");
        let path = directory.path().join("kernel.db");
        {
            let store = DurableKernelStateStore::open(path.clone()).unwrap();
            assert!(admit(&store, "a", None));
            assert!(!admit(&store, "a", None), "a retry takes no second place");
            assert!(admit(&store, "b", Some("job-b")));
            assert!(
                !admit(&store, "c", None),
                "two at the same time is the limit"
            );
            let accept = |key, allocation_ref: &str| AcquiredMachineWrite::Accept {
                key,
                allocation_ref: allocation_ref.into(),
            };
            assert!(write(&store, accept(request("a"), "alloc-a")));
            assert!(
                !write(&store, accept(request("a"), "alloc-a")),
                "accepted once"
            );
            assert!(
                !write(&store, AcquiredMachineWrite::Remove { key: request("a") }),
                "an accepted machine never leaves the ledger"
            );
            assert!(write(
                &store,
                AcquiredMachineWrite::Remove {
                    key: AcquisitionKey::Job("job-b".into())
                }
            ));
            assert!(write(
                &store,
                AcquiredMachineWrite::Deleting {
                    allocation_ref: "alloc-a".into()
                }
            ));
        }
        let store = DurableKernelStateStore::open(path).unwrap();
        assert!(admit(&store, "c", None));
        assert!(
            !admit(&store, "d", None),
            "a deleting machine still holds its place"
        );
        assert!(write(
            &store,
            AcquiredMachineWrite::End {
                allocation_ref: "alloc-a".into(),
                ended_ms: 80,
            }
        ));
        assert!(!write(
            &store,
            AcquiredMachineWrite::Deleting {
                allocation_ref: "alloc-a".into()
            }
        ));
        assert!(admit(&store, "d", None));
        assert!(
            !write(
                &store,
                AcquiredMachineWrite::Accept {
                    key: request("d"),
                    allocation_ref: "alloc-a".into(),
                }
            ),
            "a replayed allocation is not recorded twice"
        );
        assert_eq!(state_of(&store, "d"), None, "its row left the ledger");
        assert_eq!(
            state_of(&store, "a"),
            Some(AcquiredState::Ended {
                allocation_ref: "alloc-a".into(),
                ended_ms: 80
            })
        );
        assert_eq!(state_of(&store, "c"), Some(AcquiredState::Pending));
        assert_eq!(store.acquired_machines(0).unwrap().len(), 2);
        assert!(store
            .acquired_machines(81)
            .unwrap()
            .iter()
            .all(|machine| machine.request_id != "a"));
        assert!(admit(&store, "d", None));
        assert!(
            write(&store, AcquiredMachineWrite::Withdraw { key: request("d") }),
            "the attempt that admitted a request withdraws it while it is pending"
        );
        assert!(admit(&store, "d", None));
        assert!(write(
            &store,
            AcquiredMachineWrite::Uncertain { key: request("c") }
        ));
        assert!(
            !write(&store, AcquiredMachineWrite::Withdraw { key: request("c") }),
            "only a settled outcome removes an uncertain request"
        );
        assert_eq!(state_of(&store, "c"), Some(AcquiredState::Uncertain));
        assert!(write(
            &store,
            AcquiredMachineWrite::Accept {
                key: request("c"),
                allocation_ref: "alloc-c".into(),
            }
        ));
        assert!(
            !write(
                &store,
                AcquiredMachineWrite::Uncertain { key: request("c") }
            ),
            "a running machine is never uncertain again"
        );
        assert!(write(
            &store,
            AcquiredMachineWrite::Uncertain { key: request("d") }
        ));
        assert!(write(
            &store,
            AcquiredMachineWrite::Remove { key: request("d") }
        ));
    }
}
