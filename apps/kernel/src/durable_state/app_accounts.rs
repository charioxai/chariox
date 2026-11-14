//! The owner's accounts an installation may call a credentialed destination
//! with. A binding names a Vault entry and how much authority the owner gives
//! the App through it; the App's signed route table says which calls exist.
//! The value stays in the Vault: this table holds only the entry's id.
use super::{DurableKernelStateStore, DurableWriterRequest};
use chariox_app_package::RouteSafety;
use rusqlite::{params, Connection};
use std::sync::mpsc;

/// Accounts per installation.
const MAX_ACCOUNTS: i64 = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AccountBinding {
    pub(crate) account: String,
    /// The manifest's credential label this account answers to.
    pub(crate) credential: String,
    pub(crate) vault_credential_id: String,
    pub(crate) allow: RouteSafety,
    pub(crate) bound_at_ms: u64,
}

pub(crate) enum AccountCommand {
    Bind {
        owner: String,
        installation: String,
        binding: AccountBinding,
    },
    Revoke {
        owner: String,
        installation: String,
        account: String,
    },
}

pub(super) struct AccountRequest {
    command: AccountCommand,
    response: mpsc::Sender<Result<(), &'static str>>,
}
impl std::fmt::Debug for AccountRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AccountRequest(..)")
    }
}

fn allow_word(allow: RouteSafety) -> &'static str {
    match allow {
        RouteSafety::Read => "read",
        RouteSafety::Write => "write",
        RouteSafety::Destructive => "destructive",
    }
}
fn allow_of(word: &str) -> Option<RouteSafety> {
    match word {
        "read" => Some(RouteSafety::Read),
        "write" => Some(RouteSafety::Write),
        "destructive" => Some(RouteSafety::Destructive),
        _ => None,
    }
}

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_account_bindings (
            owner_id TEXT NOT NULL, installation_id TEXT NOT NULL, account TEXT NOT NULL,
            credential TEXT NOT NULL, vault_credential_id TEXT NOT NULL,
            allow TEXT NOT NULL, bound_at_ms INTEGER NOT NULL,
            PRIMARY KEY(owner_id, installation_id, account));",
    )
}

impl DurableKernelStateStore {
    pub(crate) fn app_account_command(&self, command: AccountCommand) -> Result<(), &'static str> {
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppAccount(Box::new(AccountRequest {
                command,
                response,
            })))
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        crate::durable_state::await_writer_response(receiver).map_err(|_| "STORAGE_UNAVAILABLE")?
    }

    pub(crate) fn app_accounts(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<Vec<AccountBinding>, &'static str> {
        let connection = self
            .lock_connection("durable_state.app_accounts")
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        let mut statement = connection
            .prepare(
                "SELECT account, credential, vault_credential_id, allow, bound_at_ms
                 FROM app_account_bindings WHERE owner_id=?1 AND installation_id=?2
                 ORDER BY account",
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
                ))
            })
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        rows.map(|row| {
            let (account, credential, vault_credential_id, allow, bound_at_ms) =
                row.map_err(|_| "STORAGE_UNAVAILABLE")?;
            Ok(AccountBinding {
                account,
                credential,
                vault_credential_id,
                allow: allow_of(&allow).ok_or("STORAGE_UNAVAILABLE")?,
                bound_at_ms: bound_at_ms as u64,
            })
        })
        .collect()
    }

    pub(crate) fn app_account(
        &self,
        owner: &str,
        installation: &str,
        account: &str,
    ) -> Result<Option<AccountBinding>, &'static str> {
        Ok(self
            .app_accounts(owner, installation)?
            .into_iter()
            .find(|binding| binding.account == account))
    }
}

/// Bindings end with the installation's active release, like connection grants.
pub(crate) fn forget_inactive(
    connection: &Connection,
    owner: &str,
    installation: &str,
) -> rusqlite::Result<()> {
    connection.execute(
        "DELETE FROM app_account_bindings WHERE owner_id=?1 AND installation_id=?2
           AND NOT EXISTS (SELECT 1 FROM app_installations
                           WHERE installation_id=?2 AND active_json IS NOT NULL)",
        params![owner, installation],
    )?;
    Ok(())
}

pub(super) fn execute(connection: &mut Connection, request: AccountRequest) {
    let result = apply(connection, request.command);
    let _ = request.response.send(result);
}

