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
impl ProjectEnvironmentStore {
    pub(crate) fn load_checks(
        &self,
        project: &str,
    ) -> Result<EnvironmentChecks, crate::error::DaemonError> {
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
        crate::config::write_private_file(&self.path(project).with_extension("checks.json"), &bytes)
            .map_err(|_| environment_error("Check journal write failed"))
    }
    pub(crate) fn attach_checks(
        &self,
        env: &mut ProjectEnvironment,
        target: &EnvironmentTargetBinding,
        now: u64,
    ) -> Result<(), crate::error::DaemonError> {
        let checks = self.load_checks(&env.local_project_id)?;
        let bindings = folder_bindings_digest(env);
        env.observations = checks
            .observations
            .into_iter()
            .map(|mut o| {
                o.stale |= o.target != *target
                    || o.revision_digest != env.content_digest
                    || o.environment_id != env.lineage.environment_id
                    || checks.folder_binding_digest != bindings
                    || o.checked_at_ms
                        .is_none_or(|t| t > now || now.saturating_sub(t) > OBSERVATION_TTL_MS);
                o
            })
            .collect();
        env.operations
            .extend(checks.operations.into_iter().map(|mut o| {
                if o.target != *target
                    && matches!(
                        o.phase,
                        EnvironmentOperationPhase::Requested
                            | EnvironmentOperationPhase::Running
                            | EnvironmentOperationPhase::Validating
                    )
                {
                    o.phase = EnvironmentOperationPhase::Failed;
                    o.recovery_state = EnvironmentRecoveryState::Settled;
                }
                o
            }));
        Ok(())
    }
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
