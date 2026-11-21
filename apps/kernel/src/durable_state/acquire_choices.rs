//! The machine option each agent's acquire request_id took and the allocation the service accepted
//! under it.
//! Every change is one compare-and-set on the row of (agent_id, request_id); the transitions and
//! why each one is safe are in `runtime/state/machine_acquisition/choice.rs`.
//!
//! This module owns the row's lifetime. A row stays for `REPLAY_WINDOW_MS` after its last write,
//! the time a retry of the request_id must still get the answer of the first try. After that no
//! read sees it, the next write removes it, and opening the store removes what a restart left.
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use super::{DurableKernelStateStore, DurableWriteOperation, FINISHED_RETENTION_MS};
use crate::error::DaemonError;

const REPLAY_WINDOW_MS: u64 = FINISHED_RETENTION_MS;

/// Rows last written at or before this time are past their replay window.
fn expired_before(now_ms: u64) -> i64 {
    now_ms.saturating_sub(REPLAY_WINDOW_MS) as i64
}

/// What an acquire asks the service for. Stored as its class_id and region; no class_id is the
/// service's default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AcquireOption {
    /// The service's own default, asked for when it lists no options.
    ServiceDefault,
    /// A listed class, in `region` or, without one, in the only region the service offers it in.
    Named {
        class_id: String,
        region: Option<String>,
    },
}

impl AcquireOption {
    pub(crate) fn class_id(&self) -> Option<&str> {
        match self {
            Self::ServiceDefault => None,
            Self::Named { class_id, .. } => Some(class_id),
        }
    }

