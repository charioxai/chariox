//! Activation and receipt durability, including unknown-COMMIT recovery.
use super::{store::*, *};
use rusqlite::{params, TransactionBehavior};

pub(super) fn commit(
    connection: &mut Connection,
    admission: &ApprovedFirstInstall,
    health: &FirstInstallHealth,
    budget: &AppOperationBudget,
    #[cfg(test)] fault: Option<CommitTestFault>,
) -> Result<FirstInstallCommitted> {
    limit(budget)?;
    limit(health.budget())?;
    let catalog = health.catalog().app_catalog();
    if catalog.installation_id() != admission.binding.token().installation_id
        || catalog.generation() != admission.binding.token().generation
        || catalog.package_digest() != admission.binding.package_digest()
    {
        return Err(InstallOperationError::Conflict);
    }
    let tx = sql(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
    require_start(&tx, admission)?;
    limit(budget)?;
    limit(health.budget())?;
    let time = now()?;
    admission
        .binding
        .commit_first_in(
            &tx,
            &admission.owner,
            &admission.trust,
            &admission.approval,
            time as u64,
        )
        .map_err(|_| InstallOperationError::Stale)?;
    let committed = proofs(&tx, admission, health)?;
    sql(tx.execute("UPDATE app_installation_operations SET phase='committed',updated_ms=?1 WHERE owner_id=?2 AND request_id=?3",
        params![time,admission.owner,admission.request_id]))?;
    limit(budget)?;
    limit(health.budget())?;
    match tx.commit() {
        Ok(()) => {
            #[cfg(test)]
            if let Some(fault) = fault {
                if fault.fail_reconciliation {
                    return Err(InstallOperationError::CommitUnknown);
                }
                return reconcile(connection, admission, health);
            }
            Ok(committed)
        }
        Err(_) => reconcile(connection, admission, health),
    }
}
fn proofs(
    tx: &rusqlite::Transaction<'_>,
    admission: &ApprovedFirstInstall,
    health: &FirstInstallHealth,
) -> Result<FirstInstallCommitted> {
    let active = ActiveStartAdmission::first_committed_in(
        tx,
        &admission.owner,
        &admission.attempt,
        &admission.binding,
        &admission.trust,
    )
    .map_err(|_| InstallOperationError::Stale)?;
    let activation =
        CommittedAppActivation::first_committed_in(tx, &admission.owner, health.catalog().clone())
            .map_err(|_| InstallOperationError::Stale)?;
    Ok(FirstInstallCommitted { activation, active })
}
fn reconcile(
    connection: &mut Connection,
    admission: &ApprovedFirstInstall,
    health: &FirstInstallHealth,
) -> Result<FirstInstallCommitted> {
    // A visible read is not durability evidence. Reassert the exact completed
    // receipt/health in a second FULL-synchronous writer transaction before ACK.
    let mut attempt = || -> Result<Option<FirstInstallCommitted>> {
        let tx = sql(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
        let current = load(&tx, &admission.owner, &admission.request_id)?
            .ok_or(InstallOperationError::CommitUnknown)?;
        if current.phase == InstallPhase::Starting {
            require_start(&tx, admission)?;
            tx.commit()
                .map_err(|_| InstallOperationError::CommitUnknown)?;
            return Ok(None);
        }
        if current.phase != InstallPhase::Committed
            || current.attempt.as_deref() != Some(admission.attempt.as_str())
            || current.token != *admission.binding.token()
            || current.package_digest != admission.binding.package_digest()
            || approval(&tx, &admission.owner, &admission.request_id)?.as_ref()
                != Some(&admission.approval)
        {
            return Err(InstallOperationError::CommitUnknown);
        }
        let committed = proofs(&tx, admission, health)?;
        sql(tx.execute("UPDATE app_installation_operations SET updated_ms=?1 WHERE owner_id=?2 AND request_id=?3",
            params![now()?,admission.owner,admission.request_id]))?;
        tx.commit()
            .map_err(|_| InstallOperationError::CommitUnknown)?;
        Ok(Some(committed))
    };
    match attempt() {
        Ok(Some(committed)) => Ok(committed),
        Ok(None) => Err(InstallOperationError::Storage),
        Err(_) => Err(InstallOperationError::CommitUnknown),
    }
}
