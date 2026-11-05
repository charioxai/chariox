//! Kernel-local dev trust is separate from publisher enrollment and its listing.
use super::*;
use chariox_app_package::v2::DEV_PUBLISHER_ID;
use rusqlite::{params, OptionalExtension};

pub(super) fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_dev_identities (
        owner_id TEXT PRIMARY KEY, public_key BLOB NOT NULL CHECK(length(public_key)=32));",
    )?;
    Ok(())
}

pub(super) fn snapshot(
    connection: &Connection,
    owner: &str,
    key_id: &str,
) -> Result<TrustedPublisherSnapshot> {
    valid_owner(owner)?;
    if key_id != "kernel-dev" {
        return Err(PublisherTrustError::NotFound);
    }
    let bytes: Vec<u8> = connection
        .query_row(
            "SELECT public_key FROM app_dev_identities WHERE owner_id=?1",
            [owner],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(PublisherTrustError::NotFound)?;
    let bytes: [u8; 32] = bytes.try_into().map_err(|_| PublisherTrustError::Corrupt)?;
    let public_key = ed25519_dalek::VerifyingKey::from_bytes(&bytes)
        .map_err(|_| PublisherTrustError::Corrupt)?;
    Ok(TrustedPublisherSnapshot {
        owner_id: owner.into(),
        publisher: TrustedPublisher {
            publisher_id: DEV_PUBLISHER_ID.into(),
            key_id: key_id.into(),
            public_key,
        },
        revision: 1,
    })
}

impl PublisherTrustRegistry<'_> {
    /// Called only with a public key derived from the kernel's Vault key. This
    /// cannot replace an existing key, enroll a publisher or approve capabilities.
    pub fn bind_dev_identity(
        &mut self,
        owner: &str,
        public_key: [u8; 32],
    ) -> Result<TrustDecisionReceipt> {
        valid_owner(owner)?;
        let key = ed25519_dalek::VerifyingKey::from_bytes(&public_key)
            .map_err(|_| PublisherTrustError::Invalid)?;
        if key.is_weak() {
            return Err(PublisherTrustError::Invalid);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("INSERT INTO app_dev_identities(owner_id, public_key) VALUES(?1,?2) ON CONFLICT(owner_id) DO NOTHING", params![owner, public_key.as_slice()])?;
        let current = snapshot(&tx, owner, "kernel-dev")?;
        if current.publisher.public_key != key {
            return Err(PublisherTrustError::Conflict);
        }
        tx.commit()?;
        Ok(TrustDecisionReceipt {
            decision: TrustDecision {
                decision_id: "kernel-dev".into(),
                authority_ref: "kernel_dev_identity".into(),
            },
            publisher_id: DEV_PUBLISHER_ID.into(),
            key_id: "kernel-dev".into(),
            revision: 1,
            enrolled: false,
            applied_at_ms: 0,
        })
    }
}
