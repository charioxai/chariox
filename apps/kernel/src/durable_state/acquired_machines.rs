//! The machines agents acquired without asking the owner: one row per agent request, with its
//! class, rate, start and end. Every write is a compare-and-set on the row's state, so concurrent
//! requests, a lost answer, a cancel and a stale list can never move a row the wrong way; the
//! decisions are in `runtime/state/machine_acquisition/without_asking.rs`.
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use super::{DurableKernelStateStore, DurableWriteOperation};
use crate::error::DaemonError;

/// Where one acquisition is in its life. One row per request (agent and request_id). While it is
/// pending or uncertain the row counts its attempts in flight: each attempt the agent sends holds
/// one count, from the step that registers it until its own answer settles it, once. Every
/// transition is one compare-and-set in one transaction of the kernel's single durable writer, so
/// no other transition can come between its check and its write.
///
/// | transition | caller, for one attempt or row | row before → after | carries |
/// |---|---|---|---|
/// | `Attempt`, request recorded and unsettled | the attempt's admission (`admit_without_asking`) | `Pending`/`Uncertain`, n in flight → the same, n + 1 | a counted hold; the service replays its answer |
/// | `Attempt`, request recorded and settled | the attempt's admission | `Running`/`Deleting`/`Ended` → unchanged | an uncounted hold: its answer only replays the settled one |
/// | `Attempt`, admitted | the attempt's admission, within the owner's classes | none → `Pending`, 1 in flight, while fewer than the maximum rows are live and today's cost is below the cap; otherwise none, and the owner is asked | a counted hold |
/// | `Accept` | the attempt answered with a machine (`settle_acquisition`), or a fenced job cleanup that found it | `Pending`/`Uncertain` → `Running`, none in flight; when the allocation is already another row's, this row is removed | the allocation |
/// | `Release` | the attempt refused (`settle_acquisition`) or never sent (`withdraw_acquisition`) | `Pending`, 1 in flight → removed: no attempt can still make a machine. `Pending`, n > 1 → n − 1. `Uncertain`, n > 0 → n − 1: a lost answer may still hide a machine | the counted hold, consumed |
/// | `Lost` | the attempt's answer lost (`settle_acquisition`) | `Pending`/`Uncertain`, n > 0 → `Uncertain`, n − 1 | the counted hold, consumed; the agent is told the request_id to retry with |
/// | `Remove` | a fenced job cleanup that found no machine | `Pending`/`Uncertain` → removed | the job |
/// | kernel start | `initialize` | `Pending`/`Uncertain` → `Uncertain`, none in flight: no answer sent before can arrive | — |
/// | `Deleting` | the service says the machine is being deleted | `Running` → `Deleting` | the allocation |
/// | `End` | the service confirms the deletion, or a list sent while it ran no longer has it | `Running`/`Deleting` → `Ended` | the allocation and its end |
///
/// Every live state counts toward the maximum and costs from its start; `Ended` costs until its
/// end. A hold dropped without its answer keeps its count, so the request stays counted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AcquiredState {
    /// Every attempt sent for it is waiting for its answer, or was refused: once the last attempt
    /// is refused, no machine exists.
    Pending,
    /// An attempt's answer was lost: a machine may exist that no refusal can rule out.
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

pub(crate) const MINUTE_MS: u64 = 60_000;

impl AcquiredMachine {
    /// What it costs between `from_ms` and `until_ms`, by started minute.
    pub(crate) fn cost_micros(&self, from_ms: u64, until_ms: u64) -> u64 {
        let end = match &self.state {
            AcquiredState::Ended { ended_ms, .. } => (*ended_ms).min(until_ms),
            AcquiredState::Pending
            | AcquiredState::Uncertain
            | AcquiredState::Running { .. }
            | AcquiredState::Deleting { .. } => until_ms,
        };
        let start = self.started_ms.max(from_ms);
        if end <= start {
            return 0;
        }
        (end - start).div_ceil(MINUTE_MS) * self.rate_micros
    }
}

/// One attempt at an agent's request, registered in one step.
#[derive(Debug)]
pub(crate) enum AcquisitionAttempt {
    /// Counted only when its request is already recorded.
    Recorded {
        agent_id: String,
        request_id: String,
    },
    /// Within the owner's classes: counted in its request's row, which is created pending when
    /// there is none, while fewer than `max_live` rows are live and the cost of the day that began
    /// at `day_ms` is below `daily_cap_micros`.
    Admit {
        machine: AcquiredMachine,
        max_live: u32,
        daily_cap_micros: Option<u64>,
        day_ms: u64,
    },
}

