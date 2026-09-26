//! Durable App inbox on the kernel writer connection: occurrences from an
//! external source, routed to one installation's declared incoming event.
//! Accepting records the occurrence before the source is acknowledged; the
//! kernel then delivers it to the App's handler at least once, with bounded
//! retries and a visible failed (poison) or expired outcome. Payloads are
//! dropped once an occurrence is settled. Routing grants the App nothing else.

use chariox_app_package::{EventDirection, Limits, VerifiedPackage};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const MAX_ROUTES: usize = 64;
pub const MAX_PENDING: usize = 1024;
pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024;
pub const MAX_ATTEMPTS: u32 = 8;
pub const PENDING_LIFETIME_MS: u64 = 7 * 24 * 60 * 60 * 1000;
/// A settled occurrence is kept this long after it was accepted, so a source
/// replaying it within the window is answered as a duplicate; later it is
/// pruned, and the inbox does not grow with a route's lifetime volume.
pub const DEDUPE_WINDOW_MS: u64 = 2 * PENDING_LIFETIME_MS;
/// Settled rows pruned per accepted occurrence (amortized, on the writer).
const PRUNE_BATCH: usize = 64;
const MAX_BACKOFF_MS: u64 = 60_000;

#[derive(Debug, thiserror::Error)]
pub enum InboxError {
    #[error("app_inbox_database")]
    Database(#[from] rusqlite::Error),
    #[error("app_inbox_invalid")]
    Invalid,
    #[error("app_inbox_not_found")]
    NotFound,
    #[error("app_inbox_schema")]
    Schema,
    #[error("app_inbox_conflict")]
    Conflict,
    #[error("app_inbox_limit")]
    Limit,
    #[error("app_inbox_corrupt")]
    Corrupt,
}
pub type Result<T> = std::result::Result<T, InboxError>;

/// The publisher-signed incoming event schemas of one verified release.
pub struct IncomingCatalog {
    events: BTreeMap<String, (u32, jsonschema::JSONSchema)>,
}
impl IncomingCatalog {
    pub fn compile(package: &VerifiedPackage<'_>) -> Result<Self> {
        let mut events = BTreeMap::new();
        for declaration in &package.declarations().events {
            if declaration.direction == EventDirection::Outgoing {
                continue;
            }
            let validator = chariox_app_package::compile_schema(
                &declaration.payload_schema,
                true,
                &Limits::default(),
            )
            .map_err(|_| InboxError::Schema)?;
            events.insert(
                declaration.name.clone(),
                (declaration.schema_version, validator),
            );
        }
        Ok(Self { events })
    }

    /// The declared version of an incoming event, if the release has one.
    pub fn version(&self, name: &str) -> Option<u32> {
        self.events.get(name).map(|(version, _)| *version)
    }

    pub fn validate(&self, name: &str, payload: &Value) -> Result<()> {
        let (_, validator) = self.events.get(name).ok_or(InboxError::Schema)?;
        if validator.is_valid(payload) {
            Ok(())
        } else {
            Err(InboxError::Schema)
        }
    }
}

/// Routes one external event type to an installation's incoming event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxRoute {
    pub route_id: String,
    pub owner_id: String,
    pub installation_id: String,
    pub event_name: String,
    pub source_event_type: String,
    pub source_event_version: u32,
    pub active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboxState {
    Accepted,
    Retryable,
    Delivered,
    Failed,
    Expired,
}
impl InboxState {
    fn parse(value: &str) -> Result<Self> {
        Ok(match value {
            "accepted" => Self::Accepted,
            "retryable" => Self::Retryable,
            "delivered" => Self::Delivered,
            "failed" => Self::Failed,
            "expired" => Self::Expired,
            _ => return Err(InboxError::Corrupt),
        })
    }
}

/// A pending occurrence ready for delivery to its installation.
#[derive(Debug, Clone, PartialEq)]
pub struct InboxItem {
    pub sequence: i64,
    pub owner_id: String,
    pub installation_id: String,
    pub route_id: String,
    pub event_name: String,
    pub occurrence_id: String,
    pub payload: Value,
    pub attempts: u32,
    /// The generation whose incoming schema admitted the payload.
    pub accepted_generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accepted {
    New(i64),
    /// The same occurrence with the same content was accepted before.
    Duplicate(i64),
}

pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_inbox_routes (
            owner_id TEXT NOT NULL, installation_id TEXT NOT NULL, route_id TEXT NOT NULL,
            event_name TEXT NOT NULL, source_event_type TEXT NOT NULL,
            source_event_version INTEGER NOT NULL CHECK(source_event_version>0),
            active INTEGER NOT NULL CHECK(active IN (0,1)), created_at_ms INTEGER NOT NULL,
            PRIMARY KEY(owner_id,installation_id,route_id)
         );
         CREATE TABLE IF NOT EXISTS app_inbox (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
            owner_id TEXT NOT NULL, installation_id TEXT NOT NULL, route_id TEXT NOT NULL,
            event_name TEXT NOT NULL, occurrence_id TEXT NOT NULL, content_digest TEXT NOT NULL,
            payload_json TEXT, accepted_generation INTEGER NOT NULL, delivered_generation INTEGER,
            accepted_at_ms INTEGER NOT NULL, expires_at_ms INTEGER NOT NULL,
            state TEXT NOT NULL CHECK(state IN ('accepted','retryable','delivered','failed','expired')),
            attempts INTEGER NOT NULL CHECK(attempts>=0 AND attempts<=8),
            next_attempt_at_ms INTEGER NOT NULL,
            UNIQUE(owner_id,installation_id,route_id,occurrence_id)
         );
         CREATE INDEX IF NOT EXISTS app_inbox_due ON app_inbox(state,next_attempt_at_ms,sequence);
         CREATE INDEX IF NOT EXISTS app_inbox_installation_state ON app_inbox(owner_id,installation_id,state,accepted_at_ms);",
    )?;
    Ok(())
}

