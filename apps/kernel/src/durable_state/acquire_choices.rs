//! The machine option each agent's acquire request_id took and what the service said about it.
//! Every change is one compare-and-set on the row of (agent_id, request_id); the transitions and
//! why each one is safe are in `runtime/state/machine_acquisition/choice.rs`.
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use super::{DurableKernelStateStore, DurableWriteOperation};
use crate::error::DaemonError;

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
    /// Sent, or about to be sent, with no answer that settles it.
    Pending,
    /// The service holds this allocation under the request.
    Accepted { allocation_ref: String },
    /// The service refused the request and holds nothing under it.
    Refused,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AcquireChoice {
    pub(crate) option: AcquireOption,
    pub(crate) state: AcquireChoiceState,
}

/// One compare-and-set on the row of `agent_id` and `request_id`.
#[derive(Debug)]
pub(crate) enum AcquireChoiceWrite {
    /// No row or a refused one becomes pending with `option`.
    Choose { option: AcquireOption },
    /// Any row but one accepted under another allocation becomes accepted with `option`.
    Accept {
        option: AcquireOption,
        allocation_ref: String,
    },
    /// A pending row with `option` becomes refused.
    Refuse { option: AcquireOption },
}

#[derive(Debug)]
pub(crate) struct AcquireChoiceTransition {
    agent_id: String,
    request_id: String,
    write: AcquireChoiceWrite,
}

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS durable_acquire_choices (
            agent_id TEXT NOT NULL, request_id TEXT NOT NULL,
            class_id TEXT, region TEXT CHECK (class_id IS NOT NULL OR region IS NULL),
            state TEXT NOT NULL CHECK (state IN ('pending', 'accepted', 'refused')),
            allocation_ref TEXT CHECK ((state = 'accepted') = (allocation_ref IS NOT NULL)),
            PRIMARY KEY (agent_id, request_id)
        );",
    )
}

