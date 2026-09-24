//! Durable ownership before slow preparation and exact human decision fencing.
#[cfg(test)]
mod tests;
use super::{store::*, *};
use chariox_app_runtime::{
    installation::{CapabilityDecision, InstallationRegistry},
    publisher_trust::PublisherTrustRegistry,
};
use rusqlite::{params, TransactionBehavior};

pub(crate) struct InstallApprovalChallenge {
    owner: String,
    request_id: String,
    interaction_id: String,
    deadline: std::time::Instant,
    binding: StageTrustBinding,
    trust: TrustedPublisherSnapshot,
    capabilities_digest: String,
    review: serde_json::Value,
}
impl InstallApprovalChallenge {
    pub(crate) fn installation_id(&self) -> &str {
        &self.binding.token().installation_id
    }
    pub(crate) fn interaction_id(&self) -> &str {
        &self.interaction_id
    }
    pub(crate) fn deadline(&self) -> std::time::Instant {
        self.deadline
    }
    pub(crate) fn review(&self) -> &serde_json::Value {
        &self.review
    }
}
pub(crate) enum InstallReviewDisposition {
    Prompt(InstallApprovalChallenge),
    Approved,
    Terminal,
}
pub(super) enum PublicCommand {
    Reserve {
        owner: String,
        request_id: String,
        input: InstallInput,
        digest: String,
        budget: AppOperationBudget,
    },
    Prepared {
        owner: String,
        request_id: String,
        candidate: VerifiedInstallCandidate,
        budget: AppOperationBudget,
    },
    Arm {
        owner: String,
        request_id: String,
        interaction_id: String,
        budget: AppOperationBudget,
    },
    Decide {
        challenge: Arc<InstallApprovalChallenge>,
        accepted: bool,
        budget: AppOperationBudget,
    },
    Fail {
        owner: String,
        request_id: String,
        code: String,
        budget: AppOperationBudget,
    },
}
impl DurableKernelStateStore {
    pub(crate) fn replay_public_app_install(
        &self,
        owner: &str,
        request_id: &str,
        budget: AppOperationBudget,
    ) -> Result<InstallOperation> {
        let prior = self.first_app_install_status(owner, request_id)?;
        self.replay_first_app_install(owner, request_id, &prior.package_digest, budget)
    }

