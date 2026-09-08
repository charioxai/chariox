//! Structured App state on the kernel's existing SQLite connection.
//! Admission, worker authentication and durable writer ownership belong to the
//! kernel. These transactions do not lock or roll back ordinary node:fs writes.

mod changes;
mod store;

pub use changes::{StateChanges, StateCheck, StateWrite};
use rusqlite::{Connection, Transaction, TransactionBehavior};
use serde_json::Value;

pub const MAX_KEYS: usize = 4096;
pub const MAX_KEY_BYTES: usize = 128;
pub const MAX_VALUE_BYTES: usize = 256 * 1024;
pub const MAX_CHANGE_BYTES: usize = 512 * 1024;
pub const MAX_STATE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_CHANGES: usize = 64;
pub const MAX_CHECKS: usize = 128;
pub const MAX_REVISION: u64 = 9_007_199_254_740_991;

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("app_state_database")]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Installation(#[from] crate::installation::InstallationError),
    #[error("app_state_invalid")]
    Invalid,
    #[error("app_state_conflict")]
    Conflict,
    #[error("app_state_schema_mismatch")]
    SchemaMismatch,
    #[error("app_state_limit")]
    Limit,
    #[error("app_state_corrupt")]
    Corrupt,
}
pub type Result<T> = std::result::Result<T, StateError>;

/// Supplied from the admitted worker's kernel-owned identity, never decoded
/// from an App request. Construction validates syntax, not authority.
#[derive(Debug, Clone, Copy)]
pub struct StateScope<'a> {
    owner: &'a str,
    installation: &'a str,
    generation: u64,
}
impl<'a> StateScope<'a> {
    pub fn new(owner: &'a str, installation: &'a str, generation: u64) -> Result<Self> {
        for identity in [owner, installation] {
            if identity.trim().is_empty()
                || identity.len() > 128
                || identity.chars().any(char::is_control)
            {
                return Err(StateError::Invalid);
            }
        }
        if generation == 0 || generation > i64::MAX as u64 {
            return Err(StateError::Invalid);
        }
        Ok(Self {
            owner,
            installation,
            generation,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StateRecord {
    pub value: Value,
    pub version: u64,
}

pub struct ManagedStateStore<'a> {
    connection: &'a mut Connection,
}
impl<'a> ManagedStateStore<'a> {
    pub fn new(connection: &'a mut Connection) -> Self {
        Self { connection }
    }

    pub fn initialize(&mut self) -> Result<()> {
        store::initialize(self.connection)
    }

    /// Admission and the read share one SQLite snapshot. Update quiescence or
    /// a generation switch cannot fall between them.
    pub fn get(&mut self, scope: StateScope<'_>, key: &str) -> Result<Option<StateRecord>> {
        let transaction = self.connection.transaction()?;
        let result = Self::read_in(&transaction, scope, key)?;
        transaction.commit()?;
        Ok(result)
    }

    /// Compose an authenticated publisher/worker check and state read in the
    /// same existing kernel transaction. This does not end that transaction.
    pub fn read_in(
        transaction: &Transaction<'_>,
        scope: StateScope<'_>,
        key: &str,
    ) -> Result<Option<StateRecord>> {
        changes::key(key)?;
        store::active(transaction, scope)?;
        store::read(transaction, scope.installation, key)
    }

    /// Standalone structured mutation. Returning success means SQLite commit
    /// succeeded, with durability pragmas provided by the kernel owner.
    pub fn transaction(&mut self, scope: StateScope<'_>, changes: &StateChanges) -> Result<u64> {
        let mut transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let revision = Self::apply_in(&mut transaction, scope, changes)?;
        transaction.commit()?;
        Ok(revision)
    }

    /// Compose state with the installation outbox in one existing writer
    /// transaction. This does NOT commit the outer transaction. A savepoint
    /// prevents partially applied state on error. The caller must roll back its
    /// encompassing operation on error and acknowledge only after commit.
    /// Event ownership/schema/automation admission is the outbox's responsibility.
    pub fn apply_in(
        transaction: &mut Transaction<'_>,
        scope: StateScope<'_>,
        changes: &StateChanges,
    ) -> Result<u64> {
        let savepoint = transaction.savepoint()?;
        let revision = store::apply(&savepoint, scope, changes)?;
        savepoint.commit()?;
        Ok(revision)
    }
}