fn text(value: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(InboxError::Invalid);
    }
    Ok(())
}

pub fn create_route_in(tx: &Connection, route: &InboxRoute, now_ms: u64) -> Result<()> {
    for value in [
        &route.route_id,
        &route.owner_id,
        &route.installation_id,
        &route.event_name,
        &route.source_event_type,
    ] {
        text(value)?;
    }
    if route.source_event_version == 0 {
        return Err(InboxError::Invalid);
    }
    let count: i64 = tx.query_row(
        "SELECT count(*) FROM app_inbox_routes WHERE owner_id=?1 AND installation_id=?2",
        params![route.owner_id, route.installation_id],
        |row| row.get(0),
    )?;
    if count as usize >= MAX_ROUTES {
        return Err(InboxError::Limit);
    }
    tx.execute(
        "INSERT INTO app_inbox_routes(route_id,owner_id,installation_id,event_name,
            source_event_type,source_event_version,active,created_at_ms)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            route.route_id,
            route.owner_id,
            route.installation_id,
            route.event_name,
            route.source_event_type,
            route.source_event_version,
            route.active,
            now_ms as i64
        ],
    )
    .map_err(|error| match error {
        rusqlite::Error::SqliteFailure(failure, _)
            if failure.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            InboxError::Conflict
        }
        error => error.into(),
    })?;
    Ok(())
}

/// Removing a route also drops its occurrences, delivered or not: a route
/// created again under the same name starts empty.
pub fn remove_route_in(
    tx: &Connection,
    owner_id: &str,
    installation_id: &str,
    route_id: &str,
) -> Result<()> {
    let removed = tx.execute(
        "DELETE FROM app_inbox_routes WHERE owner_id=?1 AND installation_id=?2 AND route_id=?3",
        params![owner_id, installation_id, route_id],
    )?;
    if removed == 0 {
        return Err(InboxError::NotFound);
    }
    tx.execute(
        "DELETE FROM app_inbox WHERE owner_id=?1 AND installation_id=?2 AND route_id=?3",
        params![owner_id, installation_id, route_id],
    )?;
    Ok(())
}

fn route_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InboxRoute> {
    Ok(InboxRoute {
        route_id: row.get(0)?,
        owner_id: row.get(1)?,
        installation_id: row.get(2)?,
        event_name: row.get(3)?,
        source_event_type: row.get(4)?,
        source_event_version: row.get(5)?,
        active: row.get(6)?,
    })
}
const ROUTE_COLUMNS: &str = "route_id,owner_id,installation_id,event_name,source_event_type,\
    source_event_version,active";

pub fn route(
    connection: &Connection,
    owner_id: &str,
    installation_id: &str,
    route_id: &str,
) -> Result<Option<InboxRoute>> {
    Ok(connection
        .query_row(
            &format!(
                "SELECT {ROUTE_COLUMNS} FROM app_inbox_routes
                 WHERE owner_id=?1 AND installation_id=?2 AND route_id=?3"
            ),
            params![owner_id, installation_id, route_id],
            route_row,
        )
        .optional()?)
}