    pub(crate) fn reserve_app_install(
        &self,
        owner: &str,
        request_id: &str,
        input: InstallInput,
        digest: &str,
        budget: AppOperationBudget,
    ) -> Result<InstallOperation> {
        self.public_install(PublicCommand::Reserve {
            owner: owner.into(),
            request_id: request_id.into(),
            input,
            digest: digest.into(),
            budget,
        })
    }
    pub(crate) fn complete_app_install_preparation(
        &self,
        owner: &str,
        request_id: &str,
        candidate: VerifiedInstallCandidate,
        budget: AppOperationBudget,
    ) -> Result<InstallOperation> {
        self.public_install(PublicCommand::Prepared {
            owner: owner.into(),
            request_id: request_id.into(),
            candidate,
            budget,
        })
    }
    pub(crate) fn arm_app_install_review(
        &self,
        owner: &str,
        request_id: &str,
        interaction_id: &str,
        budget: AppOperationBudget,
    ) -> Result<InstallReviewDisposition> {
        match self.first_install(Command::Public(PublicCommand::Arm {
            owner: owner.into(),
            request_id: request_id.into(),
            interaction_id: interaction_id.into(),
            budget,
        }))? {
            Reply::Review(value) => Ok(value),
            _ => Err(InstallOperationError::Storage),
        }
    }
    /// Caller consumes the authenticated generic interaction result. The budget
    /// retains its ORIGINAL interaction deadline and cancellation predicate.
    pub(crate) fn decide_app_install(
        &self,
        challenge: Arc<InstallApprovalChallenge>,
        accepted: bool,
        budget: AppOperationBudget,
    ) -> Result<InstallOperation> {
        self.public_install(PublicCommand::Decide {
            challenge,
            accepted,
            budget,
        })
    }
    pub(crate) fn fail_app_install(
        &self,
        owner: &str,
        request_id: &str,
        code: &str,
        budget: AppOperationBudget,
    ) -> Result<InstallOperation> {
        self.public_install(PublicCommand::Fail {
            owner: owner.into(),
            request_id: request_id.into(),
            code: code.into(),
            budget,
        })
    }
    fn public_install(&self, command: PublicCommand) -> Result<InstallOperation> {
        match self.first_install(Command::Public(command))? {
            Reply::Operation(value) => Ok(value),
            _ => Err(InstallOperationError::Storage),
        }
    }
    pub(crate) fn pending_public_app_installs(
        &self,
        after: Option<(&str, &str)>,
    ) -> Result<Vec<(String, String)>> {
        let connection = self
            .lock_connection("durable_state.pending_app_installs")
            .map_err(|_| InstallOperationError::Storage)?;
        let mut statement = sql(connection.prepare(
            "SELECT owner_id,request_id FROM app_installation_operations
            WHERE session_id IS NOT NULL AND phase IN ('preparing','approval','starting')
            AND (?1 IS NULL OR (owner_id,request_id)>(?1,?2)) ORDER BY owner_id,request_id LIMIT 8",
        ))?;
        let rows = sql(statement
            .query_map(params![after.map(|v| v.0), after.map(|v| v.1)], |r| {
                Ok((r.get(0)?, r.get(1)?))
            }))?;
        sql(rows.collect())
    }
}
fn commit(
    tx: rusqlite::Transaction<'_>,
    budget: &AppOperationBudget,
    operation: InstallOperation,
) -> Result<Reply> {
    limit(budget)?;
    tx.commit()
        .map_err(|_| InstallOperationError::CommitUnknown)?;
    Ok(Reply::Operation(operation))
}
pub(super) fn apply(connection: &mut Connection, command: PublicCommand) -> Result<Reply> {
    match command {
        PublicCommand::Reserve {
            owner,
            request_id,
            input,
            digest,
            budget,
        } => {
            identity(&owner)?;
            identity(&request_id)?;
            identity(&input.session_id)?;
            if input.upload_handle.len() != 71
                || !input.upload_handle.starts_with("upload_")
                || digest.len() != 71
                || !digest.strip_prefix("sha256:").is_some_and(|v| {
                    v.bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                })
            {
                return Err(InstallOperationError::Invalid);
            }
            limit(&budget)?;
            let tx = sql(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
            if let Some(old) = load(&tx, &owner, &request_id)? {
                if old.package_digest != digest || old.input.as_ref() != Some(&input) {
                    return Err(InstallOperationError::Conflict);
                }
                sql(tx.execute("UPDATE app_installation_operations SET updated_ms=?1 WHERE owner_id=?2 AND request_id=?3",params![now()?,owner,request_id]))?;
                return commit(tx, &budget, old);
            }
            admit(&tx, &owner)?;
            let id = format!("app_{:032x}", rand::random::<u128>());
            let time = now()?;
            sql(tx.execute("INSERT INTO app_installation_operations(owner_id,request_id,installation_id,package_digest,phase,session_id,upload_handle,created_ms,updated_ms)
                VALUES(?1,?2,?3,?4,'preparing',?5,?6,?7,?7)", params![owner,request_id,id,digest,input.session_id,input.upload_handle,time]))?;
            let value = load(&tx, &owner, &request_id)?.ok_or(InstallOperationError::Storage)?;
            commit(tx, &budget, value)
        }
        PublicCommand::Prepared {
            owner,
            request_id,
            candidate,
            budget,
        } => {
            limit(&budget)?;
            let review = serde_json::to_string(candidate.review_metadata())
                .map_err(|_| InstallOperationError::Storage)?;
            if review.len() > 128 * 1024 {
                return Err(InstallOperationError::Limit);
            }
            let tx = sql(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
            let current = load(&tx, &owner, &request_id)?.ok_or(InstallOperationError::NotFound)?;
            if current.package_digest != candidate.release_metadata().package_digest {
                return Err(InstallOperationError::Conflict);
            }
            if current.phase != InstallPhase::Preparing {
                return commit(tx, &budget, current);
            }
            let time = now()?;
            candidate
                .stage_first_in(&tx, &owner, &current.token.installation_id, time as u64)
                .map_err(|_| InstallOperationError::Stale)?;
            sql(tx.execute("UPDATE app_installation_operations SET phase='approval',review_json=?1,updated_ms=?2 WHERE owner_id=?3 AND request_id=?4",params![review,time,owner,request_id]))?;
            let result = load(&tx, &owner, &request_id)?.ok_or(InstallOperationError::Storage)?;
            commit(tx, &budget, result)
        }
        PublicCommand::Arm {
            owner,
            request_id,
            interaction_id,
            budget,
        } => {
            identity(&interaction_id)?;
            limit(&budget)?;
            let current =
                load(connection, &owner, &request_id)?.ok_or(InstallOperationError::NotFound)?;
            if !matches!(
                current.phase,
                InstallPhase::AwaitingApproval | InstallPhase::Starting
            ) {
                return Ok(Reply::Review(InstallReviewDisposition::Terminal));
            }
            let binding = InstallationRegistry::new(connection)
                .staged_trust(&owner, &current.token)
                .map_err(|_| InstallOperationError::Stale)?;
            let trust = PublisherTrustRegistry::new(connection)
                .trusted_publisher(&owner, binding.publisher_id(), binding.key_id())
                .map_err(|_| InstallOperationError::Stale)?;
            let tx = sql(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
            let same = load(&tx, &owner, &request_id)?.ok_or(InstallOperationError::NotFound)?;
            if same != current {
                return Err(InstallOperationError::Conflict);
            }
            let update = binding
                .review_first_in(&tx, &owner, &trust)
                .map_err(|_| InstallOperationError::Stale)?;
            if matches!(update.decision, CapabilityDecision::Approved { .. }) {
                limit(&budget)?;
                tx.commit()
                    .map_err(|_| InstallOperationError::CommitUnknown)?;
                return Ok(Reply::Review(InstallReviewDisposition::Approved));
            }
            if update.decision != CapabilityDecision::Pending {
                return Err(InstallOperationError::Conflict);
            }
            let review = current.review.ok_or(InstallOperationError::Storage)?;
            if review.get("capabilitiesDigest").and_then(|v| v.as_str())
                != Some(update.release.capabilities_digest.as_str())
            {
                return Err(InstallOperationError::Storage);
            }
            sql(tx.execute("UPDATE app_installation_operations SET interaction_id=?1,updated_ms=?2 WHERE owner_id=?3 AND request_id=?4",params![interaction_id,now()?,owner,request_id]))?;
            limit(&budget)?;
            tx.commit()
                .map_err(|_| InstallOperationError::CommitUnknown)?;
            Ok(Reply::Review(InstallReviewDisposition::Prompt(
                InstallApprovalChallenge {
                    owner,
                    request_id,
                    interaction_id,
                    deadline: std::time::Instant::now() + std::time::Duration::from_secs(300),
                    binding,
                    trust,
                    capabilities_digest: update.release.capabilities_digest,
                    review,
                },
            )))
        }
        PublicCommand::Decide {
            challenge,
            accepted,
            budget,
        } => {
            let deadline = challenge.deadline;
            let budget = budget.fork(move || std::time::Instant::now() >= deadline);
            limit(&budget)?;
            let tx = sql(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
            let current = load(&tx, &challenge.owner, &challenge.request_id)?
                .ok_or(InstallOperationError::NotFound)?;
            if current.phase != InstallPhase::AwaitingApproval
                || current.interaction_id.as_deref() != Some(challenge.interaction_id.as_str())
            {
                return Err(InstallOperationError::Conflict);
            }
            let record = challenge
                .binding
                .review_first_in(&tx, &challenge.owner, &challenge.trust)
                .map_err(|_| InstallOperationError::Stale)?;
            if record.release.capabilities_digest != challenge.capabilities_digest {
                return Err(InstallOperationError::Conflict);
            }
            let decision = if accepted {
                CapabilityDecision::Approved {
                    approval: CapabilityApproval {
                        decision_id: challenge.interaction_id.clone(),
                        authority_ref: "kernel_operation_human".into(),
                    },
                }
            } else {
                CapabilityDecision::Declined {
                    decision_id: challenge.interaction_id.clone(),
                    authority_ref: "kernel_operation_human".into(),
                }
            };
            challenge
                .binding
                .decide_first_in(
                    &tx,
                    &challenge.owner,
                    &challenge.trust,
                    decision,
                    now()? as u64,
                )
                .map_err(|_| InstallOperationError::Stale)?;
            sql(tx.execute("UPDATE app_installation_operations SET phase=?1,interaction_id=NULL,cleanup_pending=?2,updated_ms=?3 WHERE owner_id=?4 AND request_id=?5",
                params![if accepted {"approval"} else {"cancelled"},!accepted,now()?,challenge.owner,challenge.request_id]))?;
            let result = load(&tx, &challenge.owner, &challenge.request_id)?
                .ok_or(InstallOperationError::Storage)?;
            commit(tx, &budget, result)
        }
        PublicCommand::Fail {
            owner,
            request_id,
            code,
            budget,
        } => {
            identity(&code)?;
            if code.len() > 96 || !code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                return Err(InstallOperationError::Invalid);
            }
            limit(&budget)?;
            let tx = sql(connection.transaction_with_behavior(TransactionBehavior::Immediate))?;
            let current = load(&tx, &owner, &request_id)?.ok_or(InstallOperationError::NotFound)?;
            if matches!(
                current.phase,
                InstallPhase::Committed | InstallPhase::Cancelled | InstallPhase::Failed
            ) {
                return commit(tx, &budget, current);
            }
            if current.phase != InstallPhase::Preparing {
                let binding = StageTrustBinding::staged_in(&tx, &owner, &current.token)
                    .map_err(|_| InstallOperationError::Stale)?;
                binding
                    .abort_first_in(&tx, &owner, &code, now()? as u64)
                    .map_err(|_| InstallOperationError::Conflict)?;
            }
            sql(tx.execute("UPDATE app_installation_operations SET phase='failed',failure=?1,interaction_id=NULL,cleanup_pending=1,updated_ms=?2 WHERE owner_id=?3 AND request_id=?4",params![code,now()?,owner,request_id]))?;
            let result = load(&tx, &owner, &request_id)?.ok_or(InstallOperationError::Storage)?;
            commit(tx, &budget, result)
        }
    }
}
