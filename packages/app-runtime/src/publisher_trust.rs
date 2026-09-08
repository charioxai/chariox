//! Owner-scoped local publisher enrollment, using the kernel's SQLite writer.
//!
//! This is a trusted internal persistence API, not human authorization. The
//! kernel must authenticate the owner and obtain the exact enrollment/revocation
//! decision before calling it. Package bytes, a publisher file, an agent, or an
//! App-supplied decision string cannot enroll themselves. Library trust is out
//! of scope. No private signing key is stored here.

mod store;

use chariox_app_package::TrustedPublisher;
use rusqlite::{Connection, TransactionBehavior};

pub const MAX_TRUST_OWNERS: usize = 128;
pub const MAX_KEYS_PER_OWNER: usize = 64;
pub const MAX_TRUST_KEYS: usize = 4096;
pub const MAX_DECISIONS_PER_OWNER: usize = 512;
pub const MAX_TRUST_DECISIONS: usize = 32768;

#[derive(Debug, thiserror::Error)]
pub enum PublisherTrustError {
    #[error("app_publisher_trust_database: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("app_publisher_trust_invalid")]
    Invalid,
    #[error("app_publisher_trust_not_found")]
    NotFound,
    #[error("app_publisher_trust_revoked")]
    Revoked,
    #[error("app_publisher_trust_conflict")]
    Conflict,
    #[error("app_publisher_trust_limit")]
    Limit,
    #[error("app_publisher_trust_corrupt")]
    Corrupt,
    #[error("app_publisher_trust_stopped")]
    Stopped,
}
type Result<T> = std::result::Result<T, PublisherTrustError>;

/// Opaque references to an authenticated kernel decision. No wire decoder or
/// authorization assertion is supplied by this persistence component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustDecision {
    pub decision_id: String,
    pub authority_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublisherTrustEntry {
    pub owner_id: String,
    pub publisher_id: String,
    pub key_id: String,
    pub public_key: [u8; 32],
    pub revision: u64,
    pub enrolled: bool,
    pub decision: TrustDecision,
    pub updated_at_ms: u64,
}

/// Historical result of this exact decision. A successful replay does not
/// imply current trust: a later revocation may already have superseded it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustDecisionReceipt {
    pub decision: TrustDecision,
    pub publisher_id: String,
    pub key_id: String,
    pub revision: u64,
    pub enrolled: bool,
    pub applied_at_ms: u64,
}

/// Immutable verification input. Its revision must be checked again on the
/// kernel writer in the same transaction that commits an installation stage.
#[derive(Debug, Clone)]
pub struct TrustedPublisherSnapshot {
    owner_id: String,
    publisher: TrustedPublisher,
    revision: u64,
}
impl TrustedPublisherSnapshot {
    pub fn publisher(&self) -> &TrustedPublisher {
        &self.publisher
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Pass the same writer transaction that commits the installation mutation.
    /// Autocommit reads cannot establish that commit fence and are not accepted.
    ///
    /// ```compile_fail
    /// use chariox_app_runtime::publisher_trust::TrustedPublisherSnapshot;
    /// fn outside_transaction(snapshot: &TrustedPublisherSnapshot, connection: &rusqlite::Connection) {
    ///     snapshot.require_current(connection, "owner").unwrap();
    /// }
    /// ```
    pub fn require_current(
        &self,
        transaction: &rusqlite::Transaction<'_>,
        trusted_owner: &str,
    ) -> Result<()> {
        valid_owner(trusted_owner)?;
        if self.owner_id != trusted_owner {
            return Err(PublisherTrustError::NotFound);
        }
        let current = store::get(
            transaction,
            trusted_owner,
            &self.publisher.publisher_id,
            &self.publisher.key_id,
        )?
        .ok_or(PublisherTrustError::NotFound)?;
        if !current.enrolled {
            return Err(PublisherTrustError::Revoked);
        }
        if current.revision != self.revision
            || current.public_key != self.publisher.public_key.to_bytes()
        {
            return Err(PublisherTrustError::Conflict);
        }
        Ok(())
    }
}

pub struct PublisherTrustRegistry<'a> {
    connection: &'a mut Connection,
}
impl<'a> PublisherTrustRegistry<'a> {
    /// Durability pragmas, writer admission and connection ownership remain with
    /// the kernel. This API does not open or own another database.
    pub fn new(connection: &'a mut Connection) -> Self {
        Self { connection }
    }
    pub fn initialize(&mut self) -> Result<()> {
        store::initialize(self.connection)
    }

    /// Zero expected revision creates a new immutable key binding. Re-enrolling
    /// a revoked key requires its current revision and a fresh explicit decision.
    pub fn enroll(
        &mut self,
        trusted_owner: &str,
        publisher: &TrustedPublisher,
        expected_revision: u64,
        decision: &TrustDecision,
        now_ms: u64,
    ) -> Result<TrustDecisionReceipt> {
        self.enroll_guarded(
            trusted_owner,
            publisher,
            expected_revision,
            decision,
            now_ms,
            || Ok(()),
        )
    }

