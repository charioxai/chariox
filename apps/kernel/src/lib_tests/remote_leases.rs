use super::*;

mod history_recovery;
mod lease_lifecycle;
mod prompt_projection;
mod provider_account;
mod runtime_projection;
mod stale_completion;
mod turn_capacity;

// Relay submission opens the receipt before admission so the worker can record
// the different home and backing prompt IDs before the home accepts the turn.
fn submit_leased_prompt_with_receipt(
    app: &mut DaemonApp,
    leased_agent_id: &str,
    prompt: &str,
    context: crate::transport::relay_peer::RemoteGitTurnContext,
) -> Result<(String, PromptSubmissionOutcome), DaemonError> {
    let home_prompt_id = context.home_prompt_id.clone();
    let mut runtime = RemoteLeaseRuntime::new(app);
    runtime.begin_leased_prompt_receipt(leased_agent_id, &home_prompt_id)?;
    let (provider_run_id, outcome) = runtime.submit_leased_prompt_with_workflow_context(
        leased_agent_id,
        prompt,
        Vec::new(),
        None,
        Some(context),
        Vec::new(),
        None,
        crate::extension::RemoteExtensionManifest::default(),
    )?;
    runtime.update_leased_prompt_receipt(
        leased_agent_id,
        &home_prompt_id,
        crate::durable_state::worker_prompt_receipts::WorkerPromptReceiptPhase::Accepted,
        Some(&provider_run_id),
    )?;
    Ok((provider_run_id, outcome))
}
