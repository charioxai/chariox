use super::*;
use rusqlite::{params, OptionalExtension, Transaction};

pub(super) const OWNER_RECEIPTS: i64 = 4096;
pub(super) const TOTAL_RECEIPTS: i64 = 16384;
pub(super) fn sql<T>(value: rusqlite::Result<T>) -> Result<T> {
    value.map_err(|_| InstallOperationError::Storage)
}
pub(super) fn identity(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        Err(InstallOperationError::Invalid)
    } else {
        Ok(())
    }
}
pub(super) fn now() -> Result<i64> {
    i64::try_from(crate::session::unix_epoch_ms()).map_err(|_| InstallOperationError::Storage)
}
pub(super) fn limit(value: &AppOperationBudget) -> Result<()> {
    value.check().map_err(|_| InstallOperationError::Stopped)
}
pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS app_installation_operations (
        owner_id TEXT NOT NULL, request_id TEXT NOT NULL, installation_id TEXT NOT NULL UNIQUE,
        package_digest TEXT NOT NULL, phase TEXT NOT NULL CHECK(phase IN ('approval','starting','committed','cancelled','failed')),
        attempt TEXT, approval_json TEXT, failure TEXT,
        cleanup_pending INTEGER NOT NULL DEFAULT 0 CHECK(cleanup_pending IN (0,1)),
        created_ms INTEGER NOT NULL CHECK(created_ms>=0), updated_ms INTEGER NOT NULL CHECK(updated_ms>=0),
        PRIMARY KEY(owner_id,request_id));")
}
pub(super) fn load(
    connection: &Connection,
    owner: &str,
    request: &str,
) -> Result<Option<InstallOperation>> {
    identity(owner)?;
    identity(request)?;
    type Row = (String, String, String, Option<String>, Option<String>, bool);
    let row: Option<Row> = sql(connection
        .query_row(
            "SELECT installation_id,package_digest,phase,attempt,failure,cleanup_pending
         FROM app_installation_operations WHERE owner_id=?1 AND request_id=?2",
            params![owner, request],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional())?;
    row.map(
        |(installation, package_digest, phase, attempt, failure, cleanup_pending)| {
            Ok(InstallOperation {
                request_id: request.into(),
                token: StageToken {
                    installation_id: installation,
                    base_generation: 0,
                    generation: 1,
                },
                package_digest,
                phase: match phase.as_str() {
                    "approval" => InstallPhase::AwaitingApproval,
                    "starting" => InstallPhase::Starting,
                    "committed" => InstallPhase::Committed,
                    "cancelled" => InstallPhase::Cancelled,
                    "failed" => InstallPhase::Failed,
                    _ => return Err(InstallOperationError::Storage),
                },
                attempt,
                failure,
                cleanup_pending,
            })
        },
    )
    .transpose()
}
pub(super) fn require_start(
    tx: &Transaction<'_>,
    admission: &ApprovedFirstInstall,
) -> Result<InstallOperation> {
    let operation = load(tx, &admission.owner, &admission.request_id)?
        .ok_or(InstallOperationError::NotFound)?;
    if operation.phase != InstallPhase::Starting
        || operation.attempt.as_deref() != Some(admission.attempt.as_str())
        || operation.token != *admission.binding.token()
        || operation.package_digest != admission.binding.package_digest()
    {
        return Err(InstallOperationError::Conflict);
    }
    let recorded = approval(tx, &admission.owner, &admission.request_id)?;
    if recorded.as_ref() != Some(&admission.approval)
        || admission
            .binding
            .require_first_approved_in(tx, &admission.owner, &admission.trust)
            .map_err(|_| InstallOperationError::Stale)?
            != admission.approval
    {
        return Err(InstallOperationError::Stale);
    }
    Ok(operation)
}
pub(super) fn approval(
    connection: &Connection,
    owner: &str,
    request: &str,
) -> Result<Option<CapabilityApproval>> {
    let json: Option<String> = sql(connection.query_row(
        "SELECT approval_json FROM app_installation_operations WHERE owner_id=?1 AND request_id=?2",
        params![owner, request],
        |r| r.get(0),
    ))?;
    json.map(|value| serde_json::from_str(&value).map_err(|_| InstallOperationError::Storage))
        .transpose()
}
pub(super) fn admit(tx: &Transaction<'_>, owner: &str) -> Result<()> {
    // Without timestamp-bound request IDs, deleting receipts could resurrect a
    // cancelled install on delayed replay. Retain identities and backpressure.
    let (total, owned, pending, owned_pending): (i64, i64, i64, i64) = sql(tx.query_row(
        "SELECT count(*),coalesce(sum(owner_id=?1),0),
         coalesce(sum(phase IN ('approval','starting')),0),
         coalesce(sum(owner_id=?1 AND phase IN ('approval','starting')),0)
         FROM app_installation_operations",
        [owner],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    ))?;
    if total >= TOTAL_RECEIPTS || owned >= OWNER_RECEIPTS || pending >= 32 || owned_pending >= 8 {
        return Err(InstallOperationError::Limit);
    }
    Ok(())
}
pub(super) fn candidates(
    connection: &Connection,
    after: Option<(&str, &str)>,
) -> Result<Vec<(String, String)>> {
    let mut statement = sql(connection.prepare("SELECT o.owner_id,o.request_id FROM app_installation_operations o
        JOIN app_installation_updates u ON u.installation_id=o.installation_id AND u.generation=1
        WHERE o.phase IN ('approval','starting') AND json_extract(u.record_json,'$.decision.status')='approved'
        AND (?1 IS NULL OR (o.owner_id,o.request_id)>(?1,?2)) ORDER BY o.owner_id,o.request_id LIMIT 8"))?;
    let rows = sql(
        statement.query_map(params![after.map(|v| v.0), after.map(|v| v.1)], |r| {
            Ok((r.get(0)?, r.get(1)?))
        }),
    )?;
    sql(rows.collect())
}
