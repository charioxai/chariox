//! Transaction composition for supervised first installs. The kernel owns
//! approvals and actual worker health; these functions never attest either.
use super::*;
use crate::installation::{
    clear_pending, save_update, CapabilityApproval, CapabilityDecision, UpdatePhase,
};

impl VerifiedInstallCandidate {
    /// Initial stage and caller's operation receipt MUST commit together. No
    /// installation remains if the surrounding transaction is rolled back.
    pub fn stage_first_in(
        &self,
        tx: &Transaction<'_>,
        owner: &str,
        installation: &str,
        now_ms: u64,
    ) -> Result<UpdateRecord> {
        self.trust.require_current(tx, owner)?;
        create_installation(tx, installation, &self.release.app_id, owner)?;
        let record = stage_release(tx, installation, 0, self.release.clone(), now_ms)?;
        save_binding(tx, owner, &record, &self.trust)?;
        tx.execute(
            "INSERT INTO app_installation_supervised_stages(installation_id,generation)
             VALUES(?1,?2)",
            params![installation, sql_generation(record.token.generation)?],
        )?;
        Ok(record)
    }
}

impl StageTrustBinding {
    /// An immutable snapshot, not admission. Every mutation must repeat the
    /// signer/current-stage check inside its committing writer transaction.
    pub fn staged_in(tx: &Transaction<'_>, owner: &str, token: &StageToken) -> Result<Self> {
        load_binding(tx, owner, token)
    }

    pub fn review_first_in(
        &self,
        tx: &Transaction<'_>,
        owner: &str,
        trust: &TrustedPublisherSnapshot,
    ) -> Result<UpdateRecord> {
        if self.token.base_generation != 0 || self.token.generation != 1 {
            return Err(InstallationError::InvalidTransition.into());
        }
        self.require_current(tx, owner, trust)?;
        current_update(tx, &self.token).map_err(Into::into)
    }
    /// Trusted kernel decision, fenced inside its original committing transaction.
    pub fn decide_first_in(
        &self,
        tx: &Transaction<'_>,
        owner: &str,
        trust: &TrustedPublisherSnapshot,
        decision: CapabilityDecision,
        now_ms: u64,
    ) -> Result<UpdateRecord> {
        self.review_first_in(tx, owner, trust)?;
        crate::installation::decide_generation(tx, &self.token, decision, now_ms)
            .map_err(Into::into)
    }

    /// Reads the existing approval; no caller can supply an approval boolean.
    /// The caller passes an enrollment snapshot, which is rechecked in this tx.
    pub fn require_first_approved_in(
        &self,
        tx: &Transaction<'_>,
        owner: &str,
        trust: &TrustedPublisherSnapshot,
    ) -> Result<CapabilityApproval> {
        if self.token.base_generation != 0 || self.token.generation != 1 {
            return Err(InstallationError::InvalidTransition.into());
        }
        self.require_current(tx, owner, trust)?;
        let record = current_update(tx, &self.token)?;
        if !matches!(record.phase, UpdatePhase::Staged | UpdatePhase::Quiescing) {
            return Err(InstallationError::InvalidTransition.into());
        }
        match record.decision {
            CapabilityDecision::Approved { approval } => Ok(approval),
            _ => Err(InstallationError::ApprovalRequired.into()),
        }
    }

    /// Quiescing a new generation only fences admission; there is no prior
    /// worker/data snapshot to restore. Health has not run at this point.
    pub fn quiesce_first_in(
        &self,
        tx: &Transaction<'_>,
        owner: &str,
        trust: &TrustedPublisherSnapshot,
        now_ms: u64,
    ) -> Result<CapabilityApproval> {
        let approval = self.require_first_approved_in(tx, owner, trust)?;
        let mut record = current_update(tx, &self.token)?;
        record.phase = UpdatePhase::Quiescing;
        record.updated_at_ms = now_ms;
        tx.execute(
            "UPDATE app_installations SET admission_paused=1 WHERE installation_id=?1",
            [&self.token.installation_id],
        )?;
        save_update(tx, &record)?;
        Ok(approval)
    }

    /// The kernel must hold the exact supervised worker's local health proof.
    /// This joins preparation, signer/approval fencing and activation into the
    /// SAME transaction as the kernel's durable operation/worker-health record.
    pub fn commit_first_in(
        &self,
        tx: &Transaction<'_>,
        owner: &str,
        trust: &TrustedPublisherSnapshot,
        approval: &CapabilityApproval,
        now_ms: u64,
    ) -> Result<ActiveGeneration> {
        if self.require_first_approved_in(tx, owner, trust)? != *approval {
            return Err(InstallationError::ApprovalRequired.into());
        }
        let mut record = current_update(tx, &self.token)?;
        if record.phase != UpdatePhase::Quiescing {
            return Err(InstallationError::InvalidTransition.into());
        }
        record.phase = UpdatePhase::Prepared;
        record.updated_at_ms = now_ms;
        save_update(tx, &record)?;
        commit_generation(tx, &self.token, now_ms).map_err(Into::into)
    }

    /// Cancels only a still-pending first generation. A committed generation is
    /// never silently rolled back, even when the worker failed before publish.
    pub fn abort_first_in(
        &self,
        tx: &Transaction<'_>,
        owner: &str,
        reason: &str,
        now_ms: u64,
    ) -> Result<()> {
        if self.token.base_generation != 0
            || self.token.generation != 1
            || reason.is_empty()
            || reason.len() > 96
            || !reason
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(InstallationError::InvalidTransition.into());
        }
        self.require_binding(tx, owner)?;
        let stored = load_update(tx, &self.token.installation_id, self.token.generation)?;
        if stored.token == self.token && stored.phase == UpdatePhase::Aborted {
            let installation = load_installation(tx, &self.token.installation_id)?;
            if installation.generation == 0
                && installation.active.is_none()
                && installation.pending_generation.is_none()
            {
                // A separate existing capability decline may have aborted the
                // same first stage. Preserve that reason while the operation
                // receipt is terminalized in this surrounding transaction.
                return Ok(());
            }
        }
        let mut record = current_update(tx, &self.token)?;
        record.phase = UpdatePhase::Aborted;
        record.abort_reason = Some(reason.into());
        record.updated_at_ms = now_ms;
        clear_pending(tx, &self.token)?;
        save_update(tx, &record)?;
        Ok(())
    }
}
