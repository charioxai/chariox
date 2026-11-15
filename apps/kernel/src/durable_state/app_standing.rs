//! What the owner has decided once, for an App's action class in one repository: a standing
//! permission lets that class run without its card. It exists only while the account binding
//! it was made under and the installation's active release do; the person makes and ends it,
//! never an App or an agent.
use super::{DurableKernelStateStore, DurableWriterRequest};
use chariox_app_package::v2::StandingClass;
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::mpsc;

/// Standing permissions per installation.
const MAX_STANDING: i64 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StandingPermission {
    pub(crate) id: String,
    pub(crate) account: String,
    /// `owner/name`, lowercase.
    pub(crate) repo: String,
    pub(crate) class: StandingClass,
    pub(crate) created_at_ms: u64,
    /// `None`: until the owner revokes it.
    pub(crate) expires_at_ms: Option<u64>,
    pub(crate) uses: u64,
    pub(crate) last_used_ms: Option<u64>,
}

pub(crate) enum StandingCommand {
    Grant {
        owner: String,
        installation: String,
        permission: StandingPermission,
    },
    Revoke {
        owner: String,
        installation: String,
        id: String,
    },
    /// An action about to ask for its card: the permission that covers it, counted as used.
    Use {
        owner: String,
        installation: String,
        repo: String,
        class: StandingClass,
        now_ms: u64,
    },
}

pub(super) struct StandingRequest {
    command: StandingCommand,
    response: mpsc::Sender<Result<Option<String>, &'static str>>,
}
impl std::fmt::Debug for StandingRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StandingRequest(..)")
    }
}

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_standing_permissions (
            owner_id TEXT NOT NULL, installation_id TEXT NOT NULL, standing_id TEXT NOT NULL,
            account TEXT NOT NULL, repo TEXT NOT NULL, class TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL, expires_at_ms INTEGER,
            uses INTEGER NOT NULL DEFAULT 0, last_used_ms INTEGER,
            PRIMARY KEY(owner_id, installation_id, standing_id),
            UNIQUE(owner_id, installation_id, account, repo, class));",
    )
}

impl DurableKernelStateStore {
    /// `Grant` answers the id it stored; `Use` the id of the permission that covers the call.
    pub(crate) fn app_standing_command(
        &self,
        command: StandingCommand,
    ) -> Result<Option<String>, &'static str> {
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppStanding(Box::new(
                StandingRequest { command, response },
            )))
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        crate::durable_state::await_writer_response(receiver).map_err(|_| "STORAGE_UNAVAILABLE")?
    }

    pub(crate) fn app_standing(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<Vec<StandingPermission>, &'static str> {
        let connection = self
            .lock_connection("durable_state.app_standing")
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        let mut statement = connection
            .prepare(
                "SELECT standing_id, account, repo, class, created_at_ms, expires_at_ms, uses, last_used_ms
                 FROM app_standing_permissions WHERE owner_id=?1 AND installation_id=?2
                 ORDER BY repo, class, account",
            )
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        let rows = statement
            .query_map(params![owner, installation], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                ))
            })
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        rows.map(|row| {
            let (id, account, repo, class, created, expires, uses, last_used) =
                row.map_err(|_| "STORAGE_UNAVAILABLE")?;
            Ok(StandingPermission {
                id,
                account,
                repo,
                class: StandingClass::from_word(&class).ok_or("STORAGE_UNAVAILABLE")?,
                created_at_ms: created as u64,
                expires_at_ms: expires.map(|ms| ms as u64),
                uses: uses as u64,
                last_used_ms: last_used.map(|ms| ms as u64),
            })
        })
        .collect()
    }

    pub(crate) fn app_standing_count(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<u32, &'static str> {
        let connection = self
            .lock_connection("durable_state.app_standing")
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        connection
            .query_row(
                "SELECT count(*) FROM app_standing_permissions WHERE owner_id=?1 AND installation_id=?2",
                params![owner, installation],
                |row| row.get::<_, u32>(0),
            )
            .map_err(|_| "STORAGE_UNAVAILABLE")
    }
}

