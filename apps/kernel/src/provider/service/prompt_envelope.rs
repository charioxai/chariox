//! MP-08 / MP-10 / MP-11: one normal structured envelope, assembled before provider-store locking.
use crate::{
    error::DaemonError,
    prompt_assembly::{PromptAssemblyMode, PromptAssemblyService, PromptEnvelope},
    provider::RuntimeProviderRun,
    session::PromptAttachment,
};
pub(super) fn assemble(
    run: &RuntimeProviderRun,
    prompt: &str,
    hidden: &str,
    attachments: &[PromptAttachment],
    mode: PromptAssemblyMode,
    steering: bool,
    origin_prompt_id: &str,
) -> Result<PromptEnvelope, DaemonError> {
    let mode = if mode == PromptAssemblyMode::MetaagentProviderTurn {
        mode
    } else if run.client_interface().is_chariox() {
        PromptAssemblyMode::NormalProviderTurn
    } else {
        PromptAssemblyMode::NativeTuiProviderTurn
    };
    let hidden = format!(
        "{hidden}\n\n{}",
        crate::prompt_assembly::agent_message_origin_context(origin_prompt_id)
    );
    Ok(PromptAssemblyService::from_env()?
        .assemble_provider_turn(run, prompt, Some(&hidden), attachments.to_vec(), mode)?
        .with_steering(steering))
}
