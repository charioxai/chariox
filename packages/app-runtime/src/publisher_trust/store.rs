use super::*;
use rusqlite::{params, OptionalExtension, Row};

pub(super) fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_publisher_keys (
        owner_id TEXT NOT NULL,
        publisher_id TEXT NOT NULL,
        key_id TEXT NOT NULL,
        public_key BLOB NOT NULL CHECK(length(public_key) = 32),
        revision INTEGER NOT NULL CHECK(revision > 0),
        enrolled INTEGER NOT NULL CHECK(enrolled IN (0, 1)),
        decision_id TEXT NOT NULL,
        authority_ref TEXT NOT NULL,
        updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= 0),
        PRIMARY KEY(owner_id, publisher_id, key_id)
     );
     CREATE TABLE IF NOT EXISTS app_publisher_decisions (
        owner_id TEXT NOT NULL,
        decision_id TEXT NOT NULL,
        publisher_id TEXT NOT NULL,
        key_id TEXT NOT NULL,
        public_key BLOB NOT NULL CHECK(length(public_key) = 32),
        expected_revision INTEGER NOT NULL CHECK(expected_revision >= 0),
        revision INTEGER NOT NULL CHECK(revision > 0),
        enrolled INTEGER NOT NULL CHECK(enrolled IN (0, 1)),
        authority_ref TEXT NOT NULL,
        applied_at_ms INTEGER NOT NULL CHECK(applied_at_ms >= 0),
        PRIMARY KEY(owner_id, decision_id)
     );",
    )?;
    Ok(())
}

type RawEntry = (
    String,
    String,
    String,
    Vec<u8>,
    i64,
    i64,
    String,
    String,
    i64,
);
fn raw_entry(row: &Row<'_>) -> rusqlite::Result<RawEntry> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
    ))
}
fn entry(raw: RawEntry) -> Result<PublisherTrustEntry> {
    let (
        owner_id,
        publisher_id,
        key_id,
        bytes,
        revision,
        enrolled,
        decision_id,
        authority_ref,
        updated_at_ms,
    ) = raw;
    let public_key: [u8; 32] = bytes.try_into().map_err(|_| PublisherTrustError::Corrupt)?;
    let public = ed25519_dalek::VerifyingKey::from_bytes(&public_key)
        .map_err(|_| PublisherTrustError::Corrupt)?;
    if public.is_weak()
        || revision <= 0
        || updated_at_ms < 0
        || ![0, 1].contains(&enrolled)
        || valid_owner(&owner_id).is_err()
        || valid_identifier(&publisher_id).is_err()
        || valid_identifier(&key_id).is_err()
        || valid_text(&decision_id, 128).is_err()
        || valid_text(&authority_ref, 512).is_err()
    {
        return Err(PublisherTrustError::Corrupt);
    }
    Ok(PublisherTrustEntry {
        owner_id,
        publisher_id,
        key_id,
        public_key,
        revision: revision as u64,
        enrolled: enrolled == 1,
        decision: TrustDecision {
            decision_id,
            authority_ref,
        },
        updated_at_ms: updated_at_ms as u64,
    })
}

