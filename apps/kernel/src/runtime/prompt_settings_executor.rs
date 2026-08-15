use crate::error::DaemonError;
use crate::local::{
    GetPromptSettingRequest, ListPromptSettingsRequest, LocalDaemonRequest, LocalDaemonResponse,
    PreviewPromptSettingRequest, ResetAllPromptSettingsRequest, ResetPromptSettingRequest,
    UpdatePromptSettingRequest,
};
use crate::prompt_assembly::PromptTemplateRegistry;
use std::collections::BTreeMap;

pub(crate) async fn execute_prompt_settings_request(
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
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

fn render_preview(markdown: &str, variables: &BTreeMap<String, String>) -> String {
    variables
        .iter()
        .fold(markdown.to_string(), |body, (key, value)| {
            body.replace(&format!("{{{{{key}}}}}"), value)
        })
}
