//! Bounded, one-shot App copy/link offers, following file-export ownership and
//! generation fencing. Only a human terminal can take the offered action.
use super::{DurableKernelStateStore, DurableWriterRequest};
use crate::local::AppHostAction;
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::mpsc;

pub(crate) const OFFER_MS: u64 = 5 * 60 * 1000;
const MAX_OPEN: i64 = 4;

#[derive(Clone)]
pub(crate) struct HostOffer {
    pub(crate) operation_id: String,
    pub(crate) owner: String,
    pub(crate) installation: String,
    pub(crate) generation: u64,
    pub(crate) action: AppHostAction,
    pub(crate) expires_ms: u64,
}

pub(crate) enum HostActionCommand {
    Create(HostOffer),
    Accept {
        owner: String,
        operation_id: String,
        now_ms: u64,
    },
    Decline {
        owner: String,
        operation_id: String,
        now_ms: u64,
    },
    Expire {
        now_ms: u64,
    },
}

pub(super) struct HostActionRequest {
    command: HostActionCommand,
    response: mpsc::Sender<Result<Option<HostOffer>, &'static str>>,
}
impl std::fmt::Debug for HostActionRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HostActionRequest(..)")
    }
}

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_host_actions (
            operation_id TEXT PRIMARY KEY, owner_id TEXT NOT NULL, installation_id TEXT NOT NULL,
            generation INTEGER NOT NULL, action_json TEXT,
            state TEXT NOT NULL CHECK(state IN ('pending','accepted','declined','expired')),
            expires_ms INTEGER NOT NULL, updated_ms INTEGER NOT NULL);
         CREATE INDEX IF NOT EXISTS app_host_actions_open ON app_host_actions(state, expires_ms);",
    )
}

const COLUMNS: &str =
    "operation_id, owner_id, installation_id, generation, action_json, expires_ms";
const ACTIVE: &str = "SELECT 1 FROM app_installations i
    WHERE i.installation_id=app_host_actions.installation_id
      AND i.owner_id=app_host_actions.owner_id
      AND i.generation=app_host_actions.generation AND i.active_json IS NOT NULL";

fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<HostOffer> {
    let json: String = row.get(4)?;
    let action = serde_json::from_str(&json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(HostOffer {
        operation_id: row.get(0)?,
        owner: row.get(1)?,
        installation: row.get(2)?,
        generation: row.get::<_, i64>(3)? as u64,
        action,
        expires_ms: row.get::<_, i64>(5)? as u64,
    })
}

impl DurableKernelStateStore {
    pub(crate) fn app_host_action(
        &self,
        command: HostActionCommand,
    ) -> Result<Option<HostOffer>, &'static str> {
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppHostAction(Box::new(
                HostActionRequest { command, response },
            )))
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        receiver.recv().map_err(|_| "STORAGE_UNAVAILABLE")?
    }

    pub(crate) fn pending_app_host_actions(
        &self,
        limit: usize,
    ) -> Result<Vec<HostOffer>, &'static str> {
        let connection = self
            .lock_connection("durable_state.app_host_actions_pending")
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        let mut statement = connection.prepare(&format!(
            "SELECT {COLUMNS} FROM (
                SELECT *, row_number() OVER (PARTITION BY owner_id, installation_id ORDER BY updated_ms, operation_id) AS rank
                FROM app_host_actions WHERE state='pending')
             WHERE rank=1 ORDER BY updated_ms LIMIT ?1"
        )).map_err(|_| "STORAGE_UNAVAILABLE")?;
        let rows = statement
            .query_map(params![limit as i64], decode)
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        rows.collect::<Result<_, _>>()
            .map_err(|_| "STORAGE_UNAVAILABLE")
    }
}

pub(super) fn execute(connection: &mut Connection, request: HostActionRequest) {
    let _ = request.response.send(apply(connection, request.command));
}