pub(super) fn get(
    connection: &Connection,
    owner: &str,
    publisher: &str,
    key: &str,
) -> Result<Option<PublisherTrustEntry>> {
    connection.query_row("SELECT owner_id,publisher_id,key_id,public_key,revision,enrolled,decision_id,authority_ref,updated_at_ms
        FROM app_publisher_keys WHERE owner_id=?1 AND publisher_id=?2 AND key_id=?3",
        params![owner, publisher, key], raw_entry).optional()?.map(entry).transpose()
}
pub(super) fn list(connection: &Connection, owner: &str) -> Result<Vec<PublisherTrustEntry>> {
    let mut statement = connection.prepare("SELECT owner_id,publisher_id,key_id,public_key,revision,enrolled,decision_id,authority_ref,updated_at_ms
        FROM app_publisher_keys WHERE owner_id=?1 ORDER BY publisher_id,key_id LIMIT ?2")?;
    let rows = statement.query_map(params![owner, (MAX_KEYS_PER_OWNER + 1) as i64], raw_entry)?;
    let entries = rows.map(|raw| entry(raw?)).collect::<Result<Vec<_>>>()?;
    if entries.len() > MAX_KEYS_PER_OWNER {
        return Err(PublisherTrustError::Corrupt);
    }
    Ok(entries)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn decide(
    connection: &Connection,
    owner: &str,
    publisher: &str,
    key: &str,
    enroll_key: Option<[u8; 32]>,
    expected: u64,
    decision: &TrustDecision,
    now_ms: u64,
) -> Result<TrustDecisionReceipt> {
    // Receipts precede the CAS check. An identical old enrollment retry returns
    // its historical result without mutating a subsequently revoked binding.
    let prior = connection.query_row("SELECT owner_id,publisher_id,key_id,public_key,revision,enrolled,decision_id,authority_ref,applied_at_ms,expected_revision
        FROM app_publisher_decisions WHERE owner_id=?1 AND decision_id=?2", params![owner, &decision.decision_id],
        |row| Ok((raw_entry(row)?, row.get::<_, i64>(9)?))).optional()?;
    if let Some((raw, previous_expected)) = prior {
        let prior = entry(raw)?;
        if previous_expected < 0 {
            return Err(PublisherTrustError::Corrupt);
        }
        if prior.publisher_id != publisher
            || prior.key_id != key
            || prior.enrolled != enroll_key.is_some()
            || prior.decision != *decision
            || previous_expected as u64 != expected
            || enroll_key.is_some_and(|bytes| bytes != prior.public_key)
        {
            return Err(PublisherTrustError::Conflict);
        }
        return Ok(receipt(&prior));
    }
    let current = get(connection, owner, publisher, key)?;
    if current.is_none() && enroll_key.is_none() {
        return Err(PublisherTrustError::NotFound);
    }
    let old_revision = current.as_ref().map_or(0, |entry| entry.revision);
    if expected != old_revision {
        return Err(PublisherTrustError::Conflict);
    }
    let public_key = match (&current, enroll_key) {
        (Some(current), Some(bytes)) if current.public_key != bytes => {
            return Err(PublisherTrustError::Conflict)
        }
        (_, Some(bytes)) => bytes,
        (Some(current), None) => current.public_key,
        (None, None) => return Err(PublisherTrustError::NotFound),
    };
    let revision = old_revision
        .checked_add(1)
        .filter(|value| *value <= i64::MAX as u64)
        .ok_or(PublisherTrustError::Limit)?;
    admit(connection, owner, current.as_ref(), enroll_key.is_some())?;
    let updated = PublisherTrustEntry {
        owner_id: owner.into(),
        publisher_id: publisher.into(),
        key_id: key.into(),
        public_key,
        revision,
        enrolled: enroll_key.is_some(),
        decision: decision.clone(),
        updated_at_ms: now_ms,
    };
    // The caller holds BEGIN IMMEDIATE. Neither failed receipt insertion nor a
    // failed key mutation may leave the other half committed independently.
    connection.execute("INSERT INTO app_publisher_decisions
        (owner_id,decision_id,publisher_id,key_id,public_key,expected_revision,revision,enrolled,authority_ref,applied_at_ms)
        VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![owner, &decision.decision_id, publisher, key, &public_key[..], checked_integer(expected)?, checked_integer(revision)?,
            i64::from(updated.enrolled), &decision.authority_ref, checked_integer(now_ms)?])?;
    if current.is_some() {
        let changed = connection.execute("UPDATE app_publisher_keys SET revision=?4,enrolled=?5,decision_id=?6,authority_ref=?7,updated_at_ms=?8
            WHERE owner_id=?1 AND publisher_id=?2 AND key_id=?3 AND revision=?9",
            params![owner, publisher, key, checked_integer(revision)?, i64::from(updated.enrolled), &decision.decision_id,
                &decision.authority_ref, checked_integer(now_ms)?, checked_integer(expected)?])?;
        if changed != 1 {
            return Err(PublisherTrustError::Conflict);
        }
    } else {
        connection.execute("INSERT INTO app_publisher_keys
            (owner_id,publisher_id,key_id,public_key,revision,enrolled,decision_id,authority_ref,updated_at_ms)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![owner, publisher, key, &public_key[..], checked_integer(revision)?, i64::from(updated.enrolled),
                &decision.decision_id, &decision.authority_ref, checked_integer(now_ms)?])?;
    }
    Ok(receipt(&updated))
}

fn receipt(entry: &PublisherTrustEntry) -> TrustDecisionReceipt {
    TrustDecisionReceipt {
        decision: entry.decision.clone(),
        publisher_id: entry.publisher_id.clone(),
        key_id: entry.key_id.clone(),
        revision: entry.revision,
        enrolled: entry.enrolled,
        applied_at_ms: entry.updated_at_ms,
    }
}

fn admit(
    connection: &Connection,
    owner: &str,
    current: Option<&PublisherTrustEntry>,
    enroll: bool,
) -> Result<()> {
    let counts: (i64, i64, i64, i64, i64, i64, i64) = connection.query_row(
        "SELECT
        (SELECT count(*) FROM app_publisher_keys),
        (SELECT count(*) FROM app_publisher_keys WHERE owner_id=?1),
        (SELECT count(DISTINCT owner_id) FROM app_publisher_keys),
        (SELECT count(*) FROM app_publisher_keys WHERE enrolled=1),
        (SELECT count(*) FROM app_publisher_keys WHERE owner_id=?1 AND enrolled=1),
        (SELECT count(*) FROM app_publisher_decisions),
        (SELECT count(*) FROM app_publisher_decisions WHERE owner_id=?1)",
        params![owner],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        },
    )?;
    let (keys, own_keys, owners, active, own_active, decisions, own_decisions) = counts;
    if current.is_none()
        && (keys >= MAX_TRUST_KEYS as i64
            || own_keys >= MAX_KEYS_PER_OWNER as i64
            || (own_keys == 0 && owners >= MAX_TRUST_OWNERS as i64))
    {
        return Err(PublisherTrustError::Limit);
    }
    let active_delta = i64::from(enroll) - i64::from(current.is_some_and(|entry| entry.enrolled));
    // Keep one future revocation receipt reserved for every active key. Hitting
    // an enrollment limit can never prevent revocation of an existing grant.
    if decisions + 1 + active + active_delta > MAX_TRUST_DECISIONS as i64
        || own_decisions + 1 + own_active + active_delta > MAX_DECISIONS_PER_OWNER as i64
    {
        return Err(PublisherTrustError::Limit);
    }
    Ok(())
}