pub fn routes(
    connection: &Connection,
    owner_id: &str,
    installation_id: &str,
) -> Result<Vec<InboxRoute>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {ROUTE_COLUMNS} FROM app_inbox_routes
         WHERE owner_id=?1 AND installation_id=?2 ORDER BY route_id"
    ))?;
    let routes = statement
        .query_map(params![owner_id, installation_id], route_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(routes)
}

/// Records one source occurrence for its route, in the caller's transaction.
/// The payload must already be validated against the App's incoming schema.
pub fn accept_in(
    tx: &Connection,
    route: &InboxRoute,
    occurrence_id: &str,
    payload: &Value,
    generation: u64,
    now_ms: u64,
) -> Result<Accepted> {
    text(occurrence_id)?;
    if !route.active {
        return Err(InboxError::NotFound);
    }
    let payload_json = serde_json::to_string(payload).map_err(|_| InboxError::Invalid)?;
    if payload_json.len() > MAX_PAYLOAD_BYTES {
        return Err(InboxError::Limit);
    }
    // Settled occurrences past the dedupe window go, a bounded batch at a time.
    tx.execute(
        "DELETE FROM app_inbox WHERE sequence IN (
           SELECT sequence FROM app_inbox WHERE owner_id=?1 AND installation_id=?2
           AND state IN ('delivered','failed','expired') AND accepted_at_ms<=?3 LIMIT ?4)",
        params![
            route.owner_id,
            route.installation_id,
            now_ms.saturating_sub(DEDUPE_WINDOW_MS) as i64,
            PRUNE_BATCH as i64
        ],
    )?;
    let canonical = serde_json_canonicalizer::to_vec(payload).map_err(|_| InboxError::Invalid)?;
    let content_digest = format!("sha256:{:x}", Sha256::digest(&canonical));
    let existing: Option<(i64, String)> = tx
        .query_row(
            "SELECT sequence,content_digest FROM app_inbox WHERE owner_id=?1 AND installation_id=?2
             AND route_id=?3 AND occurrence_id=?4",
            params![
                route.owner_id,
                route.installation_id,
                route.route_id,
                occurrence_id
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((sequence, digest)) = existing {
        return if digest == content_digest {
            Ok(Accepted::Duplicate(sequence))
        } else {
            Err(InboxError::Conflict)
        };
    }
    let pending: i64 = tx.query_row(
        "SELECT count(*) FROM app_inbox WHERE owner_id=?1 AND installation_id=?2
         AND state IN ('accepted','retryable')",
        params![route.owner_id, route.installation_id],
        |row| row.get(0),
    )?;
    if pending as usize >= MAX_PENDING {
        return Err(InboxError::Limit);
    }
    tx.execute(
        "INSERT INTO app_inbox(owner_id,installation_id,route_id,event_name,occurrence_id,
            content_digest,payload_json,accepted_generation,accepted_at_ms,expires_at_ms,
            state,attempts,next_attempt_at_ms)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'accepted',0,?9)",
        params![
            route.owner_id,
            route.installation_id,
            route.route_id,
            route.event_name,
            occurrence_id,
            content_digest,
            payload_json,
            generation as i64,
            now_ms as i64,
            now_ms.saturating_add(PENDING_LIFETIME_MS) as i64
        ],
    )?;
    Ok(Accepted::New(tx.last_insert_rowid()))
}

/// Pending occurrences due for delivery, oldest first. Expired ones are
/// settled first so they are never delivered.
pub fn due(tx: &Connection, now_ms: u64, limit: usize) -> Result<Vec<InboxItem>> {
    tx.execute(
        "UPDATE app_inbox SET state='expired',payload_json=NULL
         WHERE state IN ('accepted','retryable') AND expires_at_ms<=?1",
        [now_ms as i64],
    )?;
    // Every pass also prunes a bounded batch of settled occurrences past the
    // dedupe window, so an idle or uninstalled installation's rows go too.
    if let Some(cutoff) = now_ms.checked_sub(DEDUPE_WINDOW_MS) {
        tx.execute(
            "DELETE FROM app_inbox WHERE sequence IN (
               SELECT sequence FROM app_inbox WHERE state IN ('delivered','failed','expired')
               AND accepted_at_ms<=?1 LIMIT ?2)",
            params![cutoff as i64, PRUNE_BATCH as i64],
        )?;
    }
    let mut statement = tx.prepare(
        "SELECT sequence,owner_id,installation_id,route_id,event_name,occurrence_id,
            payload_json,attempts,accepted_generation
         FROM app_inbox WHERE state IN ('accepted','retryable') AND next_attempt_at_ms<=?1
         ORDER BY sequence LIMIT ?2",
    )?;
    let rows = statement.query_map(params![now_ms as i64, limit as i64], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, u32>(7)?,
            row.get::<_, i64>(8)? as u64,
        ))
    })?;
    rows.map(|row| {
        let (
            sequence,
            owner_id,
            installation_id,
            route_id,
            event_name,
            occurrence_id,
            payload,
            attempts,
            accepted_generation,
        ) = row?;
        let payload = serde_json::from_str(&payload.ok_or(InboxError::Corrupt)?)
            .map_err(|_| InboxError::Corrupt)?;
        Ok(InboxItem {
            sequence,
            owner_id,
            installation_id,
            route_id,
            event_name,
            occurrence_id,
            payload,
            attempts,
            accepted_generation,
        })
    })
    .collect()
}