    pub(crate) fn region(&self) -> Option<&str> {
        match self {
            Self::ServiceDefault => None,
            Self::Named { region, .. } => region.as_deref(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AcquireChoiceState {
    /// No acceptance is recorded: the service may still accept a try, refused or not.
    Pending,
    /// The service holds this allocation under the request.
    Accepted { allocation_ref: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AcquireChoice {
    pub(crate) option: AcquireOption,
    pub(crate) state: AcquireChoiceState,
}

/// One compare-and-set on the row of `agent_id` and `request_id`.
#[derive(Debug)]
pub(crate) enum AcquireChoiceWrite {
    /// No row becomes pending with `option`; a request's option is never written again.
    Choose { option: AcquireOption },
    /// The row becomes accepted with `option` and the allocation, whatever it was or whether it
    /// was removed meanwhile: the service's acceptance is the truth, and a try that waited for it
    /// past its window must still record it. Only a row accepted under another allocation or
    /// option stays as it is.
    Accept {
        option: AcquireOption,
        allocation_ref: String,
    },
    /// The service closed the request: nothing was or will be allocated under it. The row stays
    /// for the replay window, so a retry gets the same answer.
    Closed { option: AcquireOption },
}

#[derive(Debug)]
pub(crate) struct AcquireChoiceTransition {
    agent_id: String,
    request_id: String,
    write: AcquireChoiceWrite,
    now_ms: u64,
}

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS durable_acquire_choices (
            agent_id TEXT NOT NULL, request_id TEXT NOT NULL,
            class_id TEXT, region TEXT CHECK (class_id IS NOT NULL OR region IS NULL),
            state TEXT NOT NULL CHECK (state IN ('pending', 'accepted')),
            allocation_ref TEXT CHECK ((state = 'accepted') = (allocation_ref IS NOT NULL)),
            touched_ms INTEGER NOT NULL,
            PRIMARY KEY (agent_id, request_id)
        );
        CREATE INDEX IF NOT EXISTS durable_acquire_choices_touched
            ON durable_acquire_choices (touched_ms);
        CREATE TABLE IF NOT EXISTS durable_allocation_owners (
            allocation_ref TEXT PRIMARY KEY NOT NULL, agent_id TEXT NOT NULL
        );",
    )?;
    // Rows a restart left past their window: their try is long over.
    connection.execute(
        "DELETE FROM durable_acquire_choices WHERE touched_ms <= ?1",
        params![expired_before(crate::session::unix_epoch_ms())],
    )?;
    Ok(())
}

impl DurableKernelStateStore {
    pub(crate) fn acquire_choice(
        &self,
        agent_id: &str,
        request_id: &str,
        now_ms: u64,
    ) -> Result<Option<AcquireChoice>, DaemonError> {
        self.lock_connection("acquire_choices.read")?
            .query_row(
                "SELECT class_id, region, state, allocation_ref FROM durable_acquire_choices
                WHERE agent_id = ?1 AND request_id = ?2 AND touched_ms > ?3",
                params![agent_id, request_id, expired_before(now_ms)],
                |row| {
                    let state = match (row.get::<_, String>(2)?.as_str(), row.get(3)?) {
                        ("accepted", Some(allocation_ref)) => {
                            AcquireChoiceState::Accepted { allocation_ref }
                        }
                        ("pending", None) => AcquireChoiceState::Pending,
                        _ => return Err(rusqlite::Error::InvalidQuery),
                    };
                    let option = match (row.get(0)?, row.get(1)?) {
                        (Some(class_id), region) => AcquireOption::Named { class_id, region },
                        (None, None) => AcquireOption::ServiceDefault,
                        (None, Some(_)) => return Err(rusqlite::Error::InvalidQuery),
                    };
                    Ok(AcquireChoice { option, state })
                },
            )
            .optional()
            .map_err(|error| state_error(error.to_string()))
    }

    /// The accepted allocation belongs to its agent for its lifetime, independent of the
    /// request's replay window. Read current service state before using this association.
    pub(crate) fn allocation_belongs_to_agent(
        &self,
        allocation_ref: &str,
        agent_id: &str,
    ) -> Result<bool, DaemonError> {
        self.lock_connection("acquire_choices.allocation_owner")?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM durable_allocation_owners
                    WHERE allocation_ref = ?1 AND agent_id = ?2)",
                params![allocation_ref, agent_id],
                |row| row.get(0),
            )
            .map_err(|error| state_error(error.to_string()))
    }

    /// Applies `write` when the row is in the state it expects; false when it is not.
    pub(crate) fn write_acquire_choice(
        &self,
        agent_id: &str,
        request_id: &str,
        write: AcquireChoiceWrite,
        now_ms: u64,
    ) -> Result<bool, DaemonError> {
        self.writer
            .execute(DurableWriteOperation::AcquireChoice(
                AcquireChoiceTransition {
                    agent_id: agent_id.into(),
                    request_id: request_id.into(),
                    write,
                    now_ms,
                },
            ))
            .map(|changed| changed == 1)
    }
}

/// A row in another state changes nothing and leaves the rest of the batch to commit. Rows past
/// their window go first, so a write never meets one.
pub(super) fn apply(
    transaction: &Transaction<'_>,
    transition: &AcquireChoiceTransition,
) -> rusqlite::Result<u64> {
    let AcquireChoiceTransition {
        agent_id,
        request_id,
        write,
        now_ms,
    } = transition;
    transaction.execute(
        "DELETE FROM durable_acquire_choices WHERE touched_ms <= ?1",
        params![expired_before(*now_ms)],
    )?;
    if let AcquireChoiceWrite::Accept { allocation_ref, .. } = write {
        let other_agent: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM durable_allocation_owners
                WHERE allocation_ref = ?1 AND agent_id != ?2)",
            params![allocation_ref, agent_id],
            |row| row.get(0),
        )?;
        if other_agent {
            return Ok(0);
        }
    }
    let changed = match write {
        AcquireChoiceWrite::Choose { option } => transaction.execute(
            "INSERT INTO durable_acquire_choices
                (agent_id, request_id, class_id, region, state, touched_ms)
            VALUES (?1, ?2, ?3, ?4, 'pending', ?5)
            ON CONFLICT (agent_id, request_id) DO NOTHING",
            params![
                agent_id,
                request_id,
                option.class_id(),
                option.region(),
                *now_ms as i64
            ],
        )?,
        AcquireChoiceWrite::Accept {
            option,
            allocation_ref,
        } => transaction.execute(
            "INSERT INTO durable_acquire_choices
                (agent_id, request_id, class_id, region, state, allocation_ref, touched_ms)
            VALUES (?1, ?2, ?3, ?4, 'accepted', ?5, ?6)
            ON CONFLICT (agent_id, request_id) DO UPDATE SET
                class_id = excluded.class_id, region = excluded.region, state = 'accepted',
                allocation_ref = excluded.allocation_ref, touched_ms = excluded.touched_ms
            WHERE durable_acquire_choices.state != 'accepted'
                OR (durable_acquire_choices.allocation_ref = excluded.allocation_ref
                    AND durable_acquire_choices.class_id IS excluded.class_id
                    AND durable_acquire_choices.region IS excluded.region)",
            params![
                agent_id,
                request_id,
                option.class_id(),
                option.region(),
                allocation_ref,
                *now_ms as i64
            ],
        )?,
        AcquireChoiceWrite::Closed { option } => transaction.execute(
            "UPDATE durable_acquire_choices SET touched_ms = ?5
            WHERE agent_id = ?1 AND request_id = ?2 AND class_id IS ?3 AND region IS ?4
                AND state = 'pending'",
            params![
                agent_id,
                request_id,
                option.class_id(),
                option.region(),
                *now_ms as i64
            ],
        )?,
    };
    if changed > 0 {
        if let AcquireChoiceWrite::Accept { allocation_ref, .. } = write {
            transaction.execute(
                "INSERT INTO durable_allocation_owners (allocation_ref, agent_id) VALUES (?1, ?2)
                    ON CONFLICT (allocation_ref) DO NOTHING",
                params![allocation_ref, agent_id],
            )?;
        }
    }
    Ok(changed as u64)
}