/// An account's standing permissions end with its binding, as a binding to another Vault
/// entry may be another person's account.
pub(super) fn forget_account(
    connection: &Connection,
    owner: &str,
    installation: &str,
    account: &str,
) -> rusqlite::Result<()> {
    connection.execute(
        "DELETE FROM app_standing_permissions WHERE owner_id=?1 AND installation_id=?2 AND account=?3",
        params![owner, installation, account],
    )?;
    Ok(())
}

/// Standing permissions end with the installation's active release.
pub(crate) fn forget_inactive(
    connection: &Connection,
    owner: &str,
    installation: &str,
) -> rusqlite::Result<()> {
    connection.execute(
        "DELETE FROM app_standing_permissions WHERE owner_id=?1 AND installation_id=?2
           AND NOT EXISTS (SELECT 1 FROM app_installations
                           WHERE installation_id=?2 AND active_json IS NOT NULL)",
        params![owner, installation],
    )?;
    Ok(())
}

pub(super) fn execute(connection: &mut Connection, request: StandingRequest) {
    let result = apply(connection, request.command);
    let _ = request.response.send(result);
}

fn apply(
    connection: &mut Connection,
    command: StandingCommand,
) -> Result<Option<String>, &'static str> {
    let storage = |_| "STORAGE_UNAVAILABLE";
    let transaction = connection.transaction().map_err(storage)?;
    let answer = match command {
        StandingCommand::Grant {
            owner,
            installation,
            permission,
        } => {
            let bound: i64 = transaction
                .query_row(
                    "SELECT count(*) FROM app_account_bindings
                     WHERE owner_id=?1 AND installation_id=?2 AND account=?3",
                    params![owner, installation, permission.account],
                    |row| row.get(0),
                )
                .map_err(storage)?;
            if bound == 0 {
                return Err("NOT_FOUND");
            }
            let existing: i64 = transaction
                .query_row(
                    "SELECT count(*) FROM app_standing_permissions
                     WHERE owner_id=?1 AND installation_id=?2
                       AND NOT (account=?3 AND repo=?4 AND class=?5)",
                    params![
                        owner,
                        installation,
                        permission.account,
                        permission.repo,
                        permission.class.word()
                    ],
                    |row| row.get(0),
                )
                .map_err(storage)?;
            if existing >= MAX_STANDING {
                return Err("LIMIT_EXCEEDED");
            }
            // Granting the same permission again renews it: new id, new expiry, use count reset.
            transaction
                .execute(
                    "INSERT OR REPLACE INTO app_standing_permissions
                       (owner_id, installation_id, standing_id, account, repo, class,
                        created_at_ms, expires_at_ms, uses, last_used_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, NULL)",
                    params![
                        owner,
                        installation,
                        permission.id,
                        permission.account,
                        permission.repo,
                        permission.class.word(),
                        permission.created_at_ms as i64,
                        permission.expires_at_ms.map(|ms| ms as i64),
                    ],
                )
                .map_err(storage)?;
            Some(permission.id)
        }
        StandingCommand::Revoke {
            owner,
            installation,
            id,
        } => {
            let removed = transaction
                .execute(
                    "DELETE FROM app_standing_permissions
                     WHERE owner_id=?1 AND installation_id=?2 AND standing_id=?3",
                    params![owner, installation, id],
                )
                .map_err(storage)?;
            if removed == 0 {
                return Err("NOT_FOUND");
            }
            None
        }
        StandingCommand::Use {
            owner,
            installation,
            repo,
            class,
            now_ms,
        } => {
            // Valid only while its account is still bound and the installation still active.
            let covering: Option<String> = transaction
                .query_row(
                    "SELECT s.standing_id FROM app_standing_permissions s
                     WHERE s.owner_id=?1 AND s.installation_id=?2 AND s.repo=?3 AND s.class=?4
                       AND (s.expires_at_ms IS NULL OR s.expires_at_ms > ?5)
                       AND EXISTS (SELECT 1 FROM app_account_bindings b
                                   WHERE b.owner_id=s.owner_id AND b.installation_id=s.installation_id
                                     AND b.account=s.account)
                       AND EXISTS (SELECT 1 FROM app_installations i
                                   WHERE i.installation_id=s.installation_id AND i.active_json IS NOT NULL)
                     ORDER BY s.created_at_ms DESC LIMIT 1",
                    params![owner, installation, repo, class.word(), now_ms as i64],
                    |row| row.get(0),
                )
                .optional()
                .map_err(storage)?;
            if let Some(id) = &covering {
                transaction
                    .execute(
                        "UPDATE app_standing_permissions SET uses=uses+1, last_used_ms=?4
                         WHERE owner_id=?1 AND installation_id=?2 AND standing_id=?3",
                        params![owner, installation, id, now_ms as i64],
                    )
                    .map_err(storage)?;
            }
            covering
        }
    };
    transaction.commit().map_err(storage)?;
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::durable_state::app_accounts::{AccountBinding, AccountCommand};
    use chariox_app_package::RouteSafety;

    fn store() -> (DurableKernelStateStore, std::path::PathBuf) {
        let root =
            std::env::temp_dir().join(format!("chariox-standing-{:016x}", rand::random::<u64>()));
        std::fs::create_dir(&root).unwrap();
        let store = DurableKernelStateStore::open_owned(root.join("kernel.sqlite")).unwrap();
        rusqlite::Connection::open(store.path())
            .unwrap()
            .execute_batch(
                "INSERT INTO app_installations(installation_id,app_id,owner_id,generation,allocated_generation,active_json)
                 VALUES('github','com.chariox.github','alice',1,1,'{}')",
            )
            .unwrap();
        (store, root)
    }

    fn bind(store: &DurableKernelStateStore, account: &str, entry: &str) {
        store
            .app_account_command(AccountCommand::Bind {
                owner: "alice".into(),
                installation: "github".into(),
                binding: AccountBinding {
                    account: account.into(),
                    credential: "github".into(),
                    vault_credential_id: entry.into(),
                    allow: RouteSafety::Destructive,
                    bound_at_ms: 1,
                },
            })
            .unwrap();
    }

    fn permission(
        id: &str,
        account: &str,
        repo: &str,
        class: StandingClass,
        expires: Option<u64>,
    ) -> StandingPermission {
        StandingPermission {
            id: id.into(),
            account: account.into(),
            repo: repo.into(),
            class,
            created_at_ms: 10,
            expires_at_ms: expires,
            uses: 0,
            last_used_ms: None,
        }
    }

    fn grant(
        store: &DurableKernelStateStore,
        permission: StandingPermission,
    ) -> Result<Option<String>, &'static str> {
        store.app_standing_command(StandingCommand::Grant {
            owner: "alice".into(),
            installation: "github".into(),
            permission,
        })
    }

    fn covers(
        store: &DurableKernelStateStore,
        repo: &str,
        class: StandingClass,
        now_ms: u64,
    ) -> Option<String> {
        store
            .app_standing_command(StandingCommand::Use {
                owner: "alice".into(),
                installation: "github".into(),
                repo: repo.into(),
                class,
                now_ms,
            })
            .unwrap()
    }

    #[test]
    fn a_standing_permission_covers_its_repository_and_class_only_until_it_expires_and_is_counted()
    {
        let (store, root) = store();
        assert_eq!(
            grant(
                &store,
                permission("s1", "work", "o/a", StandingClass::Merge, Some(100))
            ),
            Err("NOT_FOUND"),
            "an account that is not bound has nothing to stand on"
        );
        bind(&store, "work", "entry-1");
        grant(
            &store,
            permission("s1", "work", "o/a", StandingClass::Merge, Some(100)),
        )
        .unwrap();
        assert_eq!(
            covers(&store, "o/a", StandingClass::Merge, 50).as_deref(),
            Some("s1")
        );
        assert_eq!(
            covers(&store, "o/b", StandingClass::Merge, 50),
            None,
            "another repository still asks"
        );
        assert_eq!(
            covers(&store, "o/a", StandingClass::Close, 50),
            None,
            "another class still asks"
        );
        assert_eq!(
            covers(&store, "o/a", StandingClass::Merge, 100),
            None,
            "expired at its time"
        );
        let [listed] = store
            .app_standing("alice", "github")
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!((listed.uses, listed.last_used_ms), (1, Some(50)));
        // Never expiring, and renewing resets the count.
        grant(
            &store,
            permission("s2", "work", "o/a", StandingClass::Merge, None),
        )
        .unwrap();
        let [renewed] = store
            .app_standing("alice", "github")
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!((renewed.id.as_str(), renewed.uses), ("s2", 0));
        assert_eq!(
            covers(&store, "o/a", StandingClass::Merge, u64::MAX / 2).as_deref(),
            Some("s2")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn revoking_or_replacing_the_account_binding_or_ending_the_installation_ends_them_at_once() {
        let (store, root) = store();
        bind(&store, "work", "entry-1");
        bind(&store, "home", "entry-2");
        grant(
            &store,
            permission("s1", "work", "o/a", StandingClass::Merge, None),
        )
        .unwrap();
        grant(
            &store,
            permission("s2", "home", "o/b", StandingClass::Review, None),
        )
        .unwrap();
        // The same account on the same Vault entry keeps them; a different entry does not.
        bind(&store, "work", "entry-1");
        assert!(covers(&store, "o/a", StandingClass::Merge, 1).is_some());
        bind(&store, "work", "entry-9");
        assert_eq!(covers(&store, "o/a", StandingClass::Merge, 1), None);
        // Revoking the binding removes the other account's.
        store
            .app_account_command(AccountCommand::Revoke {
                owner: "alice".into(),
                installation: "github".into(),
                account: "home".into(),
            })
            .unwrap();
        assert_eq!(covers(&store, "o/b", StandingClass::Review, 1), None);
        assert!(store.app_standing("alice", "github").unwrap().is_empty());
        // An installation without an active release covers nothing, and its rows go.
        grant(
            &store,
            permission("s3", "work", "o/a", StandingClass::Close, None),
        )
        .unwrap();
        let connection = rusqlite::Connection::open(store.path()).unwrap();
        connection
            .execute_batch("UPDATE app_installations SET active_json=NULL")
            .unwrap();
        assert_eq!(covers(&store, "o/a", StandingClass::Close, 1), None);
        forget_inactive(&connection, "alice", "github").unwrap();
        assert_eq!(store.app_standing_count("alice", "github").unwrap(), 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn standing_permissions_are_per_owner_bounded_and_revocable_by_id() {
        let (store, root) = store();
        bind(&store, "work", "entry-1");
        for index in 0..MAX_STANDING {
            grant(
                &store,
                permission(
                    &format!("s{index}"),
                    "work",
                    &format!("o/r{index}"),
                    StandingClass::Merge,
                    None,
                ),
            )
            .unwrap();
        }
        assert_eq!(
            grant(
                &store,
                permission(
                    "one-too-many",
                    "work",
                    "o/extra",
                    StandingClass::Merge,
                    None
                )
            ),
            Err("LIMIT_EXCEEDED")
        );
        assert_eq!(store.app_standing("bob", "github").unwrap().len(), 0);
        let revoke = |id: &str| {
            store.app_standing_command(StandingCommand::Revoke {
                owner: "alice".into(),
                installation: "github".into(),
                id: id.into(),
            })
        };
        revoke("s0").unwrap();
        assert_eq!(revoke("s0"), Err("NOT_FOUND"));
        assert_eq!(covers(&store, "o/r0", StandingClass::Merge, 1), None);
        let _ = std::fs::remove_dir_all(root);
    }
}