fn apply(connection: &mut Connection, command: AccountCommand) -> Result<(), &'static str> {
    let storage = |_| "STORAGE_UNAVAILABLE";
    let transaction = connection.transaction().map_err(storage)?;
    match command {
        AccountCommand::Bind {
            owner,
            installation,
            binding,
        } => {
            let existing: i64 = transaction
                .query_row(
                    "SELECT count(*) FROM app_account_bindings
                     WHERE owner_id=?1 AND installation_id=?2 AND account<>?3",
                    params![owner, installation, binding.account],
                    |row| row.get(0),
                )
                .map_err(storage)?;
            if existing >= MAX_ACCOUNTS {
                return Err("LIMIT_EXCEEDED");
            }
            // Binding an account again replaces its entry and level.
            transaction
                .execute(
                    "INSERT OR REPLACE INTO app_account_bindings
                       (owner_id, installation_id, account, credential,
                        vault_credential_id, allow, bound_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        owner,
                        installation,
                        binding.account,
                        binding.credential,
                        binding.vault_credential_id,
                        allow_word(binding.allow),
                        binding.bound_at_ms as i64
                    ],
                )
                .map_err(storage)?;
        }
        AccountCommand::Revoke {
            owner,
            installation,
            account,
        } => {
            let removed = transaction
                .execute(
                    "DELETE FROM app_account_bindings
                     WHERE owner_id=?1 AND installation_id=?2 AND account=?3",
                    params![owner, installation, account],
                )
                .map_err(storage)?;
            if removed == 0 {
                return Err("NOT_FOUND");
            }
        }
    }
    transaction.commit().map_err(storage)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(account: &str, allow: RouteSafety) -> AccountBinding {
        AccountBinding {
            account: account.into(),
            credential: "github".into(),
            vault_credential_id: format!("entry-{account}"),
            allow,
            bound_at_ms: 5,
        }
    }

    #[test]
    fn bindings_are_per_installation_replaceable_bounded_and_revocable() {
        let root = std::env::temp_dir().join(format!(
            "chariox-account-bindings-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&root).unwrap();
        let store = DurableKernelStateStore::open_owned(root.join("kernel.sqlite")).unwrap();
        let bind = |installation: &str, account: &str, allow| {
            store.app_account_command(AccountCommand::Bind {
                owner: "alice".into(),
                installation: installation.into(),
                binding: binding(account, allow),
            })
        };
        bind("github", "work", RouteSafety::Read).unwrap();
        bind("github", "work", RouteSafety::Write).unwrap();
        assert_eq!(
            store.app_accounts("alice", "github").unwrap(),
            vec![binding("work", RouteSafety::Write)]
        );
        assert!(store.app_accounts("alice", "todo").unwrap().is_empty());
        assert!(store.app_accounts("bob", "github").unwrap().is_empty());
        for index in 1..MAX_ACCOUNTS {
            bind("github", &format!("extra-{index}"), RouteSafety::Read).unwrap();
        }
        assert_eq!(
            bind("github", "one-too-many", RouteSafety::Read),
            Err("LIMIT_EXCEEDED")
        );
        // Changing an existing account is not a new one.
        bind("github", "work", RouteSafety::Destructive).unwrap();
        let revoke = |account: &str| {
            store.app_account_command(AccountCommand::Revoke {
                owner: "alice".into(),
                installation: "github".into(),
                account: account.into(),
            })
        };
        revoke("work").unwrap();
        assert_eq!(revoke("work"), Err("NOT_FOUND"));
        // Bindings end with the active release.
        let connection = rusqlite::Connection::open(store.path()).unwrap();
        connection
            .execute_batch(
                "INSERT INTO app_installations(installation_id,app_id,owner_id,generation,allocated_generation,active_json)
                 VALUES('github','com.chariox.github','alice',1,1,'{}')",
            )
            .unwrap();
        forget_inactive(&connection, "alice", "github").unwrap();
        assert!(!store.app_accounts("alice", "github").unwrap().is_empty());
        connection
            .execute_batch("UPDATE app_installations SET active_json=NULL")
            .unwrap();
        forget_inactive(&connection, "alice", "github").unwrap();
        assert!(store.app_accounts("alice", "github").unwrap().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }
}
