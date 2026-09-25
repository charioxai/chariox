//! Transaction composition for supervised installs and local updates. The
//! kernel owns approvals and actual worker health; these functions never
//! attest either. Every transition is fenced by `current_update`: the stage's
//! base is the installation's current generation and it is still pending.
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
        self.stage_supervised_in(tx, owner, installation, 0, now_ms)
    }

    /// A local replacement of the owner's active installation. The App's data
    /// is reused as is, so a release that changes its data schema is refused
    /// until restricted migrations run as part of the update.
    pub fn stage_update_in(
        &self,
        tx: &Transaction<'_>,
        owner: &str,
        installation: &str,
        expected_generation: u64,
        now_ms: u64,
    ) -> Result<UpdateRecord> {
        self.trust.require_current(tx, owner)?;
        let current = load_installation(tx, installation)?;
        if current.owner_id != owner {
            return Err(InstallationError::NotFound.into());
        }
        let active = current.active.ok_or(InstallationError::Inactive)?;
        if active.release.schema_version != self.release.schema_version {
            return Err(InstallationError::Invalid("data migration required").into());
        }
        self.stage_supervised_in(tx, owner, installation, expected_generation, now_ms)
    }

    fn stage_supervised_in(
        &self,
        tx: &Transaction<'_>,
        owner: &str,
        installation: &str,
        expected_generation: u64,
        now_ms: u64,
    ) -> Result<UpdateRecord> {
        let record = stage_release(
            tx,
            installation,
            expected_generation,
            self.release.clone(),
            now_ms,
        )?;
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

    fn require_supervised(&self, tx: &Transaction<'_>) -> Result<()> {
        let supervised: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM app_installation_supervised_stages
             WHERE installation_id=?1 AND generation=?2)",
            params![
                self.token.installation_id,
                sql_generation(self.token.generation)?
            ],
            |row| row.get(0),
        )?;
        if !supervised {
            return Err(InstallationError::InvalidTransition.into());
        }
        Ok(())
    }

    pub fn review_in(
        &self,
        tx: &Transaction<'_>,
        owner: &str,
        trust: &TrustedPublisherSnapshot,
    ) -> Result<UpdateRecord> {
        self.require_supervised(tx)?;
        self.require_current(tx, owner, trust)?;
        current_update(tx, &self.token).map_err(Into::into)
    }
    /// Trusted kernel decision, fenced inside its original committing transaction.
    pub fn decide_in(
        &self,
        tx: &Transaction<'_>,
        owner: &str,
        trust: &TrustedPublisherSnapshot,
        decision: CapabilityDecision,
        now_ms: u64,
    ) -> Result<UpdateRecord> {
        self.review_in(tx, owner, trust)?;
        crate::installation::decide_generation(tx, &self.token, decision, now_ms)
            .map_err(Into::into)
    }

    /// Reads the existing approval; no caller can supply an approval boolean.
    /// The caller passes an enrollment snapshot, which is rechecked in this tx.
    pub fn require_approved_in(
        &self,
        tx: &Transaction<'_>,
        owner: &str,
        trust: &TrustedPublisherSnapshot,
    ) -> Result<CapabilityApproval> {
        let record = self.review_in(tx, owner, trust)?;
        if !matches!(record.phase, UpdatePhase::Staged | UpdatePhase::Quiescing) {
            return Err(InstallationError::InvalidTransition.into());
        }
        match record.decision {
            CapabilityDecision::Approved { approval } => Ok(approval),
            _ => Err(InstallationError::ApprovalRequired.into()),
        }
    }

    /// Fences admission to the installation (for an update, the old
    /// generation too); the caller stops its worker. Health has not run yet.
    pub fn quiesce_in(
        &self,
        tx: &Transaction<'_>,
        owner: &str,
        trust: &TrustedPublisherSnapshot,
        now_ms: u64,
    ) -> Result<CapabilityApproval> {
        let approval = self.require_approved_in(tx, owner, trust)?;
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
    pub fn commit_in(
        &self,
        tx: &Transaction<'_>,
        owner: &str,
        trust: &TrustedPublisherSnapshot,
        approval: &CapabilityApproval,
        now_ms: u64,
    ) -> Result<ActiveGeneration> {
        if self.require_approved_in(tx, owner, trust)? != *approval {
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

    /// Cancels only a still-pending generation; an update's base generation
    /// stays active and admission reopens. A committed generation is never
    /// silently rolled back, even when the worker failed before publish.
    pub fn abort_in(
        &self,
        tx: &Transaction<'_>,
        owner: &str,
        reason: &str,
        now_ms: u64,
    ) -> Result<()> {
        if reason.is_empty()
            || reason.len() > 96
            || !reason
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(InstallationError::InvalidTransition.into());
        }
        self.require_supervised(tx)?;
        self.require_binding(tx, owner)?;
        let stored = load_update(tx, &self.token.installation_id, self.token.generation)?;
        if stored.token == self.token && stored.phase == UpdatePhase::Aborted {
            let installation = load_installation(tx, &self.token.installation_id)?;
            if installation.generation == self.token.base_generation
                && installation.pending_generation.is_none()
            {
                // A separate existing capability decline may have aborted the
                // same stage. Preserve that reason while the operation
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
