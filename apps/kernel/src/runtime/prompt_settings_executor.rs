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
    ensure_prompt_settings_authorized(command)?;
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
        LocalDaemonRequest::UpdatePromptSetting(UpdatePromptSettingRequest { id, markdown }) => {
            Ok(LocalDaemonResponse::PromptSetting {
                setting: registry.update_setting(&id, &markdown)?,
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
            Ok(LocalDaemonResponse::PromptSetting {
                setting: registry.reset_setting(&id)?,
            })
        }
        LocalDaemonRequest::ResetAllPromptSettings(ResetAllPromptSettingsRequest) => {
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

fn ensure_prompt_settings_authorized(command: &KernelCommand) -> Result<(), DaemonError> {
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
        assert!(ensure_prompt_settings_authorized(&command).is_ok());
    }

    #[test]
    fn remote_prompt_settings_access_requires_user_identity() {
        let source = KernelCommandSource::RelayClient;
        let anonymous = command(source.clone(), KernelCaller::for_source(&source));
        assert!(ensure_prompt_settings_authorized(&anonymous).is_err());

        let mut caller = KernelCaller::for_source(&source);
        caller.user_id = Some("user-1".to_string());
        assert!(ensure_prompt_settings_authorized(&command(source, caller)).is_ok());
    }
}
