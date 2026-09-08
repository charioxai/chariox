use super::{store::*, *};
use chariox_app_runtime::{
    installation::InstallationRegistry, publisher_trust::PublisherTrustRegistry,
};
use rusqlite::{params, TransactionBehavior};

pub(super) fn apply(connection: &mut Connection, command: Command) -> Result<Reply> {
    match command {
        Command::Replay {
            owner,
            request_id,
            digest,
            budget,
        } => {
            limit(&budget)?;
            let tx = sql(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
            let current = load(&tx, &owner, &request_id)?.ok_or(InstallOperationError::NotFound)?;
            if current.package_digest != digest {
                return Err(InstallOperationError::Conflict);
            }
            sql(tx.execute("UPDATE app_installation_operations SET updated_ms=?1 WHERE owner_id=?2 AND request_id=?3",params![now()?,owner,request_id]))?;
            limit(&budget)?;
            tx.commit()
                .map_err(|_| InstallOperationError::CommitUnknown)?;
            Ok(Reply::Operation(current))
        }
        Command::Cancel {
            owner,
            request_id,
            budget,
        } => {
            limit(&budget)?;
            let tx = sql(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
            let current = load(&tx, &owner, &request_id)?.ok_or(InstallOperationError::NotFound)?;
            if current.phase == InstallPhase::Committed {
                return Err(InstallOperationError::Conflict);
            }
            if matches!(
                current.phase,
                InstallPhase::Cancelled | InstallPhase::Failed
            ) {
                return Ok(Reply::Operation(current));
            }
            let binding = StageTrustBinding::staged_in(&tx, &owner, &current.token)
                .map_err(|_| InstallOperationError::Stale)?;
            let time = now()?;
            binding
                .abort_first_in(&tx, &owner, "app_install_cancelled", time as u64)
                .map_err(|_| InstallOperationError::Conflict)?;
            sql(tx.execute("UPDATE app_installation_operations SET phase='cancelled',failure='app_install_cancelled',cleanup_pending=1,updated_ms=?1
                WHERE owner_id=?2 AND request_id=?3",params![time,owner,request_id]))?;
            let result = load(&tx, &owner, &request_id)?.ok_or(InstallOperationError::Storage)?;
            limit(&budget)?;
            tx.commit()
                .map_err(|_| InstallOperationError::CommitUnknown)?;
            Ok(Reply::Operation(result))
        }
        Command::Begin {
            owner,
            request_id,
            candidate,
            budget,
        } => {
            identity(&owner)?;
            identity(&request_id)?;
            limit(&budget)?;
            let tx = sql(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
            limit(&budget)?;
            if let Some(old) = load(&tx, &owner, &request_id)? {
                if old.package_digest != candidate.release_metadata().package_digest {
                    return Err(InstallOperationError::Conflict);
                }
                // Historical replay does not approve or reactivate anything.
                tx.commit()
                    .map_err(|_| InstallOperationError::CommitUnknown)?;
                return Ok(Reply::Operation(old));
            }
            admit(&tx, &owner)?;
            let installation = format!("app_{:032x}", rand::random::<u128>());
            let time = now()?;
            candidate
                .stage_first_in(&tx, &owner, &installation, time as u64)
                .map_err(|_| InstallOperationError::Stale)?;
            sql(tx.execute("INSERT INTO app_installation_operations(owner_id,request_id,installation_id,package_digest,phase,created_ms,updated_ms)
                VALUES(?1,?2,?3,?4,'approval',?5,?5)", params![owner,request_id,installation,candidate.release_metadata().package_digest,time]))?;
            let result = load(&tx, &owner, &request_id)?.ok_or(InstallOperationError::Storage)?;
            limit(&budget)?;
            tx.commit()
                .map_err(|_| InstallOperationError::CommitUnknown)?;
            Ok(Reply::Operation(result))
        }
        Command::Claim {
            owner,
            request_id,
            attempt,
            budget,
        } => {
            identity(&attempt)?;
            limit(&budget)?;
            let old =
                load(connection, &owner, &request_id)?.ok_or(InstallOperationError::NotFound)?;
            let binding = InstallationRegistry::new(connection)
                .staged_trust(&owner, &old.token)
                .map_err(|_| InstallOperationError::Stale)?;
            let trust = PublisherTrustRegistry::new(connection)
                .trusted_publisher(&owner, binding.publisher_id(), binding.key_id())
                .map_err(|_| InstallOperationError::Stale)?;
            let tx = sql(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
            limit(&budget)?;
            let current = load(&tx, &owner, &request_id)?.ok_or(InstallOperationError::NotFound)?;
            if !matches!(
                current.phase,
                InstallPhase::AwaitingApproval | InstallPhase::Starting
            ) || current != old
            {
                return Err(InstallOperationError::Conflict);
            }
            let time = now()?;
            let approval = binding
                .quiesce_first_in(&tx, &owner, &trust, time as u64)
                .map_err(|error| match error {
                    chariox_app_runtime::installation::VerifiedStageError::Installation(
                        chariox_app_runtime::installation::InstallationError::ApprovalRequired,
                    ) => InstallOperationError::ApprovalRequired,
                    _ => InstallOperationError::Stale,
                })?;
            if current.phase == InstallPhase::Starting
                && store::approval(&tx, &owner, &request_id)?.as_ref() != Some(&approval)
            {
                return Err(InstallOperationError::Stale);
            }
            let encoded =
                serde_json::to_string(&approval).map_err(|_| InstallOperationError::Storage)?;
            sql(tx.execute("UPDATE app_installation_operations SET phase='starting',attempt=?1,approval_json=?2,updated_ms=?3
                WHERE owner_id=?4 AND request_id=?5", params![attempt,encoded,time,owner,request_id]))?;
            limit(&budget)?;
            tx.commit()
                .map_err(|_| InstallOperationError::CommitUnknown)?;
            Ok(Reply::Approved(ApprovedFirstInstall {
                owner,
                request_id,
                attempt,
                binding,
                trust,
                approval,
            }))
        }
        Command::Verify { admission, budget } => {
            limit(&budget)?;
            let tx = sql(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
            require_start(&tx, &admission)?;
            limit(&budget)?;
            tx.commit()
                .map_err(|_| InstallOperationError::CommitUnknown)?;
            Ok(Reply::Done)
        }
        Command::Commit {
            admission,
            health,
            budget,
            #[cfg(test)]
            fault,
        } => super::commit::commit(
            connection,
            &admission,
            &health,
            &budget,
            #[cfg(test)]
            fault,
        )
        .map(Reply::Committed),
        Command::Finish {
            admission,
            cancelled,
            failure,
            budget,
        } => {
            limit(&budget)?;
            if failure.is_empty()
                || failure.len() > 96
                || !failure
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            {
                return Err(InstallOperationError::Invalid);
            }
            let tx = sql(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
            let current = load(&tx, &admission.owner, &admission.request_id)?
                .ok_or(InstallOperationError::NotFound)?;
            if current.phase == InstallPhase::Committed {
                // Startup or publication failure is now ordinary active-worker
                // recovery. Never erase committed private writes or approval.
                return Ok(Reply::Done);
            }
            if matches!(
                current.phase,
                InstallPhase::Failed | InstallPhase::Cancelled
            ) && current.attempt.as_deref() == Some(admission.attempt.as_str())
            {
                return Ok(Reply::Done);
            }
            if current.phase != InstallPhase::Starting
                || current.attempt.as_deref() != Some(admission.attempt.as_str())
            {
                return Err(InstallOperationError::Conflict);
            }
            let time = now()?;
            admission
                .binding
                .abort_first_in(&tx, &admission.owner, &failure, time as u64)
                .map_err(|_| InstallOperationError::Conflict)?;
            sql(tx.execute("UPDATE app_installation_operations SET phase=?1,failure=?2,cleanup_pending=1,updated_ms=?3
                WHERE owner_id=?4 AND request_id=?5", params![if cancelled {"cancelled"} else {"failed"},failure,time,admission.owner,admission.request_id]))?;
            limit(&budget)?;
            tx.commit()
                .map_err(|_| InstallOperationError::CommitUnknown)?;
            Ok(Reply::Done)
        }
    }
}
