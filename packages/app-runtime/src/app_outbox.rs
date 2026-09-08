//! Transactional App occurrences on the existing kernel writer connection.
//! This is receipt persistence, not workflow dispatch or automation permission.

mod admission;
mod store;

use crate::app_catalog::{AppCatalog, CatalogError};
pub use admission::{EventCatalog, VerifiedAutomation};
use chariox_app_package::VerifiedPackage;
use rusqlite::{Connection, Transaction};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, sync::Arc};
pub use store::{Receipt, ReceiptState};

pub const MAX_AUTOMATIONS: usize = 256;
pub const MAX_RECEIPTS: usize = 4096;
pub const MAX_PENDING: usize = 1024;
pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024;
pub const MAX_RETAINED_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_BATCH: usize = 16;
pub const MAX_BATCH_BYTES: usize = 512 * 1024;
pub const MAX_ATTEMPTS: u32 = 8;
pub const PENDING_LIFETIME_MS: u64 = 7 * 24 * 60 * 60 * 1000;
pub const MAX_OCCURRENCE_AGE_MS: u64 = 30 * 24 * 60 * 60 * 1000;
pub const MAX_FUTURE_SKEW_MS: u64 = 5 * 60 * 1000;
pub const MAX_SAFE_TIMESTAMP: u64 = 9_007_199_254_740_991;

#[derive(Debug, thiserror::Error)]
pub enum OutboxError {
    #[error("app_outbox_database")]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error("app_outbox_invalid")]
    Invalid,
    #[error("app_outbox_not_found")]
    NotFound,
    #[error("app_outbox_conflict")]
    Conflict,
    #[error("app_outbox_inactive_automation")]
    Inactive,
    #[error("app_outbox_schema")]
    Schema,
    #[error("app_outbox_limit")]
    Limit,
    #[error("app_outbox_corrupt")]
    Corrupt,
    #[error("app_outbox_occurrence_too_old")]
    TooOld,
}
pub type Result<T> = std::result::Result<T, OutboxError>;

/// The current SDK shape. Identity and generation come from the retained
/// worker/catalog and automation, not additional App-controlled authority fields.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Occurrence {
    pub automation_id: String,
    pub occurrence_id: String,
    pub event_version: u32,
    pub occurred_at_ms: u64,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_schedule_revision"
    )]
    pub schedule_revision: Option<String>,
    pub payload: Value,
}

fn deserialize_schedule_revision<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error> {
    // Omission is handled by `default`; explicit null is not an omitted field
    // in the SDK contract and must never be normalized into one.
    <String as serde::Deserialize>::deserialize(deserializer).map(Some)
}

pub struct AppOutbox;
impl AppOutbox {
    pub fn initialize(connection: &Connection) -> Result<()> {
        store::initialize(connection)
    }

    /// Accepts one automation's batch in a savepoint inside the existing writer
    /// transaction. StateChanges::apply_in may precede/follow this operation in
    /// that same transaction. Nothing is acknowledged before its outer commit.
    pub fn apply_in(
        transaction: &mut Transaction<'_>,
        automation: &VerifiedAutomation,
        occurrences: &[Occurrence],
        kernel_now_ms: u64,
    ) -> Result<Vec<Receipt>> {
        time(kernel_now_ms)?;
        if occurrences.is_empty() || occurrences.len() > MAX_BATCH {
            return Err(OutboxError::Limit);
        }
        let mut identities = BTreeSet::new();
        let mut total = 0usize;
        let prepared = occurrences
            .iter()
            .map(|occurrence| {
                identifier(&occurrence.occurrence_id)?;
                if occurrence.occurred_at_ms > MAX_SAFE_TIMESTAMP
                    || occurrence.schedule_revision.is_some() != automation.binding.scheduled
                {
                    return Err(OutboxError::Invalid);
                }
                if let Some(revision) = &occurrence.schedule_revision {
                    identifier(revision)?;
                }
                if occurrence.automation_id != automation.id()
                    || occurrence.event_version != automation.event_version()
                    || !identities.insert((
                        occurrence.occurrence_id.as_str(),
                        occurrence.schedule_revision.as_deref(),
                    ))
                {
                    return Err(OutboxError::Invalid);
                }
                let payload = automation.encode_payload(&occurrence.payload)?;
                total = total.checked_add(payload.len()).ok_or(OutboxError::Limit)?;
                if total > MAX_BATCH_BYTES {
                    return Err(OutboxError::Limit);
                }
                Ok((occurrence, payload))
            })
            .collect::<Result<Vec<_>>>()?;
        automation.require_current(transaction)?;
        let savepoint = transaction.savepoint()?;
        let receipts = prepared
            .iter()
            .map(|(occurrence, payload)| {
                store::accept(&savepoint, automation, occurrence, payload, kernel_now_ms)
            })
            .collect::<Result<Vec<_>>>()?;
        savepoint.commit()?;
        Ok(receipts)
    }

    /// Status is installation scoped and remains observable for paused or broken
    /// automation bindings. The admitted installation/catalog must still be current.
    pub fn status_in(
        transaction: &Transaction<'_>,
        catalog: &EventCatalog,
        trusted_owner: &str,
        receipt_id: &str,
    ) -> Result<Receipt> {
        catalog.require_current(transaction, trusted_owner)?;
        identifier(receipt_id)?;
        store::receipt(
            transaction,
            trusted_owner,
            catalog.installation_id(),
            receipt_id,
        )
    }

    /// Bounded candidate discovery. Consumers must claim/recheck each candidate
    /// in their committing transaction; this read grants no delivery authority.
    pub fn pending_in(
        transaction: &Transaction<'_>,
        catalog: &EventCatalog,
        trusted_owner: &str,
        kernel_now_ms: u64,
        limit: usize,
    ) -> Result<Vec<Receipt>> {
        catalog.require_current(transaction, trusted_owner)?;
        time(kernel_now_ms)?;
        if !(1..=64).contains(&limit) {
            return Err(OutboxError::Limit);
        }
        store::pending(
            transaction,
            trusted_owner,
            catalog.installation_id(),
            kernel_now_ms,
            limit,
        )
    }
}

fn identifier(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || value
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '\u{feff}')
    {
        return Err(OutboxError::Invalid);
    }
    Ok(())
}
fn time(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| OutboxError::Invalid)
}
fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
