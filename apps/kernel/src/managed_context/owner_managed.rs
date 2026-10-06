//! MP-05 / MP-08 / MP-11: owner admission for the shared managed-context pipeline.
use serde::{Deserialize, Serialize};

use super::outbound_service::{ManagedContextTransferTarget, ManagedContextTransferTicket};
use super::package::{
    ManagedContextGitCredentialSelection, ManagedContextKernelSelection, ManagedContextPlanBinding,
    ManagedContextProviderAccountSelection,
};
use crate::config::DaemonConfig;
use crate::error::DaemonError;

pub(crate) const TICKET_ENDPOINT: &str = "/v1/owner-managed-kernels/context/ticket";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum OwnerManagedDestination {
    OwnerManagedMachine {
        machine_id: String,
        kernel_id: String,
    },
}

impl OwnerManagedDestination {
    pub(crate) fn machine_id(&self) -> &str {
        match self {
            Self::OwnerManagedMachine { machine_id, .. } => machine_id,
        }
    }
    pub(crate) fn kernel_id(&self) -> &str {
        match self {
            Self::OwnerManagedMachine { kernel_id, .. } => kernel_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnerManagedTransfer {
    pub target: ManagedContextTransferTarget,
    pub context_selection: OwnerManagedContextSelection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnerManagedContextSelection {
    pub kernel_context: OwnerManagedKernelSelection,
    pub development_setup: OwnerManagedDevelopmentSelection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerManagedKernelSelection {
    Empty,
    SourceKernelWithoutCredentials,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum OwnerManagedDevelopmentSelection {
    Empty,
    SourceProject {
        project_id: String,
        repositories: Vec<OwnerManagedRepositorySelection>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnerManagedRepositorySelection {
    pub role: super::development::DevelopmentRepositoryRole,
    pub workspace_id: String,
    pub worktree_id: Option<String>,
}

pub(crate) fn prepare_ticket(
    config: &DaemonConfig,
    runtime: &crate::runtime::state::KernelRuntimeState,
    selection: OwnerManagedTransfer,
) -> Result<ManagedContextTransferTicket, DaemonError> {
    let profile = config
        .cloud_relay
        .as_ref()
        .ok_or_else(|| admission_error("source kernel is offline or stale"))?;
    if profile.kernel_id.as_deref() != Some(config.daemon_id.as_str())
        || profile.machine_id.as_deref() != Some(config.host_machine_id.as_str())
        || profile.kernel_public_key_thumbprint.as_deref()
            != Some(
                crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key)
                    .as_str(),
            )
        || profile
            .kernel_credential
            .as_deref()
            .is_none_or(str::is_empty)
        || !valid_identity(&profile.user_id)
        || !valid_identity(&profile.account_id)
        || !valid_identity(&profile.realm_id)
    {
        return Err(admission_error(
            "source kernel has no enrolled owner identity",
        ));
    }
    if selection.target.relay_realm_id != profile.realm_id
        || selection.target.kernel_id == config.daemon_id
    {
        return Err(admission_error("owner-managed source and destination must be distinct kernels in the same account realm"));
    }
    if let OwnerManagedDevelopmentSelection::SourceProject {
        project_id,
        repositories,
    } = &selection.context_selection.development_setup
    {
        let owner = crate::account_profile::provider_account_authority_owner_user_id(
            config,
            &profile.user_id,
        );
        let project = runtime
            .list_waiting_room_projects(&owner)
            .into_iter()
            .find(|project| project.id() == project_id)
            .ok_or_else(|| {
                admission_error("selected source Project is unavailable to this owner")
            })?;
        if project.owner_user_id() != owner
            || project.archived_at_ms().is_some()
            || repositories
                .iter()
                .any(|repository| !project.workspace_ids().contains(&repository.workspace_id))
        {
            return Err(admission_error(
                "selected source Workspace does not belong to the active Project",
            ));
        }
        for repository in repositories {
            super::outbound_service::resolve_repository_selection(
                &super::development::DevelopmentSourceRepositoryBinding {
                    role: repository.role,
                    workspace_id: repository.workspace_id.clone(),
                    worktree_id: repository.worktree_id.clone(),
                },
            )?;
        }
    }
    let context_plan =
        crate::managed_bootstrap::ManagedKernelContextPlan::for_owner_managed(config, &selection)
            .map_err(admission_error)?;
    Ok(ManagedContextTransferTicket {
        environment_id: String::new(),
        context_plan,
        target: selection.target,
    })
}

fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
}

pub(crate) fn validate_destination_binding(
    environment: &str,
    destination: Option<&OwnerManagedDestination>,
    kernel: &str,
) -> Result<(), DaemonError> {
    match destination {
        None if !environment.trim().is_empty() => Ok(()),
        Some(destination)
            if environment.is_empty()
                && valid_identity(destination.machine_id())
                && valid_identity(destination.kernel_id())
                && destination.kernel_id() == kernel =>
        {
            Ok(())
        }
        _ => Err(admission_error(
            "context transfer requires exactly one destination binding",
        )),
    }
}

pub(crate) fn validate_credential_free_plan(
    plan: &ManagedContextPlanBinding,
) -> Result<(), DaemonError> {
    if !matches!(
        plan.kernel_context,
        ManagedContextKernelSelection::Empty
            | ManagedContextKernelSelection::SourceKernelWithoutCredentials
    ) || plan.provider_accounts != ManagedContextProviderAccountSelection::None
        || plan.git_credentials != ManagedContextGitCredentialSelection::None
    {
        return Err(admission_error(
            "owner-managed context must exclude all credentials",
        ));
    }
    Ok(())
}

pub(crate) fn admission_error(message: impl Into<String>) -> DaemonError {
    DaemonError::ManagedContext {
        code: "owner_managed_context_unauthorized",
        operation: "owner-managed context transfer",
        message: message.into(),
        retryable: false,
    }
}

// MP-08/MP-11: expose only allowlisted refusal reasons, never raw Cloud errors.
pub(crate) fn cloud_admission_error(error: DaemonError) -> DaemonError {
    let (code, message, retryable) =
        match crate::runtime::cloud_api_client::cloud_error_code(&error) {
            Some("source_offline" | "source_stale") => (
                "owner_context_source_offline",
                "Source kernel is offline or stale",
                true,
            ),
            Some("source_user_mismatch") => (
                "owner_context_foreign_user",
                "This source kernel belongs to another user",
                false,
            ),
            Some("source_account_mismatch") => (
                "owner_context_foreign_account",
                "This source kernel belongs to another account",
                false,
            ),
            Some("target_offline" | "target_stale") => (
                "owner_context_target_offline",
                "Target kernel is offline or stale",
                true,
            ),
            _ => (
                "owner_managed_context_unauthorized",
                "Cloud refused owner-managed source/target authorization",
                crate::runtime::cloud_api_client::cloud_error_is_retryable(&error),
            ),
        };
    DaemonError::ManagedContext {
        code,
        operation: "reauthorize owner-managed transfer",
        message: message.into(),
        retryable,
    }
}

// MP-11: an owner's workspace root must not redirect publication through links.
pub(crate) fn target_context_parent() -> Result<std::path::PathBuf, DaemonError> {
    use std::fs;
    use std::path::{Component, PathBuf};
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| admission_error("target owner HOME is unavailable"))?;
    let mut ancestor = PathBuf::new();
    for component in home.components() {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            return Err(admission_error(
                "target owner HOME must be a real absolute directory",
            ));
        }
        ancestor.push(component);
        let metadata = fs::symlink_metadata(&ancestor)
            .map_err(|_| admission_error("target owner HOME is unavailable"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(admission_error("target owner HOME must not contain links"));
        }
    }
    let parent = home.join("chariox-contexts");
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(&parent) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(admission_error("owner context root is unavailable")),
    }
    let metadata = fs::symlink_metadata(&parent)
        .map_err(|_| admission_error("owner context root is unavailable"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(admission_error(
            "owner context root must be a real directory",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(admission_error(
                "owner context root must be private and owned by this kernel user",
            ));
        }
    }
    Ok(parent)
}

pub(crate) async fn authorize_import_ticket(
    config: &DaemonConfig,
    source: &chariox_relay::protocol::RelayCallerIdentity,
    source_kernel_id: &str,
    plan: &ManagedContextPlanBinding,
    destination: &OwnerManagedDestination,
) -> Result<(), DaemonError> {
    validate_credential_free_plan(plan)?;
    let profile = config
        .cloud_relay
        .as_ref()
        .ok_or_else(|| admission_error("target kernel has no enrolled owner"))?;
    let key = crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key);
    if destination.machine_id() != config.host_machine_id
        || destination.kernel_id() != config.daemon_id
        || profile.machine_id.as_deref() != Some(destination.machine_id())
        || profile.kernel_id.as_deref() != Some(destination.kernel_id())
        || profile.kernel_public_key_thumbprint.as_deref() != Some(key.as_str())
        || source.user_id.as_deref() != Some(profile.user_id.as_str())
        || source.realm_id != profile.realm_id
    {
        return Err(admission_error(
            "owner-managed destination enrollment, account, user or key does not match",
        ));
    }
    let credential = profile
        .kernel_credential
        .as_deref()
        .ok_or_else(|| admission_error("target kernel has no enrollment credential"))?;
    let ticket: ManagedContextTransferTicket = crate::runtime::cloud_api_client::post_cloud_json(
        profile.api_url.clone(), TICKET_ENDPOINT,
        serde_json::json!({
            "admission": "target", "accountId": profile.account_id, "userId": profile.user_id,
            "machineId": profile.machine_id, "kernelId": config.daemon_id,
            "relayRealmId": profile.realm_id, "keyThumbprint": key, "kernelCredential": credential,
            "contextId": plan.context_id, "planDigest": plan.plan_digest, "destination": destination,
            "source": { "kernelId": source_kernel_id, "userId": source.user_id, "relayRealmId": source.realm_id, "keyThumbprint": source.public_key_thumbprint },
        }),
    ).await.map_err(cloud_admission_error)?;
    let binding = ticket.context_plan.package_binding();
    let pins = &ticket.target;
    let source_binding = ticket
        .context_plan
        .source_binding()
        .ok_or_else(|| admission_error("owner context ticket has no source"))?;
    ticket.context_plan.validate().map_err(admission_error)?;
    if !ticket.environment_id.is_empty()
        || &binding != plan
        || pins.machine_id != destination.machine_id()
        || pins.kernel_id != destination.kernel_id()
        || pins.relay_realm_id != profile.realm_id
        || pins.relay_public_key != config.relay_public_key
        || pins.key_thumbprint != key
        || source_binding.kernel_id != source_kernel_id
        || source_binding.relay_realm_id != source.realm_id
        || Some(source_binding.key_thumbprint) != source.public_key_thumbprint.as_deref()
    {
        return Err(admission_error(
            "Cloud owner context ticket does not match authenticated encrypted peers",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mp08_mp11_owner_destination_rejects_ambiguous_and_foreign_bindings() {
        let destination = OwnerManagedDestination::OwnerManagedMachine {
            machine_id: "machine".into(),
            kernel_id: "kernel".into(),
        };
        assert!(validate_destination_binding("", Some(&destination), "kernel").is_ok());
        assert!(validate_destination_binding("environment", None, "kernel").is_ok());
        for (environment, destination, kernel) in [
            ("", None, "kernel"),
            ("environment", Some(&destination), "kernel"),
            ("", Some(&destination), "foreign-kernel"),
        ] {
            assert!(validate_destination_binding(environment, destination, kernel).is_err());
        }
        let plan = ManagedContextPlanBinding {
            context_id: "context".into(),
            plan_digest: "digest".into(),
            destination: Some(destination),
            kernel_context: ManagedContextKernelSelection::SourceKernel,
            development: super::super::package::ManagedContextDevelopmentSelection::Empty,
            provider_accounts: ManagedContextProviderAccountSelection::None,
            git_credentials: ManagedContextGitCredentialSelection::None,
        };
        assert!(validate_credential_free_plan(&plan).is_err());
    }
    #[test]
    fn mp08_mp11_cloud_refusals_preserve_safe_reasons_and_retryability() {
        for (cloud_code, expected, retryable) in [
            ("source_stale", "Source kernel is offline or stale", true),
            (
                "source_user_mismatch",
                "This source kernel belongs to another user",
                false,
            ),
            (
                "source_account_mismatch",
                "This source kernel belongs to another account",
                false,
            ),
            (
                "dependency_unavailable",
                "Cloud refused owner-managed source/target authorization",
                true,
            ),
        ] {
            let error = cloud_admission_error(DaemonError::LocalTransport {
                operation: "cloud relay request",
                message: format!("cloud_api_code={cloud_code}: synthetic-private-value"),
            });
            assert!(!error.to_string().contains("synthetic-private-value"));
            let DaemonError::ManagedContext {
                message,
                retryable: actual,
                ..
            } = error
            else {
                panic!("wrong error type")
            };
            assert_eq!(message, expected);
            assert_eq!(actual, retryable);
        }
    }

    #[cfg(unix)]
    #[test]
    fn mp11_owner_context_root_rejects_workspace_and_home_links() {
        crate::test_support::isolated_env_test!();
        let _guard = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-owner-boundary-{:032x}",
            rand::random::<u128>()
        ));
        std::fs::create_dir_all(root.join("home")).unwrap();
        std::fs::create_dir_all(root.join("outside")).unwrap();
        std::env::set_var("HOME", root.join("home"));
        let parent = target_context_parent().unwrap();
        std::fs::remove_dir(&parent).unwrap();
        std::os::unix::fs::symlink(root.join("outside"), &parent).unwrap();
        assert!(target_context_parent().is_err());
        std::os::unix::fs::symlink(root.join("home"), root.join("linked-home")).unwrap();
        std::env::set_var("HOME", root.join("linked-home"));
        assert!(target_context_parent().is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