impl DurableKernelStateStore {
    pub(crate) fn acquire_choice(
        &self,
        agent_id: &str,
        request_id: &str,
    ) -> Result<Option<AcquireChoice>, DaemonError> {
        self.lock_connection("acquire_choices.read")?
            .query_row(
                "SELECT class_id, region, state, allocation_ref FROM durable_acquire_choices
                WHERE agent_id = ?1 AND request_id = ?2",
                params![agent_id, request_id],
                |row| {
                    let state = match (row.get::<_, String>(2)?.as_str(), row.get(3)?) {
                        ("accepted", Some(allocation_ref)) => {
                            AcquireChoiceState::Accepted { allocation_ref }
                        }
                        ("refused", None) => AcquireChoiceState::Refused,
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

    /// Applies `write` when the row is in the state it expects; false when it is not.
    pub(crate) fn write_acquire_choice(
        &self,
        agent_id: &str,
        request_id: &str,
        write: AcquireChoiceWrite,
    ) -> Result<bool, DaemonError> {
        self.writer
            .execute(DurableWriteOperation::AcquireChoice(
                AcquireChoiceTransition {
                    agent_id: agent_id.into(),
                    request_id: request_id.into(),
                    write,
                },
            ))
            .map(|changed| changed == 1)
    }
}

/// A row in another state changes nothing and leaves the rest of the batch to commit.
pub(super) fn apply(
    transaction: &Transaction<'_>,
    transition: &AcquireChoiceTransition,
) -> rusqlite::Result<u64> {
    let AcquireChoiceTransition {
        agent_id,
        request_id,
        write,
    } = transition;
    let changed = match write {
        AcquireChoiceWrite::Choose { option } => transaction.execute(
            "INSERT INTO durable_acquire_choices (agent_id, request_id, class_id, region, state)
            VALUES (?1, ?2, ?3, ?4, 'pending')
            ON CONFLICT (agent_id, request_id) DO UPDATE SET
                class_id = excluded.class_id, region = excluded.region, state = 'pending'
            WHERE state = 'refused'",
            params![agent_id, request_id, option.class_id(), option.region()],
        )?,
        AcquireChoiceWrite::Accept {
            option,
            allocation_ref,
        } => transaction.execute(
            "UPDATE durable_acquire_choices
            SET class_id = ?3, region = ?4, state = 'accepted', allocation_ref = ?5
            WHERE agent_id = ?1 AND request_id = ?2
                AND (state != 'accepted' OR allocation_ref = ?5)",
            params![
                agent_id,
                request_id,
                option.class_id(),
                option.region(),
                allocation_ref
            ],
        )?,
        AcquireChoiceWrite::Refuse { option } => transaction.execute(
            "UPDATE durable_acquire_choices SET state = 'refused'
            WHERE agent_id = ?1 AND request_id = ?2 AND state = 'pending'
                AND class_id IS ?3 AND region IS ?4",
            params![agent_id, request_id, option.class_id(), option.region()],
        )?,
    };
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

    fn refuse(class_id: &str) -> AcquireChoiceWrite {
        AcquireChoiceWrite::Refuse {
            option: option(class_id),
        }
    }

    #[test]
    fn each_transition_applies_only_from_the_state_it_expects_and_survives_a_restart() {
        let directory = crate::test_support::TestTempDir::new("acquire-choices");
        let path = directory.path().join("kernel.db");
        let state = |store: &DurableKernelStateStore| {
            store
                .acquire_choice("agent", "r1")
                .unwrap()
                .map(|choice| (choice.option.class_id().unwrap().to_owned(), choice.state))
        };
        let pending = |class_id: &str| Some((class_id.to_owned(), AcquireChoiceState::Pending));
        {
            let store = DurableKernelStateStore::open(path.clone()).unwrap();
            assert_eq!(state(&store), None);
            assert!(!store
                .write_acquire_choice("agent", "r1", refuse("small"))
                .unwrap());
            assert!(!store
                .write_acquire_choice("agent", "r1", accept("small", "a1"))
                .unwrap());
            assert!(store
                .write_acquire_choice("agent", "r1", choose("small"))
                .unwrap());
            // A concurrent try that chose second keeps the first choice.
            assert!(!store
                .write_acquire_choice("agent", "r1", choose("big"))
                .unwrap());
            // A refusal of an option this row does not hold changes nothing.
            assert!(!store
                .write_acquire_choice("agent", "r1", refuse("big"))
                .unwrap());
            assert_eq!(state(&store), pending("small"));
            // Each agent's request_id is its own.
            assert!(store
                .write_acquire_choice("other", "r1", choose("big"))
                .unwrap());
        }
        // Restarted after the service committed and before it answered: the retry sends "small".
        let store = DurableKernelStateStore::open(path.clone()).unwrap();
        assert_eq!(state(&store), pending("small"));
        assert!(store
            .write_acquire_choice("agent", "r1", refuse("small"))
            .unwrap());
        assert_eq!(
            state(&store),
            Some(("small".into(), AcquireChoiceState::Refused))
        );
        // A refused request chooses again, and the service's acceptance wins over a refusal.
        assert!(store
            .write_acquire_choice("agent", "r1", choose("big"))
            .unwrap());
        assert!(store
            .write_acquire_choice("agent", "r1", refuse("big"))
            .unwrap());
        assert!(store
            .write_acquire_choice("agent", "r1", accept("big", "a1"))
            .unwrap());
        // Accepted is final: a replay of the same allocation is fine, nothing else changes it.
        assert!(store
            .write_acquire_choice("agent", "r1", accept("big", "a1"))
            .unwrap());
        assert!(!store
            .write_acquire_choice("agent", "r1", accept("big", "a2"))
            .unwrap());
        assert!(!store
            .write_acquire_choice("agent", "r1", refuse("big"))
            .unwrap());
        assert!(!store
            .write_acquire_choice("agent", "r1", choose("small"))
            .unwrap());
        drop(store);
        let store = DurableKernelStateStore::open(path).unwrap();
        assert_eq!(
            state(&store),
            Some((
                "big".into(),
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
                .write_acquire_choice("agent", "r1", default())
                .unwrap());
            assert!(!store
                .write_acquire_choice("agent", "r1", choose("small"))
                .unwrap());
            // A refusal of a named option is not a refusal of the default.
            assert!(!store
                .write_acquire_choice("agent", "r1", refuse("small"))
                .unwrap());
        }
        let store = DurableKernelStateStore::open(path).unwrap();
        assert_eq!(
            store.acquire_choice("agent", "r1").unwrap(),
            Some(AcquireChoice {
                option: AcquireOption::ServiceDefault,
                state: AcquireChoiceState::Pending
            })
        );
        assert!(store
            .write_acquire_choice(
                "agent",
                "r1",
                AcquireChoiceWrite::Accept {
                    option: AcquireOption::ServiceDefault,
                    allocation_ref: "a1".into()
                }
            )
            .unwrap());
        assert_eq!(
            store.acquire_choice("agent", "r1").unwrap().unwrap().option,
            AcquireOption::ServiceDefault
        );
    }
}