    /// The trusted kernel checks its original cancellation/deadline before
    /// waiting for SQLite and again inside the transaction before commit.
    /// This callback validates continued admission; it cannot grant consent.
    #[allow(clippy::too_many_arguments)]
    pub fn enroll_guarded(
        &mut self,
        trusted_owner: &str,
        publisher: &TrustedPublisher,
        expected_revision: u64,
        decision: &TrustDecision,
        now_ms: u64,
        check: impl FnMut() -> Result<()>,
    ) -> Result<TrustDecisionReceipt> {
        if publisher.public_key.is_weak() {
            return Err(PublisherTrustError::Invalid);
        }
        self.decide(
            trusted_owner,
            &publisher.publisher_id,
            &publisher.key_id,
            Some(publisher.public_key.to_bytes()),
            expected_revision,
            decision,
            now_ms,
            check,
        )
    }

    pub fn revoke(
        &mut self,
        trusted_owner: &str,
        publisher_id: &str,
        key_id: &str,
        expected_revision: u64,
        decision: &TrustDecision,
        now_ms: u64,
    ) -> Result<TrustDecisionReceipt> {
        self.revoke_guarded(
            trusted_owner,
            publisher_id,
            key_id,
            expected_revision,
            decision,
            now_ms,
            || Ok(()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn revoke_guarded(
        &mut self,
        trusted_owner: &str,
        publisher_id: &str,
        key_id: &str,
        expected_revision: u64,
        decision: &TrustDecision,
        now_ms: u64,
        check: impl FnMut() -> Result<()>,
    ) -> Result<TrustDecisionReceipt> {
        self.decide(
            trusted_owner,
            publisher_id,
            key_id,
            None,
            expected_revision,
            decision,
            now_ms,
            check,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn decide(
        &mut self,
        owner: &str,
        publisher_id: &str,
        key_id: &str,
        enroll_key: Option<[u8; 32]>,
        expected: u64,
        decision: &TrustDecision,
        now_ms: u64,
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<TrustDecisionReceipt> {
        valid_owner(owner)?;
        valid_identifier(publisher_id)?;
        valid_identifier(key_id)?;
        valid_text(&decision.decision_id, 128)?;
        valid_text(&decision.authority_ref, 512)?;
        checked_integer(expected)?;
        checked_integer(now_ms)?;
        check()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check()?;
        let receipt = store::decide(
            &transaction,
            owner,
            publisher_id,
            key_id,
            enroll_key,
            expected,
            decision,
            now_ms,
        )?;
        check()?;
        transaction.commit()?;
        Ok(receipt)
    }

    pub fn get(
        &self,
        trusted_owner: &str,
        publisher_id: &str,
        key_id: &str,
    ) -> Result<PublisherTrustEntry> {
        valid_owner(trusted_owner)?;
        valid_identifier(publisher_id)?;
        valid_identifier(key_id)?;
        store::get(self.connection, trusted_owner, publisher_id, key_id)?
            .ok_or(PublisherTrustError::NotFound)
    }

    /// Bounded list includes revocation tombstones, which are never silently
    /// deleted or reused for a different key. Decision receipts are retained too.
    pub fn list(&self, trusted_owner: &str) -> Result<Vec<PublisherTrustEntry>> {
        valid_owner(trusted_owner)?;
        store::list(self.connection, trusted_owner)
    }

    pub fn trusted_publisher(
        &self,
        trusted_owner: &str,
        publisher_id: &str,
        key_id: &str,
    ) -> Result<TrustedPublisherSnapshot> {
        let entry = self.get(trusted_owner, publisher_id, key_id)?;
        if !entry.enrolled {
            return Err(PublisherTrustError::Revoked);
        }
        let public_key = ed25519_dalek::VerifyingKey::from_bytes(&entry.public_key)
            .map_err(|_| PublisherTrustError::Corrupt)?;
        Ok(TrustedPublisherSnapshot {
            owner_id: entry.owner_id,
            publisher: TrustedPublisher {
                publisher_id: entry.publisher_id,
                key_id: entry.key_id,
                public_key,
            },
            revision: entry.revision,
        })
    }
}

fn valid_owner(value: &str) -> Result<()> {
    valid_text(value, 128)
}
fn valid_text(value: &str, bound: usize) -> Result<()> {
    if value.len() > bound || value.trim().is_empty() || value.chars().any(char::is_control) {
        Err(PublisherTrustError::Invalid)
    } else {
        Ok(())
    }
}
fn valid_identifier(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        Err(PublisherTrustError::Invalid)
    } else {
        Ok(())
    }
}
fn checked_integer(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| PublisherTrustError::Invalid)
}
