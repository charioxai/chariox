use crate::error::DaemonError;
use crate::local::{
    GetPromptSettingRequest, ListPromptSettingsRequest, LocalDaemonRequest, LocalDaemonResponse,
    PreviewPromptSettingRequest, ResetAllPromptSettingsRequest, ResetPromptSettingRequest,
    UpdatePromptSettingRequest,
};
use crate::prompt_assembly::PromptTemplateRegistry;
use crate::runtime::command::{KernelCallerKind, KernelCommand, KernelCommandSource};
use std::collections::BTreeMap;

pub(crate) async fn execute_prompt_settings_request(
    command: &KernelCommand,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    ensure_prompt_settings_read_authorized(command)?;
    let registry = PromptTemplateRegistry::from_env();
    match request {
        LocalDaemonRequest::ListPromptSettings(ListPromptSettingsRequest) => {
            Ok(LocalDaemonResponse::PromptSettingsListed {
                settings: registry.list_settings()?,
            })
        }
        LocalDaemonRequest::GetPromptSetting(GetPromptSettingRequest { id }) => {
            Ok(LocalDaemonResponse::PromptSetting {
                setting: registry.read_setting(&id)?,
            })
        }
        LocalDaemonRequest::UpdatePromptSetting(UpdatePromptSettingRequest {
            id,
            markdown,
            expected_revision,
            expected_sha256,
        }) => {
            ensure_prompt_settings_mutation_authorized(command)?;
            Ok(LocalDaemonResponse::PromptSetting {
                setting: registry.update_setting_if_version(
                    &id,
                    &markdown,
                    expected_revision,
                    &expected_sha256,
                )?,
            })
        }
        LocalDaemonRequest::PreviewPromptSetting(PreviewPromptSettingRequest { id, variables }) => {
            let setting = registry.read_setting(&id)?;
            Ok(LocalDaemonResponse::PromptSettingPreview {
                id,
                markdown: render_preview(&setting.current, &variables),
                variables,
            })
        }
        LocalDaemonRequest::ResetPromptSetting(ResetPromptSettingRequest { id }) => {
            ensure_prompt_settings_mutation_authorized(command)?;
            Ok(LocalDaemonResponse::PromptSetting {
                setting: registry.reset_setting(&id)?,
            })
        }
        LocalDaemonRequest::ResetAllPromptSettings(ResetAllPromptSettingsRequest) => {
            ensure_prompt_settings_mutation_authorized(command)?;
            Ok(LocalDaemonResponse::PromptSettingsReset {
                settings: registry.reset_all_settings()?,
            })
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "prompt settings request",
            message: "unsupported prompt settings request".to_string(),
        }),
    }
}

fn configured_prompt_settings_admins() -> Vec<String> {
    let configured_admins = std::env::var("CHARIOX_PROMPT_SETTINGS_ADMIN_USER_IDS")
        .ok()
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    configured_admins
}

fn ensure_prompt_settings_read_authorized(command: &KernelCommand) -> Result<(), DaemonError> {
    let trusted_local = matches!(
        command.source,
        KernelCommandSource::LocalCli
            | KernelCommandSource::LocalIpc
            | KernelCommandSource::DaemonBackground
    ) && matches!(command.caller.caller_kind, KernelCallerKind::LocalClient);
    if trusted_local
        || command
            .caller
            .user_id
            .as_deref()
            .is_some_and(|id| !id.trim().is_empty())
    {
        return Ok(());
    }
    Err(DaemonError::LocalTransport {
        operation: "prompt settings authorization",
        message: "prompt settings require an authenticated kernel caller".to_string(),
    })
}

fn ensure_prompt_settings_mutation_authorized(command: &KernelCommand) -> Result<(), DaemonError> {
    let configured_admins = configured_prompt_settings_admins();
    ensure_prompt_settings_mutation_authorized_with_admins(command, &configured_admins)
}

fn ensure_prompt_settings_mutation_authorized_with_admins(
    command: &KernelCommand,
    configured_admins: &[String],
) -> Result<(), DaemonError> {
    let trusted_local = matches!(
        command.source,
        KernelCommandSource::LocalCli
            | KernelCommandSource::LocalIpc
            | KernelCommandSource::DaemonBackground
    ) && matches!(command.caller.caller_kind, KernelCallerKind::LocalClient);
    let authenticated_admin = command
        .caller
        .user_id
        .as_deref()
        .is_some_and(|id| configured_admins.iter().any(|admin| admin == id));
    if trusted_local || authenticated_admin {
        return Ok(());
    }
    Err(DaemonError::LocalTransport {
        operation: "prompt settings authorization",
        message: "prompt settings require a local caller or configured kernel administrator"
            .to_string(),
    })
}

fn render_preview(markdown: &str, variables: &BTreeMap<String, String>) -> String {
    variables
        .iter()
        .fold(markdown.to_string(), |body, (key, value)| {
            body.replace(&format!("{{{{{key}}}}}"), value)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::ListPromptSettingsRequest;
    use crate::runtime::command::{KernelCaller, KernelCommandSource};

    fn command(source: KernelCommandSource, caller: KernelCaller) -> KernelCommand {
        KernelCommand::from_local_request_with_caller(
            "prompt-settings-test",
            source,
            caller,
            None,
            None,
            &LocalDaemonRequest::ListPromptSettings(ListPromptSettingsRequest),
        )
    }

    #[test]
    fn local_prompt_settings_access_is_trusted() {
        let command = command(
            KernelCommandSource::LocalCli,
            KernelCaller::for_source(&KernelCommandSource::LocalCli),
        );
        assert!(ensure_prompt_settings_read_authorized(&command).is_ok());
        assert!(ensure_prompt_settings_mutation_authorized_with_admins(&command, &[]).is_ok());
    }

    #[test]
    fn remote_prompt_settings_access_requires_user_identity() {
        let source = KernelCommandSource::RelayClient;
        let anonymous = command(source.clone(), KernelCaller::for_source(&source));
        assert!(ensure_prompt_settings_read_authorized(&anonymous).is_err());

        let mut caller = KernelCaller::for_source(&source);
        caller.user_id = Some("user-1".to_string());
        assert!(
            ensure_prompt_settings_read_authorized(&command(source.clone(), caller.clone()))
                .is_ok()
        );
        assert!(ensure_prompt_settings_mutation_authorized_with_admins(
            &command(source, caller.clone()),
            &["user-1".to_string()],
        )
        .is_ok());
        assert!(ensure_prompt_settings_mutation_authorized_with_admins(
            &command(
                KernelCommandSource::RelayClient,
                KernelCaller {
                    user_id: Some("user-2".to_string()),
                    ..caller
                },
            ),
            &["user-1".to_string()],
        )
        .is_err());
    }
}
