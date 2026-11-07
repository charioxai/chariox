//! A remote attach commits locally first and installs on the source kernel second. The
//! subscription row it replaced is kept with it, so a failed install puts back exactly the
//! prior state, and an install nobody confirmed is reconciled on the source instead of guessed.
use super::*;

/// The source kernel is asked again this long after an unconfirmed install or detach.
const RECONCILE_RETRY_MS: i64 = 30_000;
/// A reconciliation the source never matched is parked after this many asks (about ten minutes)
/// and asked again only when the source kernel connects again.
const RECONCILE_MAX_ATTEMPTS: i64 = 20;
/// The next attempt of a parked reconciliation: no clock reaches it.
const PARKED: i64 = i64::MAX;
/// An install still unsettled this long after it began lost its task (a crash): it is settled
/// as unconfirmed. Far above the peer request timeout.
const INSTALL_SETTLE_AFTER_MS: i64 = 60_000;

/// Why a reconciliation entry stopped being asked without the source matching it.
#[derive(Debug)]
enum ReconcileParked {
    AttemptsExhausted,
}

/// One unconfirmed remote install or detach the source kernel must be brought in line with.
#[derive(Debug)]
pub(crate) struct RemoteReconcile {
    pub owner: String,
    pub subscription_id: String,
    pub source_kernel_id: String,
    /// The subscription the source must hold; `None` when it must hold none.
    pub desired: Option<WorkflowNotificationSubscription>,
}