/// How an attempt counts in its request's row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Registered {
    /// It holds one count in its request's pending or uncertain row, which its answer settles.
    Counted,
    /// Its request is settled (running, deleting or ended): its answer replays that one.
    Replay,
}

/// What an admission within the owner's classes became.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Admitted {
    Registered(Registered),
    /// Not admitted: the most rows are live.
    AtMaximum,
    /// Not admitted: today's cost reached the cap.
    AtCap,
}

// What `register` answers through the writer, which carries a number.
const NOT_RECORDED: u64 = 0;
const COUNTED: u64 = 1;
const REPLAY: u64 = 2;
const AT_MAXIMUM: u64 = 3;
const AT_CAP: u64 = 4;

fn registered(code: u64) -> Result<Registered, DaemonError> {
    match code {
        COUNTED => Ok(Registered::Counted),
        REPLAY => Ok(Registered::Replay),
        code => Err(state_error(format!(
            "an attempt's registration answered {code}, which it never answers"
        ))),
    }
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
    /// Registers one attempt: see `AcquisitionAttempt`.
    Attempt(AcquisitionAttempt),
    /// A pending or uncertain row the service accepted runs. When the allocation is already another
    /// row's (the service replayed an earlier request), this row leaves instead: one machine is one
    /// row.
    Accept {
        key: AcquisitionKey,
        allocation_ref: String,
    },
    /// One attempt of a pending or uncertain request was refused or never sent. A pending row whose
    /// last attempt this was leaves the ledger; an uncertain row stays.
    Release { key: AcquisitionKey },
    /// One attempt's answer was lost: its pending or uncertain row becomes uncertain.
    Lost { key: AcquisitionKey },
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
            attempts INTEGER NOT NULL CHECK (CASE state WHEN 'pending' THEN attempts > 0
                WHEN 'uncertain' THEN attempts >= 0 ELSE attempts = 0 END),
            PRIMARY KEY (agent_id, request_id)
        );
        -- The kernel is starting: no answer to an attempt sent before can still arrive.
        UPDATE durable_acquired_machines SET state = 'uncertain', attempts = 0
        WHERE state IN ('pending', 'uncertain');",
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

    /// The row of the machine `allocation_ref`.
    pub(crate) fn acquired_machine(
        &self,
        allocation_ref: &str,
    ) -> Result<Option<AcquiredMachine>, DaemonError> {
        let connection = self.lock_connection("acquired_machines.read")?;
        connection
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM durable_acquired_machines WHERE allocation_ref = ?1"
                ),
                params![allocation_ref],
                read_row,
            )
            .optional()
            .map_err(|error| state_error(error.to_string()))
    }

    /// Counts one more attempt at a request already recorded, in one step; none when it is not.
    pub(crate) fn count_recorded_attempt(
        &self,
        agent_id: &str,
        request_id: &str,
    ) -> Result<Option<Registered>, DaemonError> {
        match self.register_attempt(AcquisitionAttempt::Recorded {
            agent_id: agent_id.into(),
            request_id: request_id.into(),
        })? {
            NOT_RECORDED => Ok(None),
            code => registered(code).map(Some),
        }
    }

    /// Admits one attempt within the owner's classes, in one step: see
    /// `AcquisitionAttempt::Admit`.
    pub(crate) fn admit_attempt(
        &self,
        machine: AcquiredMachine,
        max_live: u32,
        daily_cap_micros: Option<u64>,
        day_ms: u64,
    ) -> Result<Admitted, DaemonError> {
        match self.register_attempt(AcquisitionAttempt::Admit {
            machine,
            max_live,
            daily_cap_micros,
            day_ms,
        })? {
            AT_MAXIMUM => Ok(Admitted::AtMaximum),
            AT_CAP => Ok(Admitted::AtCap),
            code => registered(code).map(Admitted::Registered),
        }
    }

    fn register_attempt(&self, attempt: AcquisitionAttempt) -> Result<u64, DaemonError> {
        self.writer.execute(DurableWriteOperation::AcquiredMachine(
            AcquiredMachineWrite::Attempt(attempt),
        ))
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
        AcquiredMachineWrite::Attempt(attempt) => return register(transaction, attempt),
        AcquiredMachineWrite::Accept {
            key,
            allocation_ref,
        } => {
            let Some((agent_id, request_id)) = resolve(transaction, key)? else {
                return Ok(0);
            };
            let started = transaction.execute(
                "UPDATE durable_acquired_machines
                SET state = 'running', allocation_ref = ?3, attempts = 0
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
        AcquiredMachineWrite::Release { key } => {
            let Some((agent_id, request_id)) = resolve(transaction, key)? else {
                return Ok(0);
            };
            let removed = transaction.execute(
                "DELETE FROM durable_acquired_machines
                WHERE agent_id = ?1 AND request_id = ?2 AND state = 'pending' AND attempts = 1",
                params![agent_id, request_id],
            )?;
            if removed > 0 {
                removed
            } else {
                transaction.execute(
                    "UPDATE durable_acquired_machines SET attempts = attempts - 1
                    WHERE agent_id = ?1 AND request_id = ?2 AND state IN ('pending', 'uncertain')
                        AND attempts > 0",
                    params![agent_id, request_id],
                )?
            }
        }
        AcquiredMachineWrite::Lost { key } => {
            let Some((agent_id, request_id)) = resolve(transaction, key)? else {
                return Ok(0);
            };
            transaction.execute(
                "UPDATE durable_acquired_machines SET state = 'uncertain', attempts = attempts - 1
                WHERE agent_id = ?1 AND request_id = ?2 AND state IN ('pending', 'uncertain')
                    AND attempts > 0",
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

/// Registers `attempt` in one transaction: counted in its request's unsettled row, a replay of its
/// settled row, or, for an admission within the limits, its request's new pending row.
fn register(transaction: &Transaction<'_>, attempt: &AcquisitionAttempt) -> rusqlite::Result<u64> {
    let (agent_id, request_id) = match attempt {
        AcquisitionAttempt::Recorded {
            agent_id,
            request_id,
        } => (agent_id, request_id),
        AcquisitionAttempt::Admit { machine, .. } => (&machine.agent_id, &machine.request_id),
    };
    let counted = transaction.execute(
        "UPDATE durable_acquired_machines SET attempts = attempts + 1
        WHERE agent_id = ?1 AND request_id = ?2 AND state IN ('pending', 'uncertain')",
        params![agent_id, request_id],
    )?;
    if counted > 0 {
        return Ok(COUNTED);
    }
    let recorded: bool = transaction.query_row(
        "SELECT EXISTS (SELECT 1 FROM durable_acquired_machines
            WHERE agent_id = ?1 AND request_id = ?2)",
        params![agent_id, request_id],
        |row| row.get(0),
    )?;
    if recorded {
        return Ok(REPLAY);
    }
    let AcquisitionAttempt::Admit {
        machine,
        max_live,
        daily_cap_micros,
        day_ms,
    } = attempt
    else {
        return Ok(NOT_RECORDED);
    };
    let mut today = transaction.prepare(&format!(
        "SELECT {COLUMNS} FROM durable_acquired_machines WHERE state != 'ended' OR ended_ms >= ?1"
    ))?;
    let today = today
        .query_map(params![*day_ms as i64], read_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let spent: u64 = today
        .iter()
        .map(|row| row.cost_micros(*day_ms, machine.started_ms))
        .sum();
    if daily_cap_micros.is_some_and(|cap| spent >= cap) {
        return Ok(AT_CAP);
    }
    let live = today.iter().filter(|row| row.state.is_live()).count();
    if live >= *max_live as usize {
        return Ok(AT_MAXIMUM);
    }
    transaction.execute(
        "INSERT INTO durable_acquired_machines
            (agent_id, request_id, job_ref, room_id, class_id, rate_micros, currency, state,
                started_ms, attempts)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?8, 1)",
        params![
            machine.agent_id,
            machine.request_id,
            machine.job_ref,
            machine.room_id,
            machine.class_id,
            machine.rate_micros as i64,
            machine.currency,
            machine.started_ms as i64,
        ],
    )?;
    Ok(COUNTED)
}

fn state_error(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "acquired_machines",
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::Registered::{Counted, Replay};
    use super::*;

    const ADMITTED: Admitted = Admitted::Registered(Counted);

    fn request(request_id: &str) -> AcquisitionKey {
        AcquisitionKey::Request {
            agent_id: "agent".into(),
            request_id: request_id.into(),
        }
    }

    fn admit_within(
        store: &DurableKernelStateStore,
        request_id: &str,
        job_ref: Option<&str>,
        (daily_cap_micros, started_ms): (Option<u64>, u64),
    ) -> Admitted {
        store
            .admit_attempt(
                AcquiredMachine {
                    agent_id: "agent".into(),
                    request_id: request_id.into(),
                    job_ref: job_ref.map(str::to_owned),
                    room_id: "room".into(),
                    class_id: "small".into(),
                    rate_micros: 1_000,
                    currency: "EUR".into(),
                    started_ms,
                    state: AcquiredState::Pending,
                },
                2,
                daily_cap_micros,
                0,
            )
            .unwrap()
    }

    fn admit(store: &DurableKernelStateStore, request_id: &str, job_ref: Option<&str>) -> Admitted {
        admit_within(store, request_id, job_ref, (None, 10))
    }

    fn retry(store: &DurableKernelStateStore, request_id: &str) -> Option<Registered> {
        store.count_recorded_attempt("agent", request_id).unwrap()
    }

    fn write(store: &DurableKernelStateStore, write: AcquiredMachineWrite) -> bool {
        store.write_acquired_machine(write).unwrap()
    }

    fn state_of(store: &DurableKernelStateStore, request_id: &str) -> Option<AcquiredState> {
        store
            .acquired_machines(0)
            .unwrap()
            .into_iter()
            .find(|machine| machine.agent_id == "agent" && machine.request_id == request_id)
            .map(|machine| machine.state)
    }

    #[test]
    fn every_transition_is_a_compare_and_set_and_only_an_end_frees_a_place_after_a_restart() {
        let directory = crate::test_support::TestTempDir::new("acquired-machines");
        let path = directory.path().join("kernel.db");
        {
            let store = DurableKernelStateStore::open(path.clone()).unwrap();
            assert_eq!(admit(&store, "a", None), ADMITTED);
            assert_eq!(
                admit(&store, "a", None),
                ADMITTED,
                "a retry is counted in its request's place and takes no second one"
            );
            assert_eq!(admit(&store, "b", Some("job-b")), ADMITTED);
            assert_eq!(
                admit(&store, "c", None),
                Admitted::AtMaximum,
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
        let store = DurableKernelStateStore::open(path.clone()).unwrap();
        assert_eq!(admit(&store, "c", None), ADMITTED);
        assert_eq!(
            admit(&store, "d", None),
            Admitted::AtMaximum,
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
        assert_eq!(admit(&store, "d", None), ADMITTED);
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
        assert_eq!(
            admit_within(&store, "d", None, (Some(2_000), 100)),
            Admitted::AtCap,
            "the ended machine's minute and the pending one's reach today's cap"
        );
        assert_eq!(
            admit_within(&store, "d", None, (Some(2_001), 100)),
            ADMITTED,
            "below the cap, with a place free"
        );
        assert_eq!(retry(&store, "d"), Some(Counted));
        assert!(write(
            &store,
            AcquiredMachineWrite::Release { key: request("d") }
        ));
        assert_eq!(
            state_of(&store, "d"),
            Some(AcquiredState::Pending),
            "the retry still waits for its answer"
        );
        assert!(write(
            &store,
            AcquiredMachineWrite::Release { key: request("d") }
        ));
        assert_eq!(
            state_of(&store, "d"),
            None,
            "every attempt was refused, so no machine exists"
        );
        assert!(
            !write(&store, AcquiredMachineWrite::Release { key: request("d") }),
            "each attempt releases once"
        );
        assert_eq!(retry(&store, "e"), None, "a retry never creates a row");
        assert_eq!(state_of(&store, "e"), None);
        assert_eq!(admit(&store, "d", None), ADMITTED);
        assert!(write(
            &store,
            AcquiredMachineWrite::Lost { key: request("c") }
        ));
        assert!(
            !write(&store, AcquiredMachineWrite::Release { key: request("c") }),
            "a lost answer is not released by a refusal"
        );
        assert_eq!(retry(&store, "c"), Some(Counted));
        assert!(write(
            &store,
            AcquiredMachineWrite::Release { key: request("c") }
        ));
        assert_eq!(
            state_of(&store, "c"),
            Some(AcquiredState::Uncertain),
            "only a settled outcome removes an uncertain request"
        );
        assert!(write(
            &store,
            AcquiredMachineWrite::Accept {
                key: request("c"),
                allocation_ref: "alloc-c".into(),
            }
        ));
        assert_eq!(
            retry(&store, "c"),
            Some(Replay),
            "a retry of a settled request only replays its answer"
        );
        assert!(
            !write(&store, AcquiredMachineWrite::Lost { key: request("c") }),
            "a running machine is never uncertain again"
        );
        drop(store);
        let store = DurableKernelStateStore::open(path).unwrap();
        assert_eq!(
            state_of(&store, "d"),
            Some(AcquiredState::Uncertain),
            "an attempt sent before a restart never gets its answer"
        );
        assert!(!write(
            &store,
            AcquiredMachineWrite::Release { key: request("d") }
        ));
        assert!(write(
            &store,
            AcquiredMachineWrite::Remove { key: request("d") }
        ));
    }
}