fn state_error(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "acquire_choices",
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> u64 {
        crate::session::unix_epoch_ms()
    }

    fn option(class_id: &str) -> AcquireOption {
        AcquireOption::Named {
            class_id: class_id.into(),
            region: Some("north".into()),
        }
    }

    fn choose(class_id: &str) -> AcquireChoiceWrite {
        AcquireChoiceWrite::Choose {
            option: option(class_id),
        }
    }

    fn accept(class_id: &str, allocation_ref: &str) -> AcquireChoiceWrite {
        AcquireChoiceWrite::Accept {
            option: option(class_id),
            allocation_ref: allocation_ref.into(),
        }
    }

    #[test]
    fn allocation_authority_outlives_the_request_replay_window() {
        let dir = crate::test_support::TestTempDir::new("allocation-owner");
        let path = dir.path().join("kernel.db");
        let store = DurableKernelStateStore::open(&path).unwrap();
        let first = now();
        assert!(store
            .write_acquire_choice("asker", "r1", accept("small", "allocation"), first)
            .unwrap());
        assert!(!store
            .write_acquire_choice(
                "other",
                "r2",
                accept("small", "allocation"),
                first + REPLAY_WINDOW_MS + 1,
            )
            .unwrap());
        assert!(store
            .acquire_choice("asker", "r1", first + REPLAY_WINDOW_MS + 1)
            .unwrap()
            .is_none());
        assert!(store
            .allocation_belongs_to_agent("allocation", "asker")
            .unwrap());
        assert!(!store
            .allocation_belongs_to_agent("allocation", "other")
            .unwrap());
        assert!(!store
            .allocation_belongs_to_agent("foreign", "asker")
            .unwrap());
    }

    #[test]
    fn each_transition_applies_only_from_the_state_it_expects_and_survives_a_restart() {
        let directory = crate::test_support::TestTempDir::new("acquire-choices");
        let path = directory.path().join("kernel.db");
        let state = |store: &DurableKernelStateStore| {
            store
                .acquire_choice("agent", "r1", now())
                .unwrap()
                .map(|choice| (choice.option.class_id().unwrap().to_owned(), choice.state))
        };
        let pending = |class_id: &str| Some((class_id.to_owned(), AcquireChoiceState::Pending));
        {
            let store = DurableKernelStateStore::open(path.clone()).unwrap();
            assert_eq!(state(&store), None);
            assert!(store
                .write_acquire_choice("agent", "r1", choose("small"), now())
                .unwrap());
            // A concurrent try that chose second keeps the first choice.
            assert!(!store
                .write_acquire_choice("agent", "r1", choose("big"), now())
                .unwrap());
            assert_eq!(state(&store), pending("small"));
            // Each agent's request_id is its own.
            assert!(store
                .write_acquire_choice("other", "r1", choose("big"), now())
                .unwrap());
        }
        // Restarted after a try was sent: the retry resends "small".
        let store = DurableKernelStateStore::open(path.clone()).unwrap();
        assert_eq!(state(&store), pending("small"));
        assert!(!store
            .write_acquire_choice("agent", "r1", choose("big"), now())
            .unwrap());
        assert!(store
            .write_acquire_choice("agent", "r1", accept("small", "a1"), now())
            .unwrap());
        // Accepted is final: a replay of the same allocation is fine, nothing else changes it.
        assert!(store
            .write_acquire_choice("agent", "r1", accept("small", "a1"), now())
            .unwrap());
        assert!(!store
            .write_acquire_choice("agent", "r1", accept("small", "a2"), now())
            .unwrap());
        assert!(!store
            .write_acquire_choice("agent", "r1", accept("big", "a1"), now())
            .unwrap());
        assert!(!store
            .write_acquire_choice("agent", "r1", choose("big"), now())
            .unwrap());
        drop(store);
        let store = DurableKernelStateStore::open(path).unwrap();
        assert_eq!(
            state(&store),
            Some((
                "small".into(),
                AcquireChoiceState::Accepted {
                    allocation_ref: "a1".into()
                }
            ))
        );
    }

    #[test]
    fn the_service_default_is_a_choice_of_its_own_and_survives_a_restart() {
        let directory = crate::test_support::TestTempDir::new("acquire-default-choice");
        let path = directory.path().join("kernel.db");
        let default = || AcquireChoiceWrite::Choose {
            option: AcquireOption::ServiceDefault,
        };
        {
            let store = DurableKernelStateStore::open(path.clone()).unwrap();
            assert!(store
                .write_acquire_choice("agent", "r1", default(), now())
                .unwrap());
            assert!(!store
                .write_acquire_choice("agent", "r1", choose("small"), now())
                .unwrap());
        }
        let store = DurableKernelStateStore::open(path).unwrap();
        assert_eq!(
            store.acquire_choice("agent", "r1", now()).unwrap(),
            Some(AcquireChoice {
                option: AcquireOption::ServiceDefault,
                state: AcquireChoiceState::Pending,
            })
        );
        assert!(store
            .write_acquire_choice(
                "agent",
                "r1",
                AcquireChoiceWrite::Accept {
                    option: AcquireOption::ServiceDefault,
                    allocation_ref: "a1".into()
                },
                now()
            )
            .unwrap());
        assert_eq!(
            store
                .acquire_choice("agent", "r1", now())
                .unwrap()
                .unwrap()
                .option,
            AcquireOption::ServiceDefault
        );
    }

    fn state_at(
        store: &DurableKernelStateStore,
        agent_id: &str,
        now_ms: u64,
    ) -> Option<AcquireChoiceState> {
        store
            .acquire_choice(agent_id, "r1", now_ms)
            .unwrap()
            .map(|choice| choice.state)
    }

    #[test]
    fn a_row_is_read_for_its_window_after_its_last_write_and_the_next_write_removes_it() {
        let directory = crate::test_support::TestTempDir::new("acquire-choices-window");
        let store = DurableKernelStateStore::open(directory.path().join("kernel.db")).unwrap();
        let start = now();
        assert!(store
            .write_acquire_choice("agent", "r1", choose("small"), start)
            .unwrap());
        assert_eq!(
            state_at(&store, "agent", start + REPLAY_WINDOW_MS - 1),
            Some(AcquireChoiceState::Pending)
        );
        assert_eq!(state_at(&store, "agent", start + REPLAY_WINDOW_MS), None);
        // A write inside the window restarts it.
        assert!(store
            .write_acquire_choice("agent", "r1", accept("small", "a1"), start + 1_000)
            .unwrap());
        assert!(state_at(&store, "agent", start + REPLAY_WINDOW_MS).is_some());
        // The first write past the window removes the row and leaves other agents' rows alone.
        let later = start + 1_000 + REPLAY_WINDOW_MS;
        assert!(store
            .write_acquire_choice("other", "r1", choose("big"), later - 1)
            .unwrap());
        assert!(store
            .write_acquire_choice("other", "r2", choose("big"), later)
            .unwrap());
        assert_eq!(state_at(&store, "agent", start), None, "the row is gone");
        assert!(state_at(&store, "other", later).is_some());
        // A request_id used again after its window chooses anew.
        assert!(store
            .write_acquire_choice("agent", "r1", choose("big"), later)
            .unwrap());
        assert_eq!(
            store
                .acquire_choice("agent", "r1", later)
                .unwrap()
                .unwrap()
                .option,
            option("big")
        );
    }

    #[test]
    fn opening_the_store_removes_rows_a_restart_left_past_their_window() {
        let directory = crate::test_support::TestTempDir::new("acquire-choices-startup");
        let path = directory.path().join("kernel.db");
        let long_ago = now() - REPLAY_WINDOW_MS - 1;
        {
            let store = DurableKernelStateStore::open(path.clone()).unwrap();
            // A try died with the kernel in the middle of its request, long ago and just now.
            assert!(store
                .write_acquire_choice("agent", "r2", choose("small"), now())
                .unwrap());
            // Written with an old clock, as a row of an earlier run: no write of today removed it.
            assert!(store
                .write_acquire_choice("agent", "r1", choose("small"), long_ago)
                .unwrap());
            assert_eq!(
                state_at(&store, "agent", long_ago),
                Some(AcquireChoiceState::Pending)
            );
        }
        let store = DurableKernelStateStore::open(path).unwrap();
        assert_eq!(
            state_at(&store, "agent", long_ago),
            None,
            "the old row is gone"
        );
        assert_eq!(
            store
                .acquire_choice("agent", "r2", now())
                .unwrap()
                .map(|choice| choice.state),
            Some(AcquireChoiceState::Pending)
        );
    }

    #[test]
    fn a_closed_request_keeps_its_row_for_a_new_window() {
        let directory = crate::test_support::TestTempDir::new("acquire-choices-closed");
        let store = DurableKernelStateStore::open(directory.path().join("kernel.db")).unwrap();
        let start = now();
        let closed = || AcquireChoiceWrite::Closed {
            option: option("small"),
        };
        assert!(!store
            .write_acquire_choice("agent", "r1", closed(), start)
            .unwrap());
        assert!(store
            .write_acquire_choice("agent", "r1", choose("small"), start)
            .unwrap());
        assert!(!store
            .write_acquire_choice(
                "agent",
                "r1",
                AcquireChoiceWrite::Closed {
                    option: option("big")
                },
                start
            )
            .unwrap());
        assert!(store
            .write_acquire_choice("agent", "r1", closed(), start + REPLAY_WINDOW_MS - 1)
            .unwrap());
        assert!(state_at(&store, "agent", start + REPLAY_WINDOW_MS + 1).is_some());
    }

    /// A try waits for the service or the owner's decision while its row ages out, and another
    /// write sweeps the row before the try's response is recorded.
    #[test]
    fn an_acceptance_is_recorded_whether_its_row_is_swept_or_chosen_again_meanwhile() {
        let directory = crate::test_support::TestTempDir::new("acquire-choices-accept");
        let store = DurableKernelStateStore::open(directory.path().join("kernel.db")).unwrap();
        let start = now();
        let accepted = |allocation_ref: &str| {
            Some(AcquireChoiceState::Accepted {
                allocation_ref: allocation_ref.into(),
            })
        };
        // The row was swept: a write of another request past its window removed it.
        assert!(store
            .write_acquire_choice("agent", "r1", choose("small"), start)
            .unwrap());
        let swept = start + REPLAY_WINDOW_MS + 1;
        assert!(store
            .write_acquire_choice("other", "r1", choose("big"), swept)
            .unwrap());
        assert_eq!(state_at(&store, "agent", swept), None);
        assert!(store
            .write_acquire_choice("agent", "r1", accept("small", "a1"), swept + 1)
            .unwrap());
        assert_eq!(state_at(&store, "agent", swept + 1), accepted("a1"));
        // The row was swept and a retry chose another option: the service's acceptance wins.
        assert!(store
            .write_acquire_choice("agent", "r2", choose("small"), start)
            .unwrap());
        assert!(store
            .write_acquire_choice("agent", "r3", choose("small"), swept)
            .unwrap());
        assert!(store
            .write_acquire_choice("agent", "r2", choose("big"), swept)
            .unwrap());
        assert!(store
            .write_acquire_choice("agent", "r2", accept("small", "a2"), swept + 1)
            .unwrap());
        let row = store
            .acquire_choice("agent", "r2", swept + 1)
            .unwrap()
            .unwrap();
        assert_eq!(row.option, option("small"));
        assert_eq!(row.state, accepted("a2").unwrap());
        // The acceptance restarts the window.
        assert_eq!(
            state_at(&store, "agent", swept + 1 + REPLAY_WINDOW_MS - 1),
            accepted("a1")
        );
    }
}
