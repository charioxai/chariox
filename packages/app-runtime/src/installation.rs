//! Kernel-owned installation metadata and update transitions.
//!
//! This registry borrows the kernel's SQLite connection. It neither verifies
//! packages nor launches workers, snapshots files, authenticates users, or
//! changes user workflows. The trusted supervisor supplies verified release
//! metadata and policy decisions, fences all writers before preparation, and
//! completes staged migration/health checks before calling `mark_prepared`.
//! Committing metadata alone is not evidence those external steps happened.

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};

pub const RETAINED_UPDATE_RECORDS: usize = 64;
pub const MAX_INSTALLATION_PAGE_SIZE: usize = 100;

#[derive(Debug, thiserror::Error)]
pub enum InstallationError {
    #[error("app_installation_database: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("app_installation_encoding: {0}")]
    Encoding(#[from] serde_json::Error),
    #[error("app_installation_not_found")]
    NotFound,
    #[error("app_installation_generation_conflict")]
    Conflict,
    #[error("app_installation_inactive")]
    Inactive,
    #[error("app_installation_admission_paused")]
    AdmissionPaused,
    #[error("app_installation_invalid: {0}")]
    Invalid(&'static str),
    #[error("app_installation_invalid_transition")]
    InvalidTransition,
    #[error("app_installation_approval_required")]
    ApprovalRequired,
}

type Result<T> = std::result::Result<T, InstallationError>;

/// All digests and the publisher identity come from the trusted package verifier.
/// These fields describe one immutable release and cannot change after staging.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseMetadata {
    pub app_id: String,
    pub version: String,
    pub publisher_id: String,
    pub package_digest: String,
    pub schema_version: u32,
    pub capabilities_digest: String,
    pub catalog_digest: String,
    pub view_digest: String,
}

/// Opaque reference to a kernel-recorded human or existing-policy decision.
/// The caller must authenticate its authority; an App-supplied string is not
/// sufficient. Approval applies only to the immutable release in its stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityApproval {
    pub decision_id: String,
    pub authority_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum CapabilityDecision {
    Pending,
    Approved {
        approval: CapabilityApproval,
    },
    Declined {
        decision_id: String,
        authority_ref: String,
    },
}

/// Monotonic generation allocation prevents an aborted stage token from ever
/// identifying a later candidate. Tokens are correlation, not authorization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageToken {
    pub installation_id: String,
    pub base_generation: u64,
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePhase {
    Staged,
    Quiescing,
    Prepared,
    Committed,
    Aborted,
}

impl UpdatePhase {
    fn name(self) -> &'static str {
        match self {
            Self::Staged => "staged",
            Self::Quiescing => "quiescing",
            Self::Prepared => "prepared",
            Self::Committed => "committed",
            Self::Aborted => "aborted",
        }
    }

