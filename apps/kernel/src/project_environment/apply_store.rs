//! MP-08 / MP-10 / MP-11: private durable first-Apply journal, using P01 wire shapes.
use super::*;
use crate::error::DaemonError;
use serde::{Deserialize, Serialize};
use std::io::Read;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredApplyPlan {
    pub plan: EnvironmentApplyPlan,
    pub folder_binding_digest: String,
    #[serde(default)]
    pub binding_scopes: Vec<RequirementScope>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApplyAttempt {
    pub plan_id: String,
    /// Original admission identity survives retry target-generation changes.
    #[serde(default)]
    pub request_digest: Option<String>,
    pub operation: EnvironmentOperation,
    pub folder_binding_digest: String,
    #[serde(default)]
    pub binding_scopes: Vec<RequirementScope>,
    pub active_requirement: Option<String>,
    pub uncertain_requirements: Vec<String>,
    /// Per-requirement high-water mark, durable before admitting a command.
    #[serde(default)]
    pub setup_admitted_commands: std::collections::BTreeMap<String, usize>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnvironmentApplies {
    pub plans: Vec<StoredApplyPlan>,
    pub attempts: Vec<ApplyAttempt>,
}
pub(crate) fn replayed_apply_operation(
    journal: &EnvironmentApplies,
    request: &crate::local::ApplyProjectEnvironmentRequest,
) -> Result<Option<EnvironmentOperation>, DaemonError> {
    let Some(original) = journal
        .attempts
        .iter()
        .find(|a| a.operation.operation_id == request.operation_id)
    else {
        return Ok(None);
    };
    let matches = original.request_digest.as_ref().map_or_else(
        || {
            let op = &original.operation;
            original.plan_id == request.plan_id
                && op.revision_digest == request.revision_digest
                && op.target == request.target
                && op.selected_items == request.selected_items
                && op.per_item_opt_ins == request.per_item_opt_ins
        },
        |digest| *digest == metadata_digest(request),
    );
    if !matches {
        return Err(environment_error(
            "Apply operation identity names another request",
        ));
    }
    Ok(journal
        .attempts
        .iter()
        .rev()
        .find(|a| a.operation.operation_id == request.operation_id)
        .map(|a| a.operation.clone()))
}
pub(crate) fn latest_apply_attempt<'a>(
    journal: &'a mut EnvironmentApplies,
    op: &EnvironmentOperation,
) -> Option<&'a mut ApplyAttempt> {
    // Find the newest identity first: an old completion must not fall back to
    // its historical attempt after an explicit retry has superseded it.
    journal
        .attempts
        .iter_mut()
        .rev()
        .find(|a| a.operation.operation_id == op.operation_id)
        .filter(|a| a.operation.attempt == op.attempt && a.operation.target == op.target)
}
pub(crate) fn apply_active(op: &EnvironmentOperation) -> bool {
    matches!(
        op.phase,
        EnvironmentOperationPhase::Requested
            | EnvironmentOperationPhase::Preparing
            | EnvironmentOperationPhase::Running
            | EnvironmentOperationPhase::Validating
    )
}
// MP-11: target inventory alone cannot prove an interrupted command completed.
pub(crate) fn setup_reconciliation_allowed(
    has_receipt: bool,
    uncertain: bool,
    commands_confirmed: bool,
    checks_ready: bool,
) -> bool {
    (has_receipt || (uncertain && commands_confirmed)) && checks_ready
}
impl ProjectEnvironmentStore {
    pub(crate) fn load_applies(&self, project: &str) -> Result<EnvironmentApplies, DaemonError> {
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = match options.open(self.path(project).with_extension("apply.json")) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(EnvironmentApplies::default())
            }
            Err(_) => return Err(environment_error("Apply journal unavailable")),
        };
        if !file.metadata().is_ok_and(|m| m.is_file()) {
            return Err(environment_error("Apply journal must be regular"));
        }
        let mut bytes = vec![];
        file.take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| environment_error("Apply journal read failed"))?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err(environment_error("Apply journal exceeds bounds"));
        }
        let journal: EnvironmentApplies = serde_json::from_slice(&bytes)
            .map_err(|_| environment_error("Invalid Apply journal"))?;
        if journal.attempts.iter().any(|a| {
            a.operation.local_project_id != project
                || a.operation.kind != EnvironmentOperationKind::Apply
        }) || journal.plans.iter().any(|p| p.plan.project_id != project)
        {
            return Err(environment_error("Apply journal binding mismatch"));
        }
        Ok(journal)
    }
    pub(crate) fn save_applies(
        &self,
        project: &str,
        journal: &EnvironmentApplies,
    ) -> Result<(), DaemonError> {
        let bytes = serde_json::to_vec(journal)
            .map_err(|_| environment_error("Apply journal encoding failed"))?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err(environment_error("Apply journal exceeds bounds"));
        }
        crate::config::write_private_file(&self.path(project).with_extension("apply.json"), &bytes)
            .map_err(|_| environment_error("Apply journal write failed"))
    }
    pub(crate) fn recover_applies(
        &self,
        project: &str,
        target: &EnvironmentTargetBinding,
    ) -> Result<EnvironmentApplies, DaemonError> {
        let mut journal = self.load_applies(project)?;
        let mut changed = false;
        for a in &mut journal.attempts {
            if apply_active(&a.operation) && a.operation.target != *target {
                if let Some(id) = a.active_requirement.take() {
                    if !a.uncertain_requirements.contains(&id) {
                        a.uncertain_requirements.push(id);
                    }
                }
                a.operation.phase = EnvironmentOperationPhase::Failed;
                a.operation.updated_at_ms = crate::session::unix_epoch_ms();
                a.operation.recovery_state = EnvironmentRecoveryState::Resumable {
                    checkpoint_digest: metadata_digest(&a.operation.receipts),
                    last_settled_requirement_id: a
                        .operation
                        .receipts
                        .last()
                        .and_then(|r| r.requirement_id.clone()),
                };
                changed = true;
            }
        }
        if changed {
            self.save_applies(project, &journal)?;
        }
        Ok(journal)
    }
    pub(crate) fn attach_applies(
        &self,
        env: &mut ProjectEnvironment,
        target: &EnvironmentTargetBinding,
    ) -> Result<(), DaemonError> {
        // Recovery mutation is serialized with Apply/Save; a read cannot race a receipt.
        let _lock = self.lock_briefly(&env.local_project_id)?;
        let journal = self.recover_applies(&env.local_project_id, target)?;
        env.operations.extend(
            journal
                .attempts
                .into_iter()
                .rev()
                .take(4)
                .map(|a| a.operation),
        );
        Ok(())
    }
}
