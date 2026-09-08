use super::{store::*, *};
use chariox_app_runtime::publisher_trust::{enroll_in, PublisherTrustError, TrustDecision};
use rusqlite::{params, Transaction, TransactionBehavior};

fn check(budget: &AppOperationBudget) -> Result<()> {
    budget.check().map_err(|_| PublisherOperationError::Stopped)
}
fn commit(
    tx: Transaction<'_>,
    budget: &AppOperationBudget,
    value: PublisherOperation,
) -> Result<Reply> {
    check(budget)?;
    tx.commit()
        .map_err(|_| PublisherOperationError::CommitUnknown)?;
    Ok(Reply::Operation(value))
}
pub(super) fn execute(connection: &mut Connection, command: Command) -> Result<Reply> {
    match command {
        Command::Begin {
            owner,
            request,
            input,
            budget,
        } => {
            model::text(&owner)?;
            model::text(&request)?;
            input.publisher()?;
            check(&budget)?;
            let tx = sql(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
            match load(&tx, &owner, &request) {
                Ok(old) => {
                    if old.input != input {
                        return Err(PublisherOperationError::Conflict);
                    }
                    return commit(tx, &budget, old);
                }
                Err(PublisherOperationError::NotFound) => {}
                Err(error) => return Err(error),
            }
            admit(&tx, &owner)?;
            let time = now()?;
            sql(tx.execute("INSERT INTO app_publisher_operations(owner_id,request_id,session_id,publisher_id,key_id,public_key,expected_revision,phase,created_ms,updated_ms)
                VALUES(?1,?2,?3,?4,?5,?6,?7,'pending',?8,?8)",params![owner,request,input.session_id,input.publisher_id,input.key_id,&input.public_key[..],input.expected_revision as i64,time]))?;
            let value = load(&tx, &owner, &request)?;
            commit(tx, &budget, value)
        }
        Command::Arm {
            owner,
            request,
            interaction,
            deadline,
            budget,
        } => {
            model::text(&interaction)?;
            check(&budget)?;
            if deadline <= Instant::now()
                || deadline > Instant::now() + std::time::Duration::from_secs(600)
            {
                return Err(PublisherOperationError::Stopped);
            }
            let tx = sql(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
            let current = load(&tx, &owner, &request)?;
            if current.phase != PublisherOperationPhase::Pending {
                check(&budget)?;
                tx.commit()
                    .map_err(|_| PublisherOperationError::CommitUnknown)?;
                return Ok(Reply::Review(PublisherReview::Terminal(current)));
            }
            fresh_nonce(&tx, &owner, &interaction)?;
            sql(tx.execute("UPDATE app_publisher_operations SET interaction_id=?1,updated_ms=?2 WHERE owner_id=?3 AND request_id=?4",params![interaction,now()?,owner,request]))?;
            check(&budget)?;
            if Instant::now() >= deadline {
                return Err(PublisherOperationError::Stopped);
            }
            tx.commit()
                .map_err(|_| PublisherOperationError::CommitUnknown)?;
            Ok(Reply::Review(PublisherReview::Prompt(
                PublisherApprovalChallenge {
                    owner,
                    request,
                    interaction,
                    input: current.input,
                    deadline,
                },
            )))
        }
        Command::Decide {
            challenge,
            accepted,
            budget,
        } => {
            let deadline = challenge.deadline;
            let budget = budget.fork(move || Instant::now() >= deadline);
            check(&budget)?;
            let tx = sql(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
            let current = load(&tx, &challenge.owner, &challenge.request)?;
            if current.phase != PublisherOperationPhase::Pending
                || current.input != challenge.input
                || current.interaction_id.as_deref() != Some(challenge.interaction.as_str())
            {
                return Err(PublisherOperationError::Conflict);
            }
            let mut phase = "denied";
            let mut receipt = None;
            let mut failure = None;
            if accepted {
                // This operation has not committed a decision yet. A receipt
                // minted elsewhere with this nonce is never reusable consent.
                if used_decision(&tx, &challenge.owner, &challenge.interaction)? {
                    return Err(PublisherOperationError::Conflict);
                }
                let publisher = challenge.input.publisher()?;
                let decision = TrustDecision {
                    decision_id: challenge.interaction.clone(),
                    authority_ref: "kernel_operation_human".into(),
                };
                match enroll_in(
                    &tx,
                    &challenge.owner,
                    &publisher,
                    challenge.input.expected_revision,
                    &decision,
                    now()? as u64,
                ) {
                    Ok(value) => {
                        phase = "approved";
                        receipt = Some(value)
                    }
                    Err(PublisherTrustError::Conflict) => {
                        phase = "failed";
                        failure = Some("publisher_changed")
                    }
                    Err(PublisherTrustError::Limit) => {
                        phase = "failed";
                        failure = Some("publisher_limit")
                    }
                    Err(_) => return Err(PublisherOperationError::Storage),
                }
            }
            sql(tx.execute("UPDATE app_publisher_operations SET phase=?1,receipt_revision=?2,receipt_ms=?3,failure=?4,updated_ms=?5 WHERE owner_id=?6 AND request_id=?7",
                params![phase,receipt.as_ref().map(|r|r.revision as i64),receipt.as_ref().map(|r|r.applied_at_ms as i64),failure,now()?,challenge.owner,challenge.request]))?;
            let value = load(&tx, &challenge.owner, &challenge.request)?;
            commit(tx, &budget, value)
        }
        Command::Cancel {
            owner,
            request,
            budget,
        } => {
            check(&budget)?;
            let tx = sql(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
            let current = load(&tx, &owner, &request)?;
            if current.phase == PublisherOperationPhase::Approved {
                return Err(PublisherOperationError::Conflict);
            }
            if current.phase == PublisherOperationPhase::Pending {
                sql(tx.execute("UPDATE app_publisher_operations SET phase='cancelled',interaction_id=NULL,updated_ms=?1 WHERE owner_id=?2 AND request_id=?3",params![now()?,owner,request]))?;
            }
            let value = load(&tx, &owner, &request)?;
            commit(tx, &budget, value)
        }
        Command::Status {
            owner,
            request,
            budget,
        } => {
            // Barrier settles a lost reply after a successful commit. A SQLite
            // COMMIT error stops the writer; an empty transaction cannot repair
            // that durability uncertainty and must never run past its fence.
            check(&budget)?;
            let tx = sql(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
            let value = load(&tx, &owner, &request)?;
            commit(tx, &budget, value)
        }
    }
}
