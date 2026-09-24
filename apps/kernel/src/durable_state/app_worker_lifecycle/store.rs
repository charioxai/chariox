use super::*;
use chariox_app_runtime::{
    installation::InstallationRegistry, publisher_trust::PublisherTrustRegistry,
};
use rusqlite::{params, OptionalExtension, TransactionBehavior};

fn sql<T>(result: rusqlite::Result<T>) -> Result<T> {
    result.map_err(|_| LifecycleStoreError::Storage)
}
fn checked(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| LifecycleStoreError::Stale)
}
fn identity(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        Err(LifecycleStoreError::Stale)
    } else {
        Ok(())
    }
}
fn budget(value: &AppOperationBudget) -> Result<()> {
    value.check().map_err(|_| LifecycleStoreError::Stopped)
}
pub(super) fn first_committed(
    tx: &rusqlite::Transaction<'_>,
    owner: &str,
    attempt: &str,
    binding: &StageTrustBinding,
    trust: &TrustedPublisherSnapshot,
) -> Result<ActiveStartAdmission> {
    identity(owner)?;
    identity(attempt)?;
    if binding.token().base_generation != 0 || binding.token().generation != 1 {
        return Err(LifecycleStoreError::Stale);
    }
    binding.require_active(tx, owner, trust).map_err(|_| LifecycleStoreError::Stale)?;
    let installation = &binding.token().installation_id;
    match status(tx, owner, installation)? {
        Some(current) if current.generation != 1 || current.attempt != attempt
            || current.phase != WorkerPhase::Starting || !current.desired_running => return Err(LifecycleStoreError::Stale),
        Some(_) => {},
        None => {
            sql(tx.execute("INSERT INTO app_worker_lifecycle(installation_id,owner_id,generation,attempt,phase,desired_running,failure,updated_ms)
                VALUES(?1,?2,1,?3,'starting',1,NULL,?4)", params![installation,owner,attempt,checked(crate::session::unix_epoch_ms())?]))?;
        }
    }
    Ok(ActiveStartAdmission { owner:owner.into(), installation:installation.into(), attempt:attempt.into(), binding:binding.clone(), trust:trust.clone() })
}
pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS app_worker_lifecycle (
        installation_id TEXT PRIMARY KEY,owner_id TEXT NOT NULL,generation INTEGER NOT NULL CHECK(generation>=0),
        attempt TEXT NOT NULL,phase TEXT NOT NULL CHECK(phase IN ('starting','running','stopped','failed')),
        desired_running INTEGER NOT NULL CHECK(desired_running IN (0,1)),failure TEXT,updated_ms INTEGER NOT NULL CHECK(updated_ms>=0));")
}
pub(super) fn apply(connection: &mut Connection, command: Command) -> Result<Reply> {
    let now = checked(crate::session::unix_epoch_ms())?;
    match command {
        Command::Claim {
            owner,
            installation,
            attempt,
            recovery,
            budget: limit,
        } => {
            identity(&owner)?;
            identity(&installation)?;
            identity(&attempt)?;
            budget(&limit)?;
            let binding = InstallationRegistry::new(connection)
                .active_trust(&owner, &installation)
                .map_err(|_| LifecycleStoreError::Stale)?;
            let trust = PublisherTrustRegistry::new(connection)
                .trusted_publisher(&owner, binding.publisher_id(), binding.key_id())
                .map_err(|_| LifecycleStoreError::Stale)?;
            let tx = sql(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
            binding
                .require_active(&tx, &owner, &trust)
                .map_err(|_| LifecycleStoreError::Stale)?;
            budget(&limit)?;
            if recovery
                && status(&tx, &owner, &installation)?.is_some_and(|old| {
                    old.generation == binding.token().generation
                        && (!old.desired_running || old.phase == WorkerPhase::Failed)
                })
            {
                return Err(LifecycleStoreError::Stopped);
            }
            sql(tx.execute("INSERT INTO app_worker_lifecycle(installation_id,owner_id,generation,attempt,phase,desired_running,failure,updated_ms)
                VALUES(?1,?2,?3,?4,'starting',1,NULL,?5) ON CONFLICT(installation_id) DO UPDATE SET owner_id=excluded.owner_id,generation=excluded.generation,attempt=excluded.attempt,phase='starting',desired_running=1,failure=NULL,updated_ms=excluded.updated_ms",
                params![installation,owner,checked(binding.token().generation)?,attempt,now]))?;
            budget(&limit)?;
            sql(tx.commit())?;
            Ok(Reply::Admitted(ActiveStartAdmission {
                owner,
                installation,
                attempt,
                binding,
                trust,
            }))
        }
        Command::Verify {
            owner,
            attempt,
            binding,
            trust,
            budget: limit,
        } => {
            budget(&limit)?;
            let tx = sql(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
            binding
                .require_active(&tx, &owner, &trust)
                .map_err(|_| LifecycleStoreError::Stale)?;
            budget(&limit)?;
            let current = status(&tx, &owner, &binding.token().installation_id)?
                .ok_or(LifecycleStoreError::Stale)?;
            if current.attempt != attempt
                || !current.desired_running
                || !matches!(current.phase, WorkerPhase::Starting | WorkerPhase::Running)
            {
                return Err(LifecycleStoreError::Stopped);
            }
            sql(tx.commit())?;
            Ok(Reply::Done)
        }
        Command::Transition {
            owner,
            installation,
            attempt,
            binding,
            trust,
            phase,
            desired_running,
            failure,
            budget: limit,
        } => {
            budget(&limit)?;
            if failure.as_ref().is_some_and(|v| {
                v.len() > 96 || !v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            }) {
                return Err(LifecycleStoreError::Stale);
            }
            let tx = sql(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
            budget(&limit)?;
            let old = status(&tx, &owner, &installation)?.ok_or(LifecycleStoreError::Stale)?;
            if old.attempt != attempt || old.generation != binding.token().generation {
                return Err(LifecycleStoreError::Stale);
            }
            if phase == WorkerPhase::Running
                && (!old.desired_running || old.phase != WorkerPhase::Starting)
            {
                return Err(LifecycleStoreError::Stopped);
            }
            if phase == WorkerPhase::Running {
                binding
                    .require_active(&tx, &owner, &trust)
                    .map_err(|_| LifecycleStoreError::Stale)?;
            }
            // Manual stop is monotonic for this attempt, including a concurrently
            // finishing native startup or kernel-shutdown cleanup.
            sql(tx.execute("UPDATE app_worker_lifecycle SET phase=?1,desired_running=?2,failure=?3,updated_ms=?4 WHERE installation_id=?5 AND owner_id=?6 AND attempt=?7",
                params![phase.name(),i64::from(desired_running&&old.desired_running),failure,now,installation,owner,attempt]))?;
            budget(&limit)?;
            sql(tx.commit())?;
            Ok(Reply::Done)
        }
        Command::Stop {
            owner,
            installation,
            budget: limit,
        } => {
            identity(&owner)?;
            identity(&installation)?;
            budget(&limit)?;
            let tx = sql(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
            let generation:Option<i64>=sql(tx.query_row("SELECT generation FROM app_installations WHERE installation_id=?1 AND owner_id=?2",params![installation,owner],|row|row.get(0)).optional())?;
            let generation = generation.ok_or(LifecycleStoreError::Stale)?;
            budget(&limit)?;
            sql(tx.execute("INSERT INTO app_worker_lifecycle(installation_id,owner_id,generation,attempt,phase,desired_running,failure,updated_ms)
                VALUES(?1,?2,?3,'manual-stop','stopped',0,NULL,?4) ON CONFLICT(installation_id) DO UPDATE SET generation=excluded.generation,desired_running=0,updated_ms=excluded.updated_ms",params![installation,owner,generation,now]))?;
            budget(&limit)?;
            sql(tx.commit())?;
            Ok(Reply::Done)
        }
        Command::FinishStop {
            owner,
            installation,
            budget: limit,
        } => {
            budget(&limit)?;
            let tx = sql(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
            let old = status(&tx, &owner, &installation)?.ok_or(LifecycleStoreError::Stale)?;
            if old.desired_running {
                return Err(LifecycleStoreError::Stale);
            }
            sql(tx.execute("UPDATE app_worker_lifecycle SET phase='stopped',updated_ms=?1 WHERE installation_id=?2 AND owner_id=?3 AND desired_running=0",params![now,installation,owner]))?;
            budget(&limit)?;
            sql(tx.commit())?;
            Ok(Reply::Done)
        }
    }
}
pub(super) fn status(
    connection: &Connection,
    owner: &str,
    installation: &str,
) -> Result<Option<WorkerStatus>> {
    identity(owner)?;
    identity(installation)?;
    type Row = (i64, String, String, bool, Option<String>, i64);
    let value:Option<Row>=sql(connection.query_row("SELECT h.generation,h.attempt,h.phase,h.desired_running,h.failure,h.updated_ms FROM app_worker_lifecycle h
        JOIN app_installations i ON i.installation_id=h.installation_id AND i.owner_id=h.owner_id WHERE h.owner_id=?1 AND h.installation_id=?2",params![owner,installation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional())?;
    value
        .map(
            |(generation, attempt, phase, desired_running, failure, updated)| {
                Ok(WorkerStatus {
                    generation: u64::try_from(generation)
                        .map_err(|_| LifecycleStoreError::Storage)?,
                    attempt,
                    phase: match phase.as_str() {
                        "starting" => WorkerPhase::Starting,
                        "running" => WorkerPhase::Running,
                        "stopped" => WorkerPhase::Stopped,
                        "failed" => WorkerPhase::Failed,
                        _ => return Err(LifecycleStoreError::Storage),
                    },
                    desired_running,
                    failure,
                    updated_ms: u64::try_from(updated).map_err(|_| LifecycleStoreError::Storage)?,
                })
            },
        )
        .transpose()
}
pub(super) fn candidates(
    connection: &Connection,
    after: Option<(&str, &str)>,
) -> Result<Vec<(String, String)>> {
    let mut statement=sql(connection.prepare("SELECT i.owner_id,i.installation_id FROM app_installations i LEFT JOIN app_worker_lifecycle h ON h.installation_id=i.installation_id
        WHERE i.active_json IS NOT NULL AND i.admission_paused=0 AND (h.installation_id IS NULL OR h.generation!=i.generation OR (h.desired_running=1 AND h.phase!='failed'))
        AND (?1 IS NULL OR (i.owner_id,i.installation_id)>(?1,?2)) ORDER BY i.owner_id,i.installation_id LIMIT 8"))?;
    let rows = sql(
        statement.query_map(params![after.map(|v| v.0), after.map(|v| v.1)], |row| {
            Ok((row.get(0)?, row.get(1)?))
        }),
    )?;
    sql(rows.collect())
}