fn pending_update(tx: &Connection, sql: &str, args: impl rusqlite::Params) -> Result<()> {
    if tx.execute(sql, args)? == 0 {
        return Err(InboxError::NotFound);
    }
    Ok(())
}

/// The App's handler returned for this occurrence.
pub fn delivered_in(tx: &Connection, sequence: i64, generation: u64) -> Result<()> {
    pending_update(
        tx,
        "UPDATE app_inbox SET state='delivered',payload_json=NULL,delivered_generation=?2
         WHERE sequence=?1 AND state IN ('accepted','retryable')",
        params![sequence, generation as i64],
    )
}

/// A failed delivery: back off, and settle as failed (poison) after the last
/// attempt.
pub fn failed_attempt_in(tx: &Connection, sequence: i64, now_ms: u64) -> Result<InboxState> {
    let attempts: u32 = tx
        .query_row(
            "SELECT attempts FROM app_inbox WHERE sequence=?1 AND state IN ('accepted','retryable')",
            [sequence],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(InboxError::NotFound)?;
    let attempts = attempts + 1;
    if attempts >= MAX_ATTEMPTS {
        pending_update(
            tx,
            "UPDATE app_inbox SET state='failed',payload_json=NULL,attempts=?2 WHERE sequence=?1",
            params![sequence, attempts],
        )?;
        return Ok(InboxState::Failed);
    }
    let backoff = (1_000u64 << attempts.min(6)).min(MAX_BACKOFF_MS);
    pending_update(
        tx,
        "UPDATE app_inbox SET state='retryable',attempts=?2,next_attempt_at_ms=?3 WHERE sequence=?1",
        params![sequence, attempts, now_ms.saturating_add(backoff) as i64],
    )?;
    Ok(InboxState::Retryable)
}

/// An occurrence that can no longer be delivered as accepted (an update
/// removed its event or changed its schema) fails at once, visibly.
pub fn undeliverable_in(tx: &Connection, sequence: i64) -> Result<()> {
    pending_update(
        tx,
        "UPDATE app_inbox SET state='failed',payload_json=NULL
         WHERE sequence=?1 AND state IN ('accepted','retryable')",
        params![sequence],
    )
}

/// Wait without spending an attempt (the App is starting or updating).
pub fn postpone_in(tx: &Connection, sequence: i64, until_ms: u64) -> Result<()> {
    pending_update(
        tx,
        "UPDATE app_inbox SET next_attempt_at_ms=?2
         WHERE sequence=?1 AND state IN ('accepted','retryable')",
        params![sequence, until_ms as i64],
    )
}

pub fn state(connection: &Connection, sequence: i64) -> Result<InboxState> {
    let value: String = connection
        .query_row(
            "SELECT state FROM app_inbox WHERE sequence=?1",
            [sequence],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(InboxError::NotFound)?;
    InboxState::parse(&value)
}

/// Occurrence outcomes of one route, so poison and expiry stay visible.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InboxCounts {
    pub pending: u64,
    pub delivered: u64,
    pub failed: u64,
    pub expired: u64,
}

pub fn counts(connection: &Connection, route: &InboxRoute) -> Result<InboxCounts> {
    let mut statement = connection.prepare(
        "SELECT state,count(*) FROM app_inbox WHERE owner_id=?1 AND installation_id=?2
         AND route_id=?3 GROUP BY state",
    )?;
    let mut counts = InboxCounts::default();
    for row in statement.query_map(
        params![route.owner_id, route.installation_id, route.route_id],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64)),
    )? {
        let (state, count) = row?;
        match InboxState::parse(&state)? {
            InboxState::Accepted | InboxState::Retryable => counts.pending += count,
            InboxState::Delivered => counts.delivered += count,
            InboxState::Failed => counts.failed += count,
            InboxState::Expired => counts.expired += count,
        }
    }
    Ok(counts)
}
