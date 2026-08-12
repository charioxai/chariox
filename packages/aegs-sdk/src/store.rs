use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

pub use arroba_event_protocol::AegsSubscriptionClaim as SubscriptionClaim;

#[derive(Debug, Clone, PartialEq)]
pub struct AuthorizationRecord {
    pub state_digest: String,
    pub connection_id: String,
    pub provider: String,
    pub return_url: Option<String>,
    pub expires_at_ms: u64,
    pub completed: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct CreateAuthorizationRequest<'a> {
    pub state_digest: &'a str,
    pub connection_id: &'a str,
    pub owner_id: &'a str,
    pub provider: &'a str,
    pub return_url: Option<&'a str>,
    pub expires_at_ms: u64,
    pub now_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConnectionRecord {
    pub connection_id: String,
    pub owner_id: String,
    pub provider: String,
    pub status: String,
    pub encrypted_credential: Option<Vec<u8>>,
    pub metadata: Value,
    pub expires_at_ms: Option<u64>,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderHookRecord {
    pub connection_id: String,
    pub connection_scope: String,
    pub provider_hook_id: String,
    pub configuration_digest: String,
    pub expires_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AegsStoreMetrics {
    pub active_subscriptions: u64,
    pub subscriptions: u64,
    pub connections: u64,
    pub provider_hooks: u64,
}

#[derive(Clone)]
pub struct AegsStore {
    connection: Arc<Mutex<Connection>>,
}

impl AegsStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, rusqlite::Error> {
        let connection = Connection::open(path)?;
        connection.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = FULL;
            CREATE TABLE IF NOT EXISTS subscriptions (
                binding_id TEXT PRIMARY KEY NOT NULL,
                owner_id TEXT NOT NULL,
                generator_id TEXT NOT NULL,
                connection_id TEXT NOT NULL,
                connection_scope TEXT NOT NULL,
                event_interest_key TEXT NOT NULL,
                event_type TEXT NOT NULL,
                event_type_version INTEGER NOT NULL,
                filter_json TEXT NOT NULL,
                revision INTEGER NOT NULL,
                active INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS subscriptions_matching
                ON subscriptions(generator_id, event_type, connection_scope, active);
            CREATE TABLE IF NOT EXISTS connections (
                connection_id TEXT PRIMARY KEY NOT NULL,
                owner_id TEXT NOT NULL DEFAULT 'legacy',
                provider TEXT NOT NULL,
                status TEXT NOT NULL,
                encrypted_credential BLOB,
                metadata_json TEXT NOT NULL,
                expires_at_ms INTEGER,
                updated_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS authorizations (
                state_digest TEXT PRIMARY KEY NOT NULL,
                connection_id TEXT NOT NULL UNIQUE,
                provider TEXT NOT NULL,
                return_url TEXT,
                expires_at_ms INTEGER NOT NULL,
                completed INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY(connection_id) REFERENCES connections(connection_id)
            );
            CREATE INDEX IF NOT EXISTS authorizations_expiry
                ON authorizations(expires_at_ms);
            CREATE TABLE IF NOT EXISTS provider_hooks (
                connection_id TEXT NOT NULL,
                connection_scope TEXT NOT NULL,
                provider_hook_id TEXT NOT NULL,
                configuration_digest TEXT NOT NULL,
                expires_at_ms INTEGER,
                updated_at_ms INTEGER NOT NULL,
                PRIMARY KEY(connection_id, connection_scope),
                FOREIGN KEY(connection_id) REFERENCES connections(connection_id)
            );
            ",
        )?;
        migrate_subscription_owner(&connection)?;
        migrate_connection_owner(&connection)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub fn upsert(&self, claim: &SubscriptionClaim) -> Result<bool, String> {
        claim.validate()?;
        let filter_json =
            serde_json::to_string(&claim.filter).map_err(|error| error.to_string())?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| "AEGS subscription store lock was poisoned".to_string())?;
        let current_revision = connection
            .query_row(
                "SELECT revision FROM subscriptions WHERE binding_id = ?1",
                params![claim.binding_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if current_revision.is_some_and(|revision| revision > claim.revision as i64) {
            return Ok(false);
        }
        connection
            .execute(
                "
                INSERT INTO subscriptions (
                    binding_id, generator_id, connection_id, connection_scope,
                    event_interest_key, event_type, event_type_version, filter_json,
                    revision, active
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                ON CONFLICT(binding_id) DO UPDATE SET
                    generator_id = excluded.generator_id,
                    connection_id = excluded.connection_id,
                    connection_scope = excluded.connection_scope,
                    event_interest_key = excluded.event_interest_key,
                    event_type = excluded.event_type,
                    event_type_version = excluded.event_type_version,
                    filter_json = excluded.filter_json,
                    revision = excluded.revision,
                    active = excluded.active
                ",
                params![
                    claim.binding_id,
                    claim.generator_id,
                    claim.connection_id,
                    claim.connection_scope,
                    claim.event_interest_key,
                    claim.event_type,
                    claim.event_type_version,
                    filter_json,
                    claim.revision as i64,
                    claim.active,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(true)
    }

    pub fn reconcile(
        &self,
        owner_id: &str,
        generator_id: &str,
        claims: &[SubscriptionClaim],
    ) -> Result<Vec<String>, String> {
        if owner_id.trim().is_empty() {
            return Err("subscription reconciliation owner_id is required".to_string());
        }
        for claim in claims {
            if claim.generator_id != generator_id {
                return Err(format!(
                    "binding {} belongs to generator {}, not {}",
                    claim.binding_id, claim.generator_id, generator_id
                ));
            }
            claim.validate()?;
        }
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "AEGS subscription store lock was poisoned".to_string())?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let claimed_binding_ids = claims
            .iter()
            .map(|claim| claim.binding_id.as_str())
            .collect::<BTreeSet<_>>();
        let owned_binding_ids = {
            let mut statement = transaction
                .prepare(
                    "SELECT binding_id FROM subscriptions
                     WHERE generator_id = ?1 AND owner_id = ?2",
                )
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map(params![generator_id, owner_id], |row| {
                    row.get::<_, String>(0)
                })
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            rows
        };
        for binding_id in owned_binding_ids {
            if !claimed_binding_ids.contains(binding_id.as_str()) {
                transaction
                    .execute(
                        "UPDATE subscriptions SET active = 0
                         WHERE binding_id = ?1 AND owner_id = ?2",
                        params![binding_id, owner_id],
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
        let mut accepted = Vec::with_capacity(claims.len());
        for claim in claims {
            let filter_json =
                serde_json::to_string(&claim.filter).map_err(|error| error.to_string())?;
            let changed = transaction
                .execute(
                    "
                    INSERT INTO subscriptions (
                        binding_id, owner_id, generator_id, connection_id, connection_scope,
                        event_interest_key, event_type, event_type_version, filter_json,
                        revision, active
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                    ON CONFLICT(binding_id) DO UPDATE SET
                        owner_id = excluded.owner_id,
                        generator_id = excluded.generator_id,
                        connection_id = excluded.connection_id,
                        connection_scope = excluded.connection_scope,
                        event_interest_key = excluded.event_interest_key,
                        event_type = excluded.event_type,
                        event_type_version = excluded.event_type_version,
                        filter_json = excluded.filter_json,
                        revision = excluded.revision,
                        active = excluded.active
                    WHERE
                        (excluded.owner_id = subscriptions.owner_id
                            AND excluded.revision >= subscriptions.revision)
                        OR excluded.revision > subscriptions.revision
                    ",
                    params![
                        claim.binding_id,
                        owner_id,
                        claim.generator_id,
                        claim.connection_id,
                        claim.connection_scope,
                        claim.event_interest_key,
                        claim.event_type,
                        claim.event_type_version,
                        filter_json,
                        claim.revision as i64,
                        claim.active,
                    ],
                )
                .map_err(|error| error.to_string())?;
            if changed > 0 {
                accepted.push(claim.binding_id.clone());
            }
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(accepted)
    }

    pub fn matching(
        &self,
        generator_id: &str,
        event_type: &str,
        connection_scope: &str,
    ) -> Result<Vec<SubscriptionClaim>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "AEGS subscription store lock was poisoned".to_string())?;
        let mut statement = connection
            .prepare(
                "
                SELECT binding_id, generator_id, connection_id, connection_scope,
                       event_interest_key, event_type, event_type_version, filter_json,
                       revision, active
                FROM subscriptions
                WHERE generator_id = ?1 AND event_type = ?2
                  AND connection_scope = ?3 AND active = 1
                ORDER BY binding_id
                ",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![generator_id, event_type, connection_scope], |row| {
                let filter_json: String = row.get(7)?;
                Ok(SubscriptionClaim {
                    binding_id: row.get(0)?,
                    generator_id: row.get(1)?,
                    connection_id: row.get(2)?,
                    connection_scope: row.get(3)?,
                    event_interest_key: row.get(4)?,
                    event_type: row.get(5)?,
                    event_type_version: row.get(6)?,
                    filter: serde_json::from_str(&filter_json).unwrap_or(Value::Null),
                    revision: row.get::<_, i64>(8)? as u64,
                    active: row.get(9)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn all(&self, generator_id: &str) -> Result<Vec<SubscriptionClaim>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "AEGS subscription store lock was poisoned".to_string())?;
        let mut statement = connection
            .prepare(
                "
                SELECT binding_id, generator_id, connection_id, connection_scope,
                       event_interest_key, event_type, event_type_version, filter_json,
                       revision, active
                FROM subscriptions WHERE generator_id = ?1 ORDER BY binding_id
                ",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![generator_id], |row| {
                let filter_json: String = row.get(7)?;
                Ok(SubscriptionClaim {
                    binding_id: row.get(0)?,
                    generator_id: row.get(1)?,
                    connection_id: row.get(2)?,
                    connection_scope: row.get(3)?,
                    event_interest_key: row.get(4)?,
                    event_type: row.get(5)?,
                    event_type_version: row.get(6)?,
                    filter: serde_json::from_str(&filter_json).unwrap_or(Value::Null),
                    revision: row.get::<_, i64>(8)? as u64,
                    active: row.get(9)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn metrics(&self) -> Result<AegsStoreMetrics, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "AEGS subscription store lock was poisoned".to_string())?;
        let count = |table: &str, predicate: Option<&str>| -> Result<u64, String> {
            let query = predicate
                .map(|predicate| format!("SELECT COUNT(*) FROM {table} WHERE {predicate}"))
                .unwrap_or_else(|| format!("SELECT COUNT(*) FROM {table}"));
            connection
                .query_row(&query, [], |row| row.get::<_, i64>(0))
                .map(|value| value.max(0) as u64)
                .map_err(|error| error.to_string())
        };
        Ok(AegsStoreMetrics {
            active_subscriptions: count("subscriptions", Some("active = 1"))?,
            subscriptions: count("subscriptions", None)?,
            connections: count("connections", None)?,
            provider_hooks: count("provider_hooks", None)?,
        })
    }

    pub fn create_authorization(
        &self,
        request: CreateAuthorizationRequest<'_>,
    ) -> Result<(), String> {
        let CreateAuthorizationRequest {
            state_digest,
            connection_id,
            owner_id,
            provider,
            return_url,
            expires_at_ms,
            now_ms,
        } = request;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "AEGS store lock was poisoned".to_string())?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "DELETE FROM authorizations WHERE expires_at_ms < ?1",
                params![now_ms as i64],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "
                INSERT INTO connections (
                    connection_id, owner_id, provider, status, encrypted_credential,
                    metadata_json, expires_at_ms, updated_at_ms
                ) VALUES (?1, ?2, ?3, 'pending', NULL, '{}', NULL, ?4)
                ",
                params![connection_id, owner_id, provider, now_ms as i64],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "
                INSERT INTO authorizations (
                    state_digest, connection_id, provider, return_url,
                    expires_at_ms, completed
                ) VALUES (?1, ?2, ?3, ?4, ?5, 0)
                ",
                params![
                    state_digest,
                    connection_id,
                    provider,
                    return_url,
                    expires_at_ms as i64
                ],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())
    }

    pub fn create_reauthorization(
        &self,
        request: CreateAuthorizationRequest<'_>,
    ) -> Result<(), String> {
        let CreateAuthorizationRequest {
            state_digest,
            connection_id,
            owner_id,
            provider,
            return_url,
            expires_at_ms,
            now_ms,
        } = request;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "AEGS store lock was poisoned".to_string())?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let owned = transaction
            .query_row(
                "SELECT COUNT(*) FROM connections
                 WHERE connection_id = ?1 AND owner_id = ?2 AND provider = ?3",
                params![connection_id, owner_id, provider],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| error.to_string())?;
        if owned != 1 {
            return Err("the owned connection was not found".to_string());
        }
        transaction
            .execute(
                "DELETE FROM authorizations WHERE connection_id = ?1 OR expires_at_ms < ?2",
                params![connection_id, now_ms as i64],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "UPDATE connections SET status = 'pending', updated_at_ms = ?2
                 WHERE connection_id = ?1",
                params![connection_id, now_ms as i64],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT INTO authorizations (
                    state_digest, connection_id, provider, return_url,
                    expires_at_ms, completed
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 0)",
                params![
                    state_digest,
                    connection_id,
                    provider,
                    return_url,
                    expires_at_ms as i64
                ],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())
    }

    pub fn upsert_ready_connection(
        &self,
        connection_id: &str,
        owner_id: &str,
        provider: &str,
        metadata: &Value,
        now_ms: u64,
    ) -> Result<ConnectionRecord, String> {
        let metadata_json = serde_json::to_string(metadata).map_err(|error| error.to_string())?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| "AEGS store lock was poisoned".to_string())?;
        connection
            .execute(
                "INSERT INTO connections (
                    connection_id, owner_id, provider, status, encrypted_credential,
                    metadata_json, expires_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, ?3, 'ready', NULL, ?4, NULL, ?5)
                 ON CONFLICT(connection_id) DO UPDATE SET
                    owner_id = excluded.owner_id,
                    provider = excluded.provider,
                    status = 'ready',
                    metadata_json = excluded.metadata_json,
                    updated_at_ms = excluded.updated_at_ms
                 WHERE connections.owner_id = excluded.owner_id
                    OR connections.owner_id = 'legacy'",
                params![
                    connection_id,
                    owner_id,
                    provider,
                    metadata_json,
                    now_ms as i64
                ],
            )
            .map_err(|error| error.to_string())?;
        drop(connection);
        self.claim_connection_owner(connection_id, owner_id)
    }

    pub fn authorization(
        &self,
        state_digest: &str,
        now_ms: u64,
    ) -> Result<Option<AuthorizationRecord>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "AEGS store lock was poisoned".to_string())?;
        connection
            .query_row(
                "
                SELECT state_digest, connection_id, provider, return_url,
                       expires_at_ms, completed
                FROM authorizations
                WHERE state_digest = ?1 AND expires_at_ms >= ?2
                ",
                params![state_digest, now_ms as i64],
                |row| {
                    Ok(AuthorizationRecord {
                        state_digest: row.get(0)?,
                        connection_id: row.get(1)?,
                        provider: row.get(2)?,
                        return_url: row.get(3)?,
                        expires_at_ms: row.get::<_, i64>(4)? as u64,
                        completed: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn complete_authorization(
        &self,
        state_digest: &str,
        encrypted_credential: &[u8],
        metadata: &Value,
        expires_at_ms: Option<u64>,
        now_ms: u64,
    ) -> Result<ConnectionRecord, String> {
        let metadata_json = serde_json::to_string(metadata).map_err(|error| error.to_string())?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "AEGS store lock was poisoned".to_string())?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let connection_id = transaction
            .query_row(
                "
                SELECT connection_id
                FROM authorizations
                WHERE state_digest = ?1 AND expires_at_ms >= ?2 AND completed = 0
                ",
                params![state_digest, now_ms as i64],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| {
                "authorization state is invalid, expired, or already used".to_string()
            })?;
        transaction
            .execute(
                "
                UPDATE connections
                SET status = 'ready', encrypted_credential = ?2, metadata_json = ?3,
                    expires_at_ms = ?4, updated_at_ms = ?5
                WHERE connection_id = ?1
                ",
                params![
                    connection_id,
                    encrypted_credential,
                    metadata_json,
                    expires_at_ms.map(|value| value as i64),
                    now_ms as i64,
                ],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "UPDATE authorizations SET completed = 1 WHERE state_digest = ?1",
                params![state_digest],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        drop(connection);
        self.connection(&connection_id)?
            .ok_or_else(|| "completed connection was not found".to_string())
    }

    pub fn connection(&self, connection_id: &str) -> Result<Option<ConnectionRecord>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "AEGS store lock was poisoned".to_string())?;
        connection
            .query_row(
                "
                SELECT connection_id, owner_id, provider, status, encrypted_credential,
                       metadata_json, expires_at_ms, updated_at_ms
                FROM connections WHERE connection_id = ?1
                ",
                params![connection_id],
                |row| {
                    let metadata_json: String = row.get(5)?;
                    Ok(ConnectionRecord {
                        connection_id: row.get(0)?,
                        owner_id: row.get(1)?,
                        provider: row.get(2)?,
                        status: row.get(3)?,
                        encrypted_credential: row.get(4)?,
                        metadata: serde_json::from_str(&metadata_json).unwrap_or(Value::Null),
                        expires_at_ms: row.get::<_, Option<i64>>(6)?.map(|value| value as u64),
                        updated_at_ms: row.get::<_, i64>(7)?.max(0) as u64,
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn claim_connection_owner(
        &self,
        connection_id: &str,
        owner_id: &str,
    ) -> Result<ConnectionRecord, String> {
        let connection = self
            .connection(connection_id)?
            .ok_or_else(|| "the authorized connection was not found".to_string())?;
        if connection.owner_id == owner_id {
            return Ok(connection);
        }
        if connection.owner_id != "legacy" {
            return Err("the authorized connection belongs to another owner".to_string());
        }
        let database = self
            .connection
            .lock()
            .map_err(|_| "AEGS store lock was poisoned".to_string())?;
        let changed = database
            .execute(
                "UPDATE connections SET owner_id = ?2 WHERE connection_id = ?1 AND owner_id = 'legacy'",
                params![connection_id, owner_id],
            )
            .map_err(|error| error.to_string())?;
        if changed == 0 {
            return Err("the authorized connection belongs to another owner".to_string());
        }
        drop(database);
        self.connection(connection_id)?
            .ok_or_else(|| "the authorized connection was not found".to_string())
    }

    pub fn connections_for_owner(&self, owner_id: &str) -> Result<Vec<ConnectionRecord>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "AEGS store lock was poisoned".to_string())?;
        let mut statement = connection
            .prepare(
                "SELECT connection_id, owner_id, provider, status, encrypted_credential,
                        metadata_json, expires_at_ms, updated_at_ms
                 FROM connections WHERE owner_id = ?1 ORDER BY updated_at_ms DESC, connection_id",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![owner_id], |row| {
                let metadata_json: String = row.get(5)?;
                Ok(ConnectionRecord {
                    connection_id: row.get(0)?,
                    owner_id: row.get(1)?,
                    provider: row.get(2)?,
                    status: row.get(3)?,
                    encrypted_credential: row.get(4)?,
                    metadata: serde_json::from_str(&metadata_json).unwrap_or(Value::Null),
                    expires_at_ms: row.get::<_, Option<i64>>(6)?.map(|value| value as u64),
                    updated_at_ms: row.get::<_, i64>(7)?.max(0) as u64,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn revoke_connection(
        &self,
        connection_id: &str,
        owner_id: &str,
        now_ms: u64,
    ) -> Result<bool, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "AEGS store lock was poisoned".to_string())?;
        connection
            .execute(
                "UPDATE connections SET status = 'revoked', encrypted_credential = NULL,
                        updated_at_ms = ?3
                 WHERE connection_id = ?1 AND owner_id = ?2 AND status != 'revoked'",
                params![connection_id, owner_id, now_ms as i64],
            )
            .map(|changed| changed > 0)
            .map_err(|error| error.to_string())
    }

    pub fn update_connection_credential(
        &self,
        connection_id: &str,
        encrypted_credential: &[u8],
        expires_at_ms: Option<u64>,
        now_ms: u64,
    ) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "AEGS store lock was poisoned".to_string())?;
        let changed = connection
            .execute(
                "
                UPDATE connections
                SET encrypted_credential = ?2, expires_at_ms = ?3, updated_at_ms = ?4
                WHERE connection_id = ?1 AND status = 'ready'
                ",
                params![
                    connection_id,
                    encrypted_credential,
                    expires_at_ms.map(|value| value as i64),
                    now_ms as i64,
                ],
            )
            .map_err(|error| error.to_string())?;
        if changed == 0 {
            return Err("ready connection was not found".to_string());
        }
        Ok(())
    }

    pub fn provider_hooks(&self) -> Result<Vec<ProviderHookRecord>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "AEGS store lock was poisoned".to_string())?;
        let mut statement = connection
            .prepare(
                "
                SELECT connection_id, connection_scope, provider_hook_id,
                       configuration_digest, expires_at_ms
                FROM provider_hooks
                ORDER BY connection_id, connection_scope
                ",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(ProviderHookRecord {
                    connection_id: row.get(0)?,
                    connection_scope: row.get(1)?,
                    provider_hook_id: row.get(2)?,
                    configuration_digest: row.get(3)?,
                    expires_at_ms: row.get::<_, Option<i64>>(4)?.map(|value| value as u64),
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn upsert_provider_hook(
        &self,
        hook: &ProviderHookRecord,
        now_ms: u64,
    ) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "AEGS store lock was poisoned".to_string())?;
        connection
            .execute(
                "
                INSERT INTO provider_hooks (
                    connection_id, connection_scope, provider_hook_id,
                    configuration_digest, expires_at_ms, updated_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                ON CONFLICT(connection_id, connection_scope) DO UPDATE SET
                    provider_hook_id = excluded.provider_hook_id,
                    configuration_digest = excluded.configuration_digest,
                    expires_at_ms = excluded.expires_at_ms,
                    updated_at_ms = excluded.updated_at_ms
                ",
                params![
                    hook.connection_id,
                    hook.connection_scope,
                    hook.provider_hook_id,
                    hook.configuration_digest,
                    hook.expires_at_ms.map(|value| value as i64),
                    now_ms as i64,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn delete_provider_hook(
        &self,
        connection_id: &str,
        connection_scope: &str,
    ) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "AEGS store lock was poisoned".to_string())?;
        connection
            .execute(
                "
                DELETE FROM provider_hooks
                WHERE connection_id = ?1 AND connection_scope = ?2
                ",
                params![connection_id, connection_scope],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}

fn migrate_subscription_owner(connection: &Connection) -> Result<(), rusqlite::Error> {
    let mut columns = connection.prepare("PRAGMA table_info(subscriptions)")?;
    let has_owner = columns
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .any(|column| column == "owner_id");
    drop(columns);
    if !has_owner {
        connection.execute(
            "ALTER TABLE subscriptions
             ADD COLUMN owner_id TEXT NOT NULL DEFAULT 'legacy'",
            [],
        )?;
    }
    Ok(())
}

fn migrate_connection_owner(connection: &Connection) -> Result<(), rusqlite::Error> {
    let mut columns = connection.prepare("PRAGMA table_info(connections)")?;
    let has_owner = columns
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .any(|column| column == "owner_id");
    drop(columns);
    if !has_owner {
        connection.execute_batch(
            "ALTER TABLE connections
             ADD COLUMN owner_id TEXT NOT NULL DEFAULT 'legacy';",
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(binding_id: &str, revision: u64, active: bool) -> SubscriptionClaim {
        SubscriptionClaim {
            binding_id: binding_id.to_string(),
            generator_id: "dev.arroba.github".to_string(),
            connection_id: "installation-1".to_string(),
            connection_scope: "arroba/arroba".to_string(),
            event_interest_key: format!("sha256:{binding_id}"),
            event_type: "pull_request.opened".to_string(),
            event_type_version: 1,
            filter: serde_json::json!({}),
            revision,
            active,
        }
    }

    #[test]
    fn reconciliation_is_authoritative_and_revision_fenced() {
        let store = AegsStore::open(":memory:").unwrap();
        assert_eq!(
            store
                .reconcile(
                    "kernel-a",
                    "dev.arroba.github",
                    &[claim("binding-a", 2, true)],
                )
                .unwrap(),
            vec!["binding-a"]
        );
        assert!(store
            .matching("dev.arroba.github", "pull_request.opened", "arroba/arroba")
            .unwrap()
            .iter()
            .any(|value| value.binding_id == "binding-a"));
        assert_eq!(
            store.metrics().unwrap(),
            AegsStoreMetrics {
                active_subscriptions: 1,
                subscriptions: 1,
                connections: 0,
                provider_hooks: 0,
            }
        );

        assert!(store
            .reconcile(
                "kernel-a",
                "dev.arroba.github",
                &[claim("binding-a", 1, true)],
            )
            .unwrap()
            .is_empty());
        assert_eq!(store.all("dev.arroba.github").unwrap()[0].revision, 2);

        store
            .reconcile("kernel-a", "dev.arroba.github", &[])
            .unwrap();
        assert!(store
            .matching("dev.arroba.github", "pull_request.opened", "arroba/arroba")
            .unwrap()
            .is_empty());
        assert_eq!(store.metrics().unwrap().active_subscriptions, 0);
    }

    #[test]
    fn reconciliation_is_authoritative_only_for_its_owner_and_fences_transfer_revision() {
        let store = AegsStore::open(":memory:").unwrap();
        store
            .reconcile(
                "kernel-a",
                "dev.arroba.github",
                &[claim("binding-a", 2, true)],
            )
            .unwrap();
        store
            .reconcile(
                "kernel-b",
                "dev.arroba.github",
                &[claim("binding-b", 1, true)],
            )
            .unwrap();
        assert_eq!(store.metrics().unwrap().active_subscriptions, 2);

        store
            .reconcile("kernel-b", "dev.arroba.github", &[])
            .unwrap();
        let subscriptions = store.all("dev.arroba.github").unwrap();
        assert!(subscriptions
            .iter()
            .any(|subscription| subscription.binding_id == "binding-a" && subscription.active));
        assert!(subscriptions
            .iter()
            .any(|subscription| subscription.binding_id == "binding-b" && !subscription.active));

        assert!(store
            .reconcile(
                "kernel-b",
                "dev.arroba.github",
                &[claim("binding-a", 2, true)],
            )
            .unwrap()
            .is_empty());
        assert_eq!(store.metrics().unwrap().active_subscriptions, 1);

        assert_eq!(
            store
                .reconcile(
                    "kernel-b",
                    "dev.arroba.github",
                    &[claim("binding-a", 3, true)],
                )
                .unwrap(),
            vec!["binding-a"]
        );
        store
            .reconcile("kernel-a", "dev.arroba.github", &[])
            .unwrap();
        assert!(store
            .matching("dev.arroba.github", "pull_request.opened", "arroba/arroba")
            .unwrap()
            .iter()
            .any(|subscription| {
                subscription.binding_id == "binding-a"
                    && subscription.revision == 3
                    && subscription.active
            }));
    }

    #[test]
    fn authorization_state_is_single_use_and_persists_opaque_credentials() {
        let store = AegsStore::open(":memory:").unwrap();
        store
            .create_authorization(CreateAuthorizationRequest {
                state_digest: "state-digest",
                connection_id: "connection-1",
                owner_id: "owner-kernel-user",
                provider: "github",
                return_url: Some("https://terminal.example/workflows"),
                expires_at_ms: 2_000,
                now_ms: 1_000,
            })
            .unwrap();
        let pending = store.connection("connection-1").unwrap().unwrap();
        assert_eq!(pending.status, "pending");
        assert_eq!(pending.owner_id, "owner-kernel-user");

        let ready = store
            .complete_authorization(
                "state-digest",
                b"opaque-encrypted-credential",
                &serde_json::json!({"installation_id": 42}),
                Some(3_000),
                1_500,
            )
            .unwrap();
        assert_eq!(ready.status, "ready");
        assert_eq!(
            ready.encrypted_credential.as_deref(),
            Some(b"opaque-encrypted-credential".as_slice())
        );
        assert!(store
            .complete_authorization("state-digest", b"replacement", &Value::Null, None, 1_600,)
            .unwrap_err()
            .contains("already used"));
    }

    #[test]
    fn connections_are_owner_scoped_and_legacy_connections_are_claimed_once() {
        let store = AegsStore::open(":memory:").unwrap();
        store
            .create_authorization(CreateAuthorizationRequest {
                state_digest: "state-owner-a",
                connection_id: "connection-owner-a",
                owner_id: "owner-a",
                provider: "github",
                return_url: None,
                expires_at_ms: 2_000,
                now_ms: 1_000,
            })
            .unwrap();
        assert_eq!(store.connections_for_owner("owner-a").unwrap().len(), 1);
        assert!(store.connections_for_owner("owner-b").unwrap().is_empty());
        assert!(store
            .claim_connection_owner("connection-owner-a", "owner-b")
            .unwrap_err()
            .contains("another owner"));

        {
            let database = store.connection.lock().unwrap();
            database
                .execute(
                    "UPDATE connections SET owner_id = 'legacy' WHERE connection_id = ?1",
                    params!["connection-owner-a"],
                )
                .unwrap();
        }
        assert_eq!(
            store
                .claim_connection_owner("connection-owner-a", "owner-b")
                .unwrap()
                .owner_id,
            "owner-b"
        );
        assert!(store
            .claim_connection_owner("connection-owner-a", "owner-a")
            .unwrap_err()
            .contains("another owner"));
    }

    #[test]
    fn reauthorization_retains_connection_identity_and_owner() {
        let store = AegsStore::open(":memory:").unwrap();
        store
            .create_authorization(CreateAuthorizationRequest {
                state_digest: "initial-state",
                connection_id: "connection-owner-a",
                owner_id: "owner-a",
                provider: "github",
                return_url: None,
                expires_at_ms: 2_000,
                now_ms: 1_000,
            })
            .unwrap();
        store
            .complete_authorization(
                "initial-state",
                b"old-encrypted-credential",
                &serde_json::json!({"account": "arroba"}),
                None,
                1_100,
            )
            .unwrap();
        store
            .create_reauthorization(CreateAuthorizationRequest {
                state_digest: "replacement-state",
                connection_id: "connection-owner-a",
                owner_id: "owner-a",
                provider: "github",
                return_url: Some("https://terminal.arroba.dev/notifications/callback"),
                expires_at_ms: 3_000,
                now_ms: 2_000,
            })
            .unwrap();
        let pending = store.connection("connection-owner-a").unwrap().unwrap();
        assert_eq!(pending.owner_id, "owner-a");
        assert_eq!(pending.status, "pending");
        assert!(store
            .create_reauthorization(CreateAuthorizationRequest {
                state_digest: "foreign-state",
                connection_id: "connection-owner-a",
                owner_id: "owner-b",
                provider: "github",
                return_url: None,
                expires_at_ms: 3_000,
                now_ms: 2_100,
            })
            .unwrap_err()
            .contains("owned connection"));
        let ready = store
            .complete_authorization(
                "replacement-state",
                b"new-encrypted-credential",
                &serde_json::json!({"account": "arroba"}),
                None,
                2_200,
            )
            .unwrap();
        assert_eq!(ready.connection_id, "connection-owner-a");
        assert_eq!(ready.owner_id, "owner-a");
        assert_eq!(ready.status, "ready");
    }
}
