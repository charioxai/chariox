//! MP-08 / MP-10 / MP-11: one normal structured envelope, assembled before provider-store locking.
use crate::{
    error::DaemonError,
    prompt_assembly::{PromptAssemblyMode, PromptAssemblyService, PromptEnvelope},
    provider::RuntimeProviderRun,
    session::PromptAttachment,
};
#[allow(
    clippy::too_many_arguments,
    reason = "Mirror the enqueue_structured_prompt_submit context arguments"
)]
pub(super) fn assemble(
    run: &RuntimeProviderRun,
    prompt: &str,
    hidden: &str,
    attachments: &[PromptAttachment],
    mode: PromptAssemblyMode,
    private_assessment_evidence: Option<crate::prompt_assembly::AssessmentEvidence>,
    steering: bool,
    origin_prompt_id: &str,
) -> Result<PromptEnvelope, DaemonError> {
    let assessment_only = mode == PromptAssemblyMode::AssessmentTurn;
    let mode = if assessment_only || mode == PromptAssemblyMode::MetaagentProviderTurn {
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
    let mut envelope = PromptAssemblyService::from_env()?
        .assemble_provider_turn(run, prompt, Some(&hidden), attachments.to_vec(), mode)?
        .with_steering(steering);
    if assessment_only {
        envelope.private_assessment_evidence = private_assessment_evidence;
    }
    Ok(envelope)
}
