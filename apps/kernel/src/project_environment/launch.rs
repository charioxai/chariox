//! Variables and Secret values enter only the ephemeral launch bindings of processes the kernel
//! starts for the Project, the same way for every placement.
use super::{
    project_environment_vault_service, record::parse_variables, ProjectEnvironmentRecord,
    ProjectEnvironmentStore,
};
use crate::{config::DaemonConfig, error::DaemonError, provider::ProviderCredentialEnvironment};

pub(crate) struct ProjectLaunchEnvironment {
    pub environment: ProviderCredentialEnvironment,
    /// Secret names the Vault has no value for on this kernel.
    pub missing_secrets: Vec<String>,
}

pub(crate) fn project_launch_environment(
    config: &DaemonConfig,
    project_id: &str,
    record: &ProjectEnvironmentRecord,
) -> Result<ProjectLaunchEnvironment, DaemonError> {
    let mut environment = ProviderCredentialEnvironment::default();
    for (name, value) in parse_variables(&record.variables).map_err(super::environment_error)? {
        environment.insert(name, zeroize::Zeroizing::new(value));
    }
    let mut missing_secrets = Vec::new();
    if !record.secrets.is_empty() {
        let vault = crate::secret::project_environment_vault(config)?;
        let service = project_environment_vault_service(project_id);
        for name in &record.secrets {
            // A locked Vault is an error here so the launch asks the owner to unlock it.
            match vault.find_secret(&service, name)? {
                Some(value) => environment.insert(name.clone(), zeroize::Zeroizing::new(value)),
                None => missing_secrets.push(name.clone()),
            }
        }
    }
    Ok(ProjectLaunchEnvironment {
        environment,
        missing_secrets,
    })
}

/// Provider reuse observes both the saved record and the freshly resolved values. The
/// process-keyed marker stays runtime-only, so a low-entropy value cannot be guessed from it.
pub(crate) fn project_environment_launch_revision(
    record: &ProjectEnvironmentRecord,
    environment: &ProviderCredentialEnvironment,
) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use std::sync::OnceLock;
    static PROCESS_KEY: OnceLock<[u8; 32]> = OnceLock::new();
    let mut mac = Hmac::<Sha256>::new_from_slice(PROCESS_KEY.get_or_init(rand::random))
        .expect("fixed HMAC key length");
    let mut update = |bytes: &[u8]| {
        mac.update(&(bytes.len() as u64).to_le_bytes());
        mac.update(bytes);
    };
    update(&serde_json::to_vec(record).expect("record encodes"));
    for (name, value) in environment.iter() {
        update(name.as_bytes());
        update(value.as_bytes());
    }
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// One resolver for ordinary prompt activation and runtime launches.
pub(crate) fn attach_project_provider_environment(
    config: &DaemonConfig,
    session: &crate::session::RuntimeSession,
    agent: Option<&crate::agent::AgentInstance>,
    mut request: crate::provider::LaunchProviderRequest,
) -> Result<crate::provider::LaunchProviderRequest, DaemonError> {
    let Some(directory) = request.working_directory.as_deref() else {
        return Ok(request);
    };
    // A metadata utility executes in a neutral directory outside the Project.
    let is_agent_worktree = agent
        .and_then(|agent| agent.worktree_id().map(std::path::PathBuf::from))
        .is_some_and(|worktree| worktree == directory);
    if directory != std::path::Path::new(session.worktree_id()) && !is_agent_worktree {
        return Ok(request);
    }
    let Some(record) = ProjectEnvironmentStore::new(&config.private_runtime_state_root())
        .load(session.project_id())?
    else {
        return Ok(request);
    };
    let launch = project_launch_environment(config, session.project_id(), &record)?;
    let previously_bound = request.project_environment_revision.is_some();
    request.project_environment_revision = Some(project_environment_launch_revision(
        &record,
        &launch.environment,
    ));
    if !launch.missing_secrets.is_empty() {
        request.provider_account_env.insert(
            "CHARIOX_PROJECT_MISSING_INPUTS".into(),
            serde_json::to_string(&launch.missing_secrets).expect("missing names encode"),
        );
    }
    for (name, value) in launch.environment.iter() {
        if crate::account_profile::provider_auth_env_vars(&request.provider).contains(&name)
            || (!previously_bound
                && request
                    .provider_credential_env
                    .iter()
                    .any(|(existing, _)| existing == name))
        {
            return Err(DaemonError::LocalTransport {
                operation: "Project environment",
                message: "Project input conflicts with provider credential binding".into(),
            });
        }
        request
            .provider_credential_env
            .insert(name, zeroize::Zeroizing::new(value.to_string()));
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_reuse_changes_with_the_record_and_with_live_values() {
        let mut record = ProjectEnvironmentRecord {
            schema: 2,
            ..Default::default()
        };
        let mut environment = ProviderCredentialEnvironment::default();
        environment.insert("INPUT", zeroize::Zeroizing::new("synthetic-one".into()));
        let first = project_environment_launch_revision(&record, &environment);
        assert_eq!(
            first,
            project_environment_launch_revision(&record, &environment)
        );
        environment.insert("INPUT", zeroize::Zeroizing::new("synthetic-two".into()));
        let second = project_environment_launch_revision(&record, &environment);
        assert_ne!(first, second);
        record.variables = "A=1".into();
        assert_ne!(
            second,
            project_environment_launch_revision(&record, &environment)
        );
        assert!(!format!("{environment:?}").contains("synthetic-two"));
    }
}
