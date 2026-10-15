//! MP-08 / MP-10 / MP-11: private observation journal, distinct from immutable specification.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;

pub(crate) const OBSERVATION_TTL_MS: u64 = 5 * 60 * 1000;
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnvironmentChecks {
    pub operations: Vec<EnvironmentOperation>,
    pub observations: Vec<MachineObservation>,
    // Public binding fingerprints stay private; never a client value/hash oracle.
    pub public_bindings: BTreeMap<String, String>,
    pub folder_binding_digest: String,
}
// MP-08 / MP-10 / MP-11: current observations are separate from operation history.
impl EnvironmentChecks {
    pub(crate) fn invalidate_observations(
        &mut self,
        env: &ProjectEnvironment,
        target: &EnvironmentTargetBinding,
        now: u64,
    ) {
        let bindings = folder_bindings_digest(env);
        for observation in &mut self.observations {
            observation.stale |= observation.target != *target
                || observation.revision_digest != env.content_digest
                || observation.environment_id != env.lineage.environment_id
                || self.folder_binding_digest != bindings
                || observation.checked_at_ms.is_none_or(|checked| {
                    checked > now || now.saturating_sub(checked) > OBSERVATION_TTL_MS
                });
        }
        super::check::invalidate_check_dependencies(env, &mut self.observations, target, None);
    }
    pub(crate) fn record_observation(&mut self, observation: MachineObservation) {
        self.observations.retain(|prior| {
            // A new probe supersedes this machine/slice's previous process generation.
            // Other targets and unselected requirements keep their own stale evidence.
            prior.requirement_id != observation.requirement_id
                || prior.target.machine_id != observation.target.machine_id
                || prior.target.slice_ref != observation.target.slice_ref
        });
        self.observations.push(observation);
    }
}
impl ProjectEnvironmentStore {
    pub(crate) fn load_checks(
        &self,
        project: &str,
    ) -> Result<EnvironmentChecks, crate::error::DaemonError> {
        let _publication = self.check_journal_lock(project)?;
        let bytes =
            match super::check_journal::read(&self.path(project).with_extension("checks.wal"))? {
                Some(bytes) => bytes,
                None => {
                    let mut o = std::fs::OpenOptions::new();
                    o.read(true);
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::OpenOptionsExt;
                        o.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
                    }
                    let f = match o.open(self.path(project).with_extension("checks.json")) {
                        Ok(f) => f,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                            return Ok(EnvironmentChecks::default())
                        }
                        Err(_) => return Err(environment_error("Check journal unavailable")),
                    };
                    if !f.metadata().is_ok_and(|m| m.is_file()) {
                        return Err(environment_error("Check journal must be regular"));
                    }
                    let mut bytes = vec![];
                    f.take(8 * 1024 * 1024 + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|_| environment_error("Check journal unavailable"))?;
                    if bytes.len() > 8 * 1024 * 1024 {
                        return Err(environment_error("Check journal exceeds bounds"));
                    }
                    bytes
                }
            };
        let checks: EnvironmentChecks = serde_json::from_slice(&bytes)
            .map_err(|_| environment_error("invalid Check journal"))?;
        if checks
            .operations
            .iter()
            .any(|o| o.local_project_id != project || o.kind != EnvironmentOperationKind::Check)
        {
            return Err(environment_error("Check journal binding mismatch"));
        }
        Ok(checks)
    }
    pub(crate) fn save_checks(
        &self,
        project: &str,
        checks: &EnvironmentChecks,
    ) -> Result<(), crate::error::DaemonError> {
        let bytes = serde_json::to_vec(checks)
            .map_err(|_| environment_error("Check journal encoding failed"))?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err(environment_error("Check journal exceeds bounds"));
        }
        let _publication = self.check_journal_lock(project)?;
        super::check_journal::append(&self.path(project).with_extension("checks.wal"), &bytes)
    }
    pub(crate) fn attach_checks(
        &self,
        env: &mut ProjectEnvironment,
        target: &EnvironmentTargetBinding,
        now: u64,
    ) -> Result<(), crate::error::DaemonError> {
        let mut checks = self.load_checks(&env.local_project_id)?;
        checks.invalidate_observations(env, target, now);
        let requirements: std::collections::HashSet<_> = env
            .project_requirements
            .iter()
            .chain(env.folders.iter().flat_map(|f| &f.requirements))
            .map(|r| r.requirement_id.clone())
            .collect();
        env.observations = checks
            .observations
            .into_iter()
            .filter(|o| requirements.contains(&o.requirement_id))
            .collect();
        // MP-08 / MP-10 / MP-11: the current view need not replay old result
        // bodies on every poll. Retained operation IDs remain queryable below.
        env.operations.extend(
            checks
                .operations
                .into_iter()
                .last()
                .map(|o| project_check_operation(o, target)),
        );
        Ok(())
    }
    pub(crate) fn check_operation(
        &self,
        project: &str,
        id: &str,
        target: &EnvironmentTargetBinding,
    ) -> Result<Option<EnvironmentOperation>, crate::error::DaemonError> {
        Ok(self
            .load_checks(project)?
            .operations
            .into_iter()
            .find(|o| o.operation_id == id)
            .map(|o| project_check_operation(o, target)))
    }
}
fn project_check_operation(
    mut operation: EnvironmentOperation,
    target: &EnvironmentTargetBinding,
) -> EnvironmentOperation {
    if operation.target != *target
        && matches!(
            operation.phase,
            EnvironmentOperationPhase::Requested
                | EnvironmentOperationPhase::Running
                | EnvironmentOperationPhase::Validating
        )
    {
        operation.phase = EnvironmentOperationPhase::Failed;
        operation.recovery_state = EnvironmentRecoveryState::Settled;
    }
    operation
}
pub(crate) fn folder_bindings_digest(env: &ProjectEnvironment) -> String {
    super::aggregate_projection::metadata_digest(
        &env.folders
            .iter()
            .map(|f| (&f.folder_id, &f.local_workspace_binding))
            .collect::<Vec<_>>(),
    )
}

/// MP-08 / MP-10 / MP-11: durable daemon identity selects a kernel; a
/// process-local generation binds its observations across kernel restarts.
pub(crate) fn current_check_target(
    config: &crate::config::DaemonConfig,
) -> EnvironmentTargetBinding {
    static GENERATION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let generation = GENERATION.get_or_init(|| format!("{:032x}", rand::random::<u128>()));
    EnvironmentTargetBinding {
        machine_id: config.host_machine_id.clone(),
        target_instance_generation: format!("{}:{generation}", config.daemon_id),
        slice_ref: None,
    }
}
