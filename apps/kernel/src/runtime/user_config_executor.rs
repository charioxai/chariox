use crate::error::DaemonError;
use crate::local::{
    DeleteCredentialSecretRequest, GetUserConfigRequest, GetUserConfigSchemaRequest,
    LocalDaemonRequest, LocalDaemonResponse, SetCredentialSecretRequest, SetUserConfigValueRequest,
    SetWorkspaceLiveSyncModeRequest, UnsetUserConfigValueRequest, UserConfigMutationEffect,
};
use crate::runtime::command::{command_caller_user_id, KernelCommand};
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::runtime::state::KernelRuntimeState;
use crate::runtime::user_config_policy::{user_config_mutation_effects, UserConfigMutation};

pub(crate) async fn execute_user_config_request(
    config_projection: &DaemonConfigProjectionStore,
    runtime_state: &KernelRuntimeState,
    command: &KernelCommand,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::GetUserConfig(request) => {
            execute_get_user_config_request(config_projection, request).await
        }
        LocalDaemonRequest::GetUserConfigSchema(request) => {
            execute_get_user_config_schema_request(request).await
        }
        LocalDaemonRequest::SetUserConfigValue(request) => {
            execute_set_user_config_value_request(runtime_state, request).await
        }
        LocalDaemonRequest::SetWorkspaceLiveSyncMode(request) => {
            execute_set_workspace_live_sync_mode_request(runtime_state, command, request).await
        }
        LocalDaemonRequest::UnsetUserConfigValue(request) => {
            execute_unset_user_config_value_request(runtime_state, request).await
        }
        LocalDaemonRequest::SetCredentialSecret(request) => {
            execute_set_credential_secret_request(config_projection, request).await
        }
        LocalDaemonRequest::DeleteCredentialSecret(request) => {
            execute_delete_credential_secret_request(config_projection, request).await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "user config request",
            message: "unsupported user config request".to_string(),
        }),
    }
}

pub(crate) async fn execute_get_user_config_request(
    config_projection: &DaemonConfigProjectionStore,
    _request: GetUserConfigRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let config = config_projection.snapshot();
    Ok(LocalDaemonResponse::UserConfig {
        path: config.user_config_path().clone(),
        config: config.user_config,
    })
}

pub(crate) async fn execute_get_user_config_schema_request(
    _request: GetUserConfigSchemaRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    Ok(LocalDaemonResponse::UserConfigSchema {
        entries: crate::config::DaemonConfig::user_config_schema(),
    })
}

pub(crate) async fn execute_set_user_config_value_request(
    runtime_state: &KernelRuntimeState,
    request: SetUserConfigValueRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let (config, effects) = apply_user_config_mutation(
        runtime_state,
        UserConfigMutation::Set {
            path: request.path,
            value: request.value,
        },
    )
    .await?;
    Ok(LocalDaemonResponse::UserConfigUpdated {
        path: config.user_config_path().clone(),
        config: config.user_config,
        effects,
    })
}

pub(crate) async fn execute_set_workspace_live_sync_mode_request(
    runtime_state: &KernelRuntimeState,
    command: &KernelCommand,
    request: SetWorkspaceLiveSyncModeRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let session = runtime_state.set_workspace_live_sync_mode(
        &request.session_id,
        request.mode,
        &command_caller_user_id(command),
        Some(command),
    )?;
    Ok(LocalDaemonResponse::WorkspaceLiveSyncModeUpdated { session })
}

pub(crate) async fn execute_unset_user_config_value_request(
    runtime_state: &KernelRuntimeState,
    request: UnsetUserConfigValueRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let (config, effects) = apply_user_config_mutation(
        runtime_state,
        UserConfigMutation::Unset { path: request.path },
    )
    .await?;
    Ok(LocalDaemonResponse::UserConfigUpdated {
        path: config.user_config_path().clone(),
        config: config.user_config,
        effects,
    })
}

async fn apply_user_config_mutation(
    runtime_state: &KernelRuntimeState,
    mutation: UserConfigMutation,
) -> Result<(crate::config::DaemonConfig, Vec<UserConfigMutationEffect>), DaemonError> {
    let changed_path = match &mutation {
        UserConfigMutation::Set { path, .. } | UserConfigMutation::Unset { path } => {
            path.trim().to_string()
        }
    };
    let config = match mutation {
        UserConfigMutation::Set { path, value } => {
            runtime_state.set_user_config_value(path, value).await?
        }
        UserConfigMutation::Unset { path } => runtime_state.unset_user_config_value(path).await?,
    };
    let effects = user_config_mutation_effects(runtime_state, &changed_path).await?;
    Ok((config, effects))
}

pub(crate) async fn execute_set_credential_secret_request(
    config_projection: &DaemonConfigProjectionStore,
    request: SetCredentialSecretRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let user_config = config_projection.snapshot().user_config;
    let service = crate::secret::RuntimeSecretService::with_vault_config(
        Vec::new(),
        &user_config.credential_vault,
    )?;
    service.set_vault_secret(&request.key, &request.value)?;
    Ok(LocalDaemonResponse::CredentialSecretStored { key: request.key })
}

pub(crate) async fn execute_delete_credential_secret_request(
    config_projection: &DaemonConfigProjectionStore,
    request: DeleteCredentialSecretRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let user_config = config_projection.snapshot().user_config;
    let service = crate::secret::RuntimeSecretService::with_vault_config(
        Vec::new(),
        &user_config.credential_vault,
    )?;
    service.delete_vault_secret(&request.key)?;
    Ok(LocalDaemonResponse::CredentialSecretDeleted { key: request.key })
}