fn apply(
    connection: &mut Connection,
    command: HostActionCommand,
) -> Result<Option<HostOffer>, &'static str> {
    let storage = |_| "STORAGE_UNAVAILABLE";
    let transaction = connection.transaction().map_err(storage)?;
    let reply = match command {
        HostActionCommand::Create(offer) => {
            let active: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM app_installations WHERE owner_id=?1 AND installation_id=?2
                 AND generation=?3 AND active_json IS NOT NULL)",
                params![offer.owner, offer.installation, offer.generation as i64], |row| row.get(0)
            ).map_err(storage)?;
            if !active {
                return Err("CONFLICT");
            }
            let now_ms = crate::session::unix_epoch_ms();
            let open: i64 = transaction
                .query_row(
                    "SELECT count(*) FROM app_host_actions WHERE owner_id=?1 AND installation_id=?2
                 AND state='pending' AND expires_ms>?3",
                    params![offer.owner, offer.installation, now_ms as i64],
                    |row| row.get(0),
                )
                .map_err(storage)?;
            if open >= MAX_OPEN {
                return Err("LIMIT_EXCEEDED");
            }
            let json = serde_json::to_string(&offer.action).map_err(|_| "INVALID_ARGUMENT")?;
            transaction
                .execute(
                    "INSERT INTO app_host_actions VALUES (?1,?2,?3,?4,?5,'pending',?6,?7)",
                    params![
                        offer.operation_id,
                        offer.owner,
                        offer.installation,
                        offer.generation as i64,
                        json,
                        offer.expires_ms as i64,
                        now_ms as i64
                    ],
                )
                .map_err(storage)?;
            Some(offer)
        }
        HostActionCommand::Accept {
            owner,
            operation_id,
            now_ms,
        } => {
            let offer = transaction.query_row(&format!(
                "SELECT {COLUMNS} FROM app_host_actions WHERE operation_id=?1 AND owner_id=?2 AND state='pending'
                 AND expires_ms>?3 AND EXISTS ({ACTIVE})"
            ), params![operation_id, owner, now_ms as i64], decode).optional().map_err(storage)?;
            let Some(offer) = offer else {
                return Err("NOT_FOUND");
            };
            transaction.execute(
                "UPDATE app_host_actions SET state='accepted', action_json=NULL, updated_ms=?2 WHERE operation_id=?1",
                params![operation_id, now_ms as i64]
            ).map_err(storage)?;
            Some(offer)
        }
        HostActionCommand::Decline {
            owner,
            operation_id,
            now_ms,
        } => {
            transaction
                .execute(
                    "UPDATE app_host_actions SET state='declined', action_json=NULL, updated_ms=?3
                 WHERE operation_id=?1 AND owner_id=?2 AND state='pending'",
                    params![operation_id, owner, now_ms as i64],
                )
                .map_err(storage)?;
            None
        }
        HostActionCommand::Expire { now_ms } => {
            transaction
                .execute(
                    &format!(
                "UPDATE app_host_actions SET state='expired', action_json=NULL, updated_ms=?1
                 WHERE state='pending' AND (expires_ms<=?1 OR NOT EXISTS ({ACTIVE}))"
            ),
                    params![now_ms as i64],
                )
                .map_err(storage)?;
            transaction
                .execute(
                    "DELETE FROM app_host_actions WHERE state!='pending' AND updated_ms<?1",
                    params![now_ms.saturating_sub(24 * 60 * 60 * 1000) as i64],
                )
                .map_err(storage)?;
            None
        }
    };
    transaction.commit().map_err(storage)?;
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        root: std::path::PathBuf,
        store: DurableKernelStateStore,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "chariox-host-actions-{:016x}",
                rand::random::<u64>()
            ));
            std::fs::create_dir(&root).unwrap();
            let store = DurableKernelStateStore::open_owned(root.join("kernel.sqlite")).unwrap();
            Connection::open(store.path()).unwrap().execute_batch(
                "INSERT INTO app_installations(installation_id,app_id,owner_id,generation,allocated_generation,active_json)
                 VALUES('docs','com.example.docs','alice',3,3,'{}')"
            ).unwrap();
            Self { root, store }
        }
        fn offer(&self, id: &str) {
            self.store
                .app_host_action(HostActionCommand::Create(HostOffer {
                    operation_id: id.into(),
                    owner: "alice".into(),
                    installation: "docs".into(),
                    generation: 3,
                    action: AppHostAction::ClipboardWrite {
                        text: "fixture text".into(),
                    },
                    expires_ms: crate::session::unix_epoch_ms() + OFFER_MS,
                }))
                .unwrap();
        }
        fn accept(
            &self,
            id: &str,
            owner: &str,
            now_ms: u64,
        ) -> Result<Option<HostOffer>, &'static str> {
            self.store.app_host_action(HostActionCommand::Accept {
                owner: owner.into(),
                operation_id: id.into(),
                now_ms,
            })
        }
        fn dropped(&self, id: &str) -> bool {
            Connection::open(self.store.path())
                .unwrap()
                .query_row(
                    "SELECT action_json IS NULL FROM app_host_actions WHERE operation_id=?1",
                    params![id],
                    |row| row.get(0),
                )
                .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn app_host_offers_are_owner_only_one_shot_and_drop_payloads_when_settled() {
        let f = Fixture::new();
        f.offer("copy");
        let now = crate::session::unix_epoch_ms();
        assert!(matches!(f.accept("copy", "bob", now), Err("NOT_FOUND")));
        assert!(!f.dropped("copy"));
        assert_eq!(
            f.accept("copy", "alice", now).unwrap().unwrap().action,
            AppHostAction::ClipboardWrite {
                text: "fixture text".into()
            }
        );
        assert!(f.dropped("copy"));
        assert!(matches!(f.accept("copy", "alice", now), Err("NOT_FOUND")));
        f.offer("decline");
        f.store
            .app_host_action(HostActionCommand::Decline {
                owner: "bob".into(),
                operation_id: "decline".into(),
                now_ms: now,
            })
            .unwrap();
        assert!(!f.dropped("decline"));
        f.store
            .app_host_action(HostActionCommand::Decline {
                owner: "alice".into(),
                operation_id: "decline".into(),
                now_ms: now,
            })
            .unwrap();
        assert!(f.dropped("decline"));
        assert!(matches!(
            f.accept("decline", "alice", now),
            Err("NOT_FOUND")
        ));
    }

    #[test]
    fn app_host_expiry_update_and_uninstall_fence_acceptance_and_drop_payloads() {
        let f = Fixture::new();
        f.offer("expires");
        let expired = crate::session::unix_epoch_ms() + OFFER_MS + 1;
        assert!(matches!(
            f.accept("expires", "alice", expired),
            Err("NOT_FOUND")
        ));
        f.store
            .app_host_action(HostActionCommand::Expire { now_ms: expired })
            .unwrap();
        assert!(f.dropped("expires"));
        for update in [
            "UPDATE app_installations SET generation=4, allocated_generation=4",
            "UPDATE app_installations SET active_json=NULL",
        ] {
            Connection::open(f.store.path())
                .unwrap()
                .execute_batch("UPDATE app_installations SET generation=3, active_json='{}'")
                .unwrap();
            f.offer("stale");
            Connection::open(f.store.path())
                .unwrap()
                .execute_batch(update)
                .unwrap();
            assert!(matches!(
                f.accept("stale", "alice", crate::session::unix_epoch_ms()),
                Err("NOT_FOUND")
            ));
            f.store
                .app_host_action(HostActionCommand::Expire {
                    now_ms: crate::session::unix_epoch_ms(),
                })
                .unwrap();
            assert!(f.dropped("stale"));
            Connection::open(f.store.path())
                .unwrap()
                .execute(
                    "DELETE FROM app_host_actions WHERE operation_id='stale'",
                    [],
                )
                .unwrap();
        }
    }

    #[test]
    fn app_host_pending_count_is_bounded_and_inactive_generation_cannot_offer() {
        let f = Fixture::new();
        for i in 0..4 {
            f.offer(&format!("offer-{i}"));
        }
        let offer = HostOffer {
            operation_id: "excess".into(),
            owner: "alice".into(),
            installation: "docs".into(),
            generation: 3,
            action: AppHostAction::OpenLink {
                url: "https://example.org".into(),
            },
            expires_ms: crate::session::unix_epoch_ms() + OFFER_MS,
        };
        assert!(matches!(
            f.store
                .app_host_action(HostActionCommand::Create(offer.clone())),
            Err("LIMIT_EXCEEDED")
        ));
        assert!(matches!(
            f.store
                .app_host_action(HostActionCommand::Create(HostOffer {
                    generation: 2,
                    ..offer
                })),
            Err("CONFLICT")
        ));
    }
}