    fn terminal(self) -> bool {
        matches!(self, Self::Committed | Self::Aborted)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateRecord {
    pub token: StageToken,
    pub release: ReleaseMetadata,
    pub decision: CapabilityDecision,
    pub phase: UpdatePhase,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub abort_reason: Option<String>,
}

/// One atomic record identifies worker, staged data, capabilities, catalog and
/// view generation. The supervisor derives their runtime handles from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveGeneration {
    pub generation: u64,
    pub release: ReleaseMetadata,
    pub approval: CapabilityApproval,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installation {
    pub installation_id: String,
    pub app_id: String,
    pub owner_id: String,
    pub generation: u64,
    pub active: Option<ActiveGeneration>,
    pub pending_generation: Option<u64>,
    pub admission_paused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallationPage {
    pub installations: Vec<Installation>,
    pub next_cursor: Option<String>,
}

pub struct InstallationRegistry<'a> {
    connection: &'a mut Connection,
}

impl<'a> InstallationRegistry<'a> {
    /// Connection ownership, durability pragmas and writer admission remain the
    /// kernel's responsibility. This does not open a second database.
    pub fn new(connection: &'a mut Connection) -> Self {
        Self { connection }
    }

    pub fn initialize(&mut self) -> Result<()> {
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS app_installations (
                installation_id TEXT PRIMARY KEY,
                app_id TEXT NOT NULL,
                owner_id TEXT NOT NULL,
                generation INTEGER NOT NULL DEFAULT 0 CHECK(generation >= 0),
                allocated_generation INTEGER NOT NULL DEFAULT 0
                    CHECK(allocated_generation >= generation),
                active_json TEXT,
                pending_generation INTEGER,
                admission_paused INTEGER NOT NULL DEFAULT 0
                    CHECK(admission_paused IN (0, 1))
             );
             CREATE INDEX IF NOT EXISTS app_installations_owner
                ON app_installations(owner_id, installation_id);
             CREATE TABLE IF NOT EXISTS app_installation_updates (
                installation_id TEXT NOT NULL,
                generation INTEGER NOT NULL CHECK(generation > 0),
                phase TEXT NOT NULL CHECK(phase IN
                    ('staged', 'quiescing', 'prepared', 'committed', 'aborted')),
                record_json TEXT NOT NULL,
                PRIMARY KEY(installation_id, generation)
             );",
        )?;
        Ok(())
    }

    pub fn create(
        &mut self,
        installation_id: &str,
        app_id: &str,
        owner_id: &str,
    ) -> Result<Installation> {
        create_installation(self.connection, installation_id, app_id, owner_id)?;
        self.get(installation_id)
    }

    /// Initial identity and candidate are committed together. A failed stage
    /// leaves neither an installation nor a consumed generation behind.
    pub fn create_and_stage(
        &mut self,
        installation_id: &str,
        owner_id: &str,
        release: ReleaseMetadata,
        now_ms: u64,
    ) -> Result<UpdateRecord> {
        validate_release(&release)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        create_installation(&transaction, installation_id, &release.app_id, owner_id)?;
        let record = stage_release(&transaction, installation_id, 0, release, now_ms)?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn get(&self, installation_id: &str) -> Result<Installation> {
        load_installation(self.connection, installation_id)
    }

    /// Stable identifier pagination, always scoped to one authenticated owner
    /// supplied by the kernel. Concurrent state changes appear on later reads.
    pub fn list(
        &self,
        owner_id: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<InstallationPage> {
        identifier(owner_id)?;
        if let Some(cursor) = after {
            identifier(cursor)?;
        }
        if !(1..=MAX_INSTALLATION_PAGE_SIZE).contains(&limit) {
            return Err(InstallationError::Invalid("installation page limit"));
        }
        let mut statement = self.connection.prepare(
            "SELECT installation_id, app_id, owner_id, generation, active_json,
                pending_generation, admission_paused FROM app_installations
             WHERE owner_id = ?1 AND installation_id > ?2
             ORDER BY installation_id LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![owner_id, after.unwrap_or(""), sql_limit(limit + 1)?],
            |row| Ok((row.get::<_, String>(0)?, stored_installation_row(row, 1)?)),
        )?;
        let mut installations = rows
            .map(|row| {
                let (id, fields) = row?;
                installation_from_fields(&id, fields)
            })
            .collect::<Result<Vec<_>>>()?;
        let next_cursor = if installations.len() > limit {
            installations.pop();
            installations
                .last()
                .map(|item| item.installation_id.clone())
        } else {
            None
        };
        Ok(InstallationPage {
            installations,
            next_cursor,
        })
    }

    /// Check immediately before admission; an old cached catalog is insufficient.
    /// The supervisor must also cancel/fence already admitted work on quiesce.
    pub fn require_active(
        &self,
        installation_id: &str,
        generation: u64,
    ) -> Result<ActiveGeneration> {
        let installation = self.get(installation_id)?;
        expected_generation(&installation, generation)?;
        if installation.admission_paused {
            return Err(InstallationError::AdmissionPaused);
        }
        installation.active.ok_or(InstallationError::Inactive)
    }

    pub fn stage(
        &mut self,
        installation_id: &str,
        expected: u64,
        release: ReleaseMetadata,
        now_ms: u64,
    ) -> Result<UpdateRecord> {
        validate_release(&release)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record = stage_release(&transaction, installation_id, expected, release, now_ms)?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn decide(
        &mut self,
        token: &StageToken,
        decision: CapabilityDecision,
        now_ms: u64,
    ) -> Result<UpdateRecord> {
        match &decision {
            CapabilityDecision::Approved { approval } => {
                identifier(&approval.decision_id)?;
                identifier(&approval.authority_ref)?;
            }
            CapabilityDecision::Declined {
                decision_id,
                authority_ref,
            } => {
                identifier(decision_id)?;
                identifier(authority_ref)?;
            }
            CapabilityDecision::Pending => return Err(InstallationError::InvalidTransition),
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut record = current_update(&transaction, token)?;
        if record.phase != UpdatePhase::Staged || record.decision != CapabilityDecision::Pending {
            return Err(InstallationError::InvalidTransition);
        }
        record.decision = decision;
        record.updated_at_ms = now_ms;
        if matches!(record.decision, CapabilityDecision::Declined { .. }) {
            record.phase = UpdatePhase::Aborted;
            record.abort_reason = Some("capabilities declined".into());
            clear_pending(&transaction, token)?;
        }
        save_update(&transaction, &record)?;
        prune_journal(&transaction, &token.installation_id)?;
        transaction.commit()?;
        Ok(record)
    }

    /// Durably stops new operation admission. The supervisor then stops existing
    /// workers, timers and descriptors before preparing a staged data generation.
    pub fn quiesce(&mut self, token: &StageToken, now_ms: u64) -> Result<UpdateRecord> {
        self.advance(token, UpdatePhase::Staged, UpdatePhase::Quiescing, now_ms)
    }

    /// Records the supervisor's completed preparation. Does not perform or prove
    /// filesystem snapshot, migration, sandbox lockdown or worker health itself.
    pub fn mark_prepared(&mut self, token: &StageToken, now_ms: u64) -> Result<UpdateRecord> {
        self.advance(token, UpdatePhase::Quiescing, UpdatePhase::Prepared, now_ms)
    }

    fn advance(
        &mut self,
        token: &StageToken,
        from: UpdatePhase,
        to: UpdatePhase,
        now_ms: u64,
    ) -> Result<UpdateRecord> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut record = current_update(&transaction, token)?;
        if record.phase != from {
            return Err(InstallationError::InvalidTransition);
        }
        if !matches!(record.decision, CapabilityDecision::Approved { .. }) {
            return Err(InstallationError::ApprovalRequired);
        }
        record.phase = to;
        record.updated_at_ms = now_ms;
        transaction.execute(
            "UPDATE app_installations SET admission_paused = 1 WHERE installation_id = ?1",
            [&token.installation_id],
        )?;
        save_update(&transaction, &record)?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn commit(&mut self, token: &StageToken, now_ms: u64) -> Result<ActiveGeneration> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut record = current_update(&transaction, token)?;
        if record.phase != UpdatePhase::Prepared {
            return Err(InstallationError::InvalidTransition);
        }
        let CapabilityDecision::Approved { approval } = record.decision.clone() else {
            return Err(InstallationError::ApprovalRequired);
        };
        let active = ActiveGeneration {
            generation: token.generation,
            release: record.release.clone(),
            approval,
        };
        let changed = transaction.execute(
            "UPDATE app_installations SET generation = ?2, active_json = ?3,
                 pending_generation = NULL, admission_paused = 0
             WHERE installation_id = ?1 AND generation = ?4 AND pending_generation = ?2",
            params![
                token.installation_id,
                sql_generation(token.generation)?,
                serde_json::to_string(&active)?,
                sql_generation(token.base_generation)?
            ],
        )?;
        require_changed(changed)?;
        record.phase = UpdatePhase::Committed;
        record.updated_at_ms = now_ms;
        save_update(&transaction, &record)?;
        prune_journal(&transaction, &token.installation_id)?;
        transaction.commit()?;
        Ok(active)
    }

    /// Only a precommit candidate can be aborted. No method restores an old
    /// active-data snapshot after commit; downgrade needs an explicit migration.
    pub fn abort(&mut self, token: &StageToken, reason: &str, now_ms: u64) -> Result<UpdateRecord> {
        if reason.is_empty() || reason.len() > 512 || reason.chars().any(char::is_control) {
            return Err(InstallationError::Invalid("abort reason"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut record = current_update(&transaction, token)?;
        record.phase = UpdatePhase::Aborted;
        record.abort_reason = Some(reason.into());
        record.updated_at_ms = now_ms;
        clear_pending(&transaction, token)?;
        save_update(&transaction, &record)?;
        prune_journal(&transaction, &token.installation_id)?;
        transaction.commit()?;
        Ok(record)
    }

    /// Deactivates installation metadata and fences all prior generations.
    /// Worker stop, grant/token revocation and App data retention are supervisor
    /// responsibilities. User workflow and agent assets are never deleted here.
    pub fn uninstall(
        &mut self,
        installation_id: &str,
        expected: u64,
        now_ms: u64,
    ) -> Result<Installation> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let installation = load_installation(&transaction, installation_id)?;
        expected_generation(&installation, expected)?;
        if let Some(pending) = installation.pending_generation {
            let mut record = load_update(&transaction, installation_id, pending)?;
            record.phase = UpdatePhase::Aborted;
            record.abort_reason = Some("installation uninstalled".into());
            record.updated_at_ms = now_ms;
            save_update(&transaction, &record)?;
        }
        let generation = allocate_generation(&transaction, installation_id)?;
        let changed = transaction.execute(
            "UPDATE app_installations SET generation = ?2, active_json = NULL,
                pending_generation = NULL, admission_paused = 0
             WHERE installation_id = ?1 AND generation = ?3",
            params![
                installation_id,
                sql_generation(generation)?,
                sql_generation(expected)?
            ],
        )?;
        require_changed(changed)?;
        prune_journal(&transaction, installation_id)?;
        let result = load_installation(&transaction, installation_id)?;
        transaction.commit()?;
        Ok(result)
    }

    /// At most one pending entry plus the most recent terminal records.
    pub fn journal(&self, installation_id: &str) -> Result<Vec<UpdateRecord>> {
        self.get(installation_id)?;
        let mut statement = self.connection.prepare(
            "SELECT record_json FROM app_installation_updates
             WHERE installation_id = ?1 ORDER BY generation DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(
            params![installation_id, sql_limit(RETAINED_UPDATE_RECORDS + 1)?],
            |row| row.get::<_, String>(0),
        )?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }
}

fn create_installation(
    connection: &Connection,
    installation_id: &str,
    app_id: &str,
    owner_id: &str,
) -> Result<()> {
    identifier(installation_id)?;
    identifier(app_id)?;
    identifier(owner_id)?;
    require_changed(connection.execute(
        "INSERT INTO app_installations(installation_id, app_id, owner_id)
         VALUES (?1, ?2, ?3) ON CONFLICT(installation_id) DO NOTHING",
        params![installation_id, app_id, owner_id],
    )?)
}

fn stage_release(
    connection: &Connection,
    installation_id: &str,
    expected: u64,
    release: ReleaseMetadata,
    now_ms: u64,
) -> Result<UpdateRecord> {
    let installation = load_installation(connection, installation_id)?;
    expected_generation(&installation, expected)?;
    if installation.pending_generation.is_some() {
        return Err(InstallationError::Conflict);
    }
    if installation.app_id != release.app_id {
        return Err(InstallationError::Invalid("different App identity"));
    }
    if installation
        .active
        .as_ref()
        .is_some_and(|active| active.release.publisher_id != release.publisher_id)
    {
        return Err(InstallationError::Invalid(
            "publisher change requires separate trust migration",
        ));
    }
    let generation = allocate_generation(connection, installation_id)?;
    let record = UpdateRecord {
        token: StageToken {
            installation_id: installation_id.into(),
            base_generation: expected,
            generation,
        },
        release,
        decision: CapabilityDecision::Pending,
        phase: UpdatePhase::Staged,
        created_at_ms: now_ms,
        updated_at_ms: now_ms,
        abort_reason: None,
    };
    connection.execute(
        "INSERT INTO app_installation_updates
             (installation_id, generation, phase, record_json)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            installation_id,
            sql_generation(generation)?,
            record.phase.name(),
            serde_json::to_string(&record)?
        ],
    )?;
    let changed = connection.execute(
        "UPDATE app_installations SET pending_generation = ?2
         WHERE installation_id = ?1 AND generation = ?3 AND pending_generation IS NULL",
        params![
            installation_id,
            sql_generation(generation)?,
            sql_generation(expected)?
        ],
    )?;
    require_changed(changed)?;
    Ok(record)
}

fn load_installation(connection: &Connection, id: &str) -> Result<Installation> {
    let row = connection
        .query_row(
            "SELECT app_id, owner_id, generation, active_json, pending_generation,
            admission_paused FROM app_installations WHERE installation_id = ?1",
            [id],
            |row| stored_installation_row(row, 0),
        )
        .optional()?
        .ok_or(InstallationError::NotFound)?;
    installation_from_fields(id, row)
}

type StoredInstallation = (String, String, i64, Option<String>, Option<i64>, bool);

fn stored_installation_row(
    row: &rusqlite::Row<'_>,
    start: usize,
) -> rusqlite::Result<StoredInstallation> {
    Ok((
        row.get(start)?,
        row.get(start + 1)?,
        row.get(start + 2)?,
        row.get(start + 3)?,
        row.get(start + 4)?,
        row.get(start + 5)?,
    ))
}

fn installation_from_fields(id: &str, row: StoredInstallation) -> Result<Installation> {
    Ok(Installation {
        installation_id: id.into(),
        app_id: row.0,
        owner_id: row.1,
        generation: stored_generation(row.2)?,
        active: row.3.map(|json| serde_json::from_str(&json)).transpose()?,
        pending_generation: row.4.map(stored_generation).transpose()?,
        admission_paused: row.5,
    })
}

fn load_update(connection: &Connection, id: &str, generation: u64) -> Result<UpdateRecord> {
    let json: String = connection.query_row(
        "SELECT record_json FROM app_installation_updates WHERE installation_id = ?1 AND generation = ?2",
        params![id, sql_generation(generation)?], |row| row.get(0),
    ).optional()?.ok_or(InstallationError::NotFound)?;
    Ok(serde_json::from_str(&json)?)
}

fn current_update(connection: &Connection, token: &StageToken) -> Result<UpdateRecord> {
    sql_generation(token.base_generation)?;
    sql_generation(token.generation)?;
    let record = load_update(connection, &token.installation_id, token.generation)?;
    if record.phase.terminal() {
        return Err(InstallationError::InvalidTransition);
    }
    let installation = load_installation(connection, &token.installation_id)?;
    expected_generation(&installation, token.base_generation)?;
    if record.token != *token || installation.pending_generation != Some(token.generation) {
        return Err(InstallationError::Conflict);
    }
    Ok(record)
}

fn save_update(connection: &Connection, record: &UpdateRecord) -> Result<()> {
    require_changed(connection.execute(
        "UPDATE app_installation_updates SET phase = ?3, record_json = ?4
         WHERE installation_id = ?1 AND generation = ?2",
        params![
            record.token.installation_id,
            sql_generation(record.token.generation)?,
            record.phase.name(),
            serde_json::to_string(record)?
        ],
    )?)
}

fn clear_pending(connection: &Connection, token: &StageToken) -> Result<()> {
    require_changed(connection.execute(
        "UPDATE app_installations SET pending_generation = NULL, admission_paused = 0
         WHERE installation_id = ?1 AND generation = ?2 AND pending_generation = ?3",
        params![
            token.installation_id,
            sql_generation(token.base_generation)?,
            sql_generation(token.generation)?
        ],
    )?)
}

fn allocate_generation(connection: &Connection, id: &str) -> Result<u64> {
    let generation: i64 = connection.query_row(
        "SELECT allocated_generation FROM app_installations WHERE installation_id = ?1",
        [id],
        |row| row.get(0),
    )?;
    let next = stored_generation(generation)?
        .checked_add(1)
        .ok_or(InstallationError::Invalid("generation exhausted"))?;
    let next_sql =
        i64::try_from(next).map_err(|_| InstallationError::Invalid("generation exhausted"))?;
    connection.execute(
        "UPDATE app_installations SET allocated_generation = ?2 WHERE installation_id = ?1",
        params![id, next_sql],
    )?;
    Ok(next)
}

fn prune_journal(connection: &Connection, id: &str) -> Result<()> {
    connection.execute(
        "DELETE FROM app_installation_updates WHERE installation_id = ?1
         AND phase IN ('committed', 'aborted') AND generation NOT IN
            (SELECT generation FROM app_installation_updates WHERE installation_id = ?1
             AND phase IN ('committed', 'aborted') ORDER BY generation DESC LIMIT ?2)",
        params![id, sql_limit(RETAINED_UPDATE_RECORDS)?],
    )?;
    Ok(())
}

fn expected_generation(installation: &Installation, expected: u64) -> Result<()> {
    sql_generation(expected)?;
    if installation.generation != expected {
        return Err(InstallationError::Conflict);
    }
    Ok(())
}

fn sql_generation(generation: u64) -> Result<i64> {
    i64::try_from(generation)
        .map_err(|_| InstallationError::Invalid("generation outside SQLite integer range"))
}

fn stored_generation(generation: i64) -> Result<u64> {
    u64::try_from(generation).map_err(|_| InstallationError::Invalid("negative stored generation"))
}

fn sql_limit(limit: usize) -> Result<i64> {
    i64::try_from(limit).map_err(|_| InstallationError::Invalid("journal limit"))
}

fn require_changed(changed: usize) -> Result<()> {
    if changed != 1 {
        return Err(InstallationError::Conflict);
    }
    Ok(())
}

fn identifier(value: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        return Err(InstallationError::Invalid("identifier"));
    }
    Ok(())
}

fn validate_release(release: &ReleaseMetadata) -> Result<()> {
    identifier(&release.app_id)?;
    identifier(&release.version)?;
    identifier(&release.publisher_id)?;
    for digest in [
        &release.package_digest,
        &release.capabilities_digest,
        &release.catalog_digest,
        &release.view_digest,
    ] {
        let value = digest
            .strip_prefix("sha256:")
            .ok_or(InstallationError::Invalid("digest"))?;
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(InstallationError::Invalid("digest"));
        }
    }
    Ok(())
}