/// Runs inside the local attach commit, before the row is replaced.
pub(super) fn begin_in(
    tx: &Transaction<'_>,
    sub: &WorkflowNotificationSubscription,
) -> Result<(), DaemonError> {
    let prior:Option<(String,String,String)>=tx.query_row("SELECT automation_id,status,notification_json FROM app_automations WHERE source_kind='workflow_completion' AND owner_id=?1 AND installation_id=?2 AND json_extract(notification_json,'$.target_kernel_id')=?3 AND session_id=?4 AND publication_id=?5",params![sub.owner_user_id,sub.source_id,sub.target_kernel_id,sub.session_id,sub.publication_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(sql)?;
    let (id, status, json, grant) = match prior {
        Some((id, status, json)) => {
            let grant: Option<String> = tx.query_row("SELECT grant_id FROM workflow_notification_grants WHERE owner_id=?1 AND subscription_id=?2",params![sub.owner_user_id,id],|r|r.get(0)).optional().map_err(sql)?;
            (Some(id), Some(status), Some(json), grant)
        }
        None => (None, None, None, None),
    };
    // The row is keyed by the id the save will settle on: the reused one, or the new one.
    let id = id.unwrap_or_else(|| sub.subscription_id.clone());
    tx.execute("INSERT OR REPLACE INTO workflow_notification_remote_installs VALUES (?1,?2,?3,'installing',?4,?5,?6,NULL,CAST(strftime('%s','now') AS INTEGER)*1000,0)",params![sub.owner_user_id,id,sub.source_kernel_id,status,json,grant]).map_err(sql)?;
    Ok(())
}

/// The source answered with the exact subscription: nothing is left to undo or reconcile.
pub(super) fn confirm_in(
    tx: &Transaction<'_>,
    owner: &str,
    subscription_id: &str,
) -> Result<(), DaemonError> {
    tx.execute("DELETE FROM workflow_notification_remote_installs WHERE owner_id=?1 AND subscription_id=?2",params![owner,subscription_id]).map_err(sql)?;
    Ok(())
}

/// The install was refused or went unanswered: the row goes back to what it was before the
/// attach. The source may still have installed it, so it is asked to match the restored state.
pub(super) fn undo_in(
    tx: &Transaction<'_>,
    owner: &str,
    subscription_id: &str,
) -> Result<(), DaemonError> {
    let held:Option<(Option<String>,Option<String>,Option<String>)>=tx.query_row("SELECT prior_status,prior_json,prior_grant FROM workflow_notification_remote_installs WHERE owner_id=?1 AND subscription_id=?2 AND state='installing'",params![owner,subscription_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(sql)?;
    let Some((status, json, grant)) = held else {
        return Ok(());
    };
    let attempted:Option<String>=tx.query_row("SELECT notification_json FROM app_automations WHERE source_kind='workflow_completion' AND owner_id=?1 AND automation_id=?2",params![owner,subscription_id],|r|r.get(0)).optional().map_err(sql)?;
    match (&status, &json) {
        (Some(status), Some(json)) => {
            tx.execute("UPDATE app_automations SET status=?3,notification_json=?4 WHERE source_kind='workflow_completion' AND owner_id=?1 AND automation_id=?2",params![owner,subscription_id,status,json]).map_err(sql)?;
        }
        _ => {
            tx.execute("UPDATE app_automations SET status='disabled' WHERE source_kind='workflow_completion' AND owner_id=?1 AND automation_id=?2",params![owner,subscription_id]).map_err(sql)?;
        }
    }
    tx.execute(
        "DELETE FROM workflow_notification_grants WHERE owner_id=?1 AND subscription_id=?2",
        params![owner, subscription_id],
    )
    .map_err(sql)?;
    if let Some(grant) = grant {
        tx.execute(
            "INSERT INTO workflow_notification_grants VALUES (?1,?2,?3)",
            params![owner, subscription_id, grant],
        )
        .map_err(sql)?;
    }
    let desired = match (&status, &json) {
        (Some(status), Some(json)) if status == "active" => Some(json.clone()),
        _ => None,
    };
    if desired.is_some() && desired == attempted {
        // The source holds the same subscription whether or not it installed the repeat.
        return confirm_in(tx, owner, subscription_id);
    }
    tx.execute("UPDATE workflow_notification_remote_installs SET state='reconcile',desired_json=?3,next_attempt_at_ms=0,attempts=0 WHERE owner_id=?1 AND subscription_id=?2",params![owner,subscription_id,desired]).map_err(sql)?;
    Ok(())
}

/// A detach ends whatever install or reconciliation was still pending for the subscription, so
/// neither a late settlement nor a retry can bring the follow back.
pub(super) fn supersede_in(
    tx: &Transaction<'_>,
    owner: &str,
    subscription_id: &str,
) -> Result<(), DaemonError> {
    tx.execute("DELETE FROM workflow_notification_remote_installs WHERE owner_id=?1 AND subscription_id=?2",params![owner,subscription_id]).map_err(sql)?;
    Ok(())
}

/// A detach the source kernel did not confirm leaves a row there that must still go.
pub(super) fn mark_detach_in(
    tx: &Transaction<'_>,
    owner: &str,
    subscription_id: &str,
    source_kernel: &str,
) -> Result<(), DaemonError> {
    tx.execute("INSERT OR REPLACE INTO workflow_notification_remote_installs VALUES (?1,?2,?3,'reconcile',NULL,NULL,NULL,NULL,0,0)",params![owner,subscription_id,source_kernel]).map_err(sql)?;
    Ok(())
}

/// Hands out the due reconciliations and pushes their next attempt back, so an unreachable
/// source is asked again later and never blocks the others.
pub(super) fn claim_in(
    tx: &Transaction<'_>,
    now: u64,
    limit: usize,
) -> Result<Vec<RemoteReconcile>, DaemonError> {
    let rows = {
        let mut q = tx.prepare("SELECT owner_id,subscription_id,source_kernel_id,desired_json,attempts FROM workflow_notification_remote_installs WHERE state='reconcile' AND next_attempt_at_ms<=?1 AND next_attempt_at_ms<?3 ORDER BY next_attempt_at_ms,subscription_id LIMIT ?2").map_err(sql)?;
        let rows = q
            .query_map(params![now as i64, limit as i64, PARKED], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            })
            .map_err(sql)?;
        rows.collect::<Result<Vec<(String, String, String, Option<String>, i64)>, _>>()
            .map_err(sql)?
    };
    let mut due = Vec::new();
    for (owner, subscription_id, source_kernel_id, desired, attempts) in rows {
        if attempts >= RECONCILE_MAX_ATTEMPTS {
            tx.execute("UPDATE workflow_notification_remote_installs SET next_attempt_at_ms=?3 WHERE owner_id=?1 AND subscription_id=?2 AND state='reconcile'",params![owner,subscription_id,PARKED]).map_err(sql)?;
            crate::logging::warn_with_fields(
                "workflow.notifications",
                "remote notification reconciliation parked",
                serde_json::json!({
                    "reason": format!("{:?}", ReconcileParked::AttemptsExhausted),
                    "subscription_id": subscription_id,
                    "source_kernel_id": source_kernel_id,
                }),
            );
            continue;
        }
        tx.execute("UPDATE workflow_notification_remote_installs SET next_attempt_at_ms=?3,attempts=attempts+1 WHERE owner_id=?1 AND subscription_id=?2",params![owner,subscription_id,(now as i64).saturating_add(RECONCILE_RETRY_MS)]).map_err(sql)?;
        due.push(RemoteReconcile {
            desired: desired.as_deref().map(decode).transpose()?,
            owner,
            subscription_id,
            source_kernel_id,
        });
    }
    Ok(due)
}

/// The source matched the desired state: only a reconciliation entry is dropped, never an
/// install that a newer attach has put in flight.
pub(super) fn reconciled_in(
    tx: &Transaction<'_>,
    owner: &str,
    subscription_id: &str,
) -> Result<(), DaemonError> {
    tx.execute("DELETE FROM workflow_notification_remote_installs WHERE owner_id=?1 AND subscription_id=?2 AND state='reconcile'",params![owner,subscription_id]).map_err(sql)?;
    Ok(())
}

/// The source kernels connected again: their unreconciled entries are due now, with a fresh
/// set of attempts. Only entries of those kernels change, so one connection asks at most once
/// per entry per window.
pub(super) fn connected_in(tx: &Transaction<'_>, kernels: &[String]) -> Result<(), DaemonError> {
    for kernel in kernels {
        tx.execute("UPDATE workflow_notification_remote_installs SET next_attempt_at_ms=0,attempts=0 WHERE state='reconcile' AND source_kernel_id=?1",params![kernel]).map_err(sql)?;
    }
    Ok(())
}

/// Installs that began long ago and were never settled (the kernel stopped between the local
/// commit and the source's answer) are settled as unconfirmed: the prior row comes back and the
/// source is reconciled.
pub(super) fn settle_interrupted_in(tx: &Transaction<'_>, now: u64) -> Result<(), DaemonError> {
    let stale = {
        let mut q = tx.prepare("SELECT owner_id,subscription_id FROM workflow_notification_remote_installs WHERE state='installing' AND next_attempt_at_ms<=?1").map_err(sql)?;
        let rows = q
            .query_map(
                params![(now as i64).saturating_sub(INSTALL_SETTLE_AFTER_MS)],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .map_err(sql)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(sql)?
    };
    for (owner, subscription_id) in stale {
        undo_in(tx, &owner, &subscription_id)?;
    }
    Ok(())
}

#[cfg(test)]
impl DurableKernelStateStore {
    /// Unreconciled entries, read without claiming them.
    pub(crate) fn unconfirmed_installs(&self) -> i64 {
        self.lock_connection("test")
            .unwrap()
            .query_row("SELECT count(*) FROM workflow_notification_remote_installs WHERE state='reconcile'",[],|r|r.get(0))
            .unwrap()
    }
    /// Detaches no source kernel has confirmed yet, read without claiming them.
    pub(crate) fn unconfirmed_detaches(&self) -> i64 {
        self.lock_connection("test")
            .unwrap()
            .query_row("SELECT count(*) FROM workflow_notification_remote_installs WHERE state='reconcile' AND desired_json IS NULL",[],|r|r.get(0))
            .unwrap()
    }
}
