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

/// Requested Variables and Secret values for an MCP server. A locked Vault leaves Secrets
/// out while the Project's ordinary Variables remain available.
pub(crate) fn project_mcp_environment(
    config: &DaemonConfig,
    project_id: &str,
    names: &[String],
) -> std::collections::BTreeMap<String, String> {
    let mut values = std::collections::BTreeMap::new();
    let Ok(Some(record)) =
        ProjectEnvironmentStore::new(&config.private_runtime_state_root()).load(project_id)
    else {
        return values;
    };
    for (name, value) in
        parse_variables(&record.variables).expect("loaded record has valid Variables")
    {
        if names.contains(&name) {
            values.insert(name, value);
        }
    }
    let wanted: Vec<&String> = names
        .iter()
        .filter(|name| record.secrets.contains(name))
        .collect();
    if wanted.is_empty() {
        return values;
    }
    let Ok(vault) = crate::secret::project_environment_vault(config) else {
        return values;
    };
    let service = project_environment_vault_service(project_id);
    for name in wanted {
        if let Ok(Some(value)) = vault.find_secret(&service, name) {
            values.insert(name.clone(), value);
        }
    }
    values
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
    sessions: &crate::session::SessionStateStore,
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
    let Some(project) = sessions.environment_project(session) else {
        return Ok(request);
    };
    let Some(record) =
        ProjectEnvironmentStore::new(&config.private_runtime_state_root()).load(project.id())?
    else {
        return Ok(request);
    };
    let launch = project_launch_environment(config, project.id(), &record)?;
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
    fn mcp_environment_keeps_variables_when_the_project_vault_is_locked() {
        let mut config = DaemonConfig::for_tests();
        let root = config.private_runtime_state_root();
        let vault_path = root.join("vault.json");
        config.user_config.credential_vault.path = vault_path.display().to_string();
        let store = ProjectEnvironmentStore::new(&root);
        store
            .save(
                "project",
                0,
                ProjectEnvironmentRecord {
                    schema: super::super::PROJECT_ENVIRONMENT_SCHEMA,
                    variables: "API_BASE_URL=https://project.example.test\nUNREQUESTED=other"
                        .into(),
                    secrets: vec!["API_KEY".into()],
                    ..Default::default()
                },
            )
            .unwrap();
        crate::secret::unlock_chariox_encrypted_vault(
            &vault_path,
            "synthetic-test-passphrase",
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
        .unwrap();
        let vault = crate::secret::project_environment_vault(&config).unwrap();
        vault
            .set_secret(
                &project_environment_vault_service("project"),
                "API_KEY",
                "synthetic-key",
            )
            .unwrap();
        let names = ["API_BASE_URL".into(), "API_KEY".into(), "ABSENT".into()];
        let unlocked = project_mcp_environment(&config, "project", &names);
        crate::secret::lock_chariox_encrypted_vault(&vault_path).unwrap();
        let locked = project_mcp_environment(&config, "project", &names);
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(
            unlocked,
            std::collections::BTreeMap::from([
                ("API_BASE_URL".into(), "https://project.example.test".into()),
                ("API_KEY".into(), "synthetic-key".into()),
            ])
        );
        assert_eq!(
            locked,
            std::collections::BTreeMap::from([(
                "API_BASE_URL".into(),
                "https://project.example.test".into()
            ),])
        );
    }

    #[test]
    fn mcp_environment_resolves_only_requested_project_variables_without_a_vault() {
        let config = DaemonConfig::for_tests();
        let root = config.private_runtime_state_root();
        let store = ProjectEnvironmentStore::new(&root);
        store
            .save(
                "project-a",
                0,
                ProjectEnvironmentRecord {
                    schema: super::super::PROJECT_ENVIRONMENT_SCHEMA,
                    variables:
                        "ACCOUNT=project-a\nURL=https://example.test/mcp\nUNREQUESTED=private"
                            .into(),
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .save(
                "project-b",
                0,
                ProjectEnvironmentRecord {
                    schema: super::super::PROJECT_ENVIRONMENT_SCHEMA,
                    variables: "ACCOUNT=project-b".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let names = ["ACCOUNT".into(), "URL".into(), "ABSENT".into()];
        let a = project_mcp_environment(&config, "project-a", &names);
        let b = project_mcp_environment(&config, "project-b", &names);
        let absent = project_mcp_environment(&config, "other-project", &names);
        // Clean the fixture before asserting so fail-first runs leave no state behind.
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(
            a,
            std::collections::BTreeMap::from([
                ("ACCOUNT".into(), "project-a".into()),
                ("URL".into(), "https://example.test/mcp".into()),
            ])
        );
        assert_eq!(
            b,
            std::collections::BTreeMap::from([("ACCOUNT".into(), "project-b".into())])
        );
        assert!(absent.is_empty());
    }

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
