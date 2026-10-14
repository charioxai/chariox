//! MP-08 / MP-10 / MP-11: worker prompt admission, durable receipts and shared dispatch handoff.
use super::*;
use crate::app::RemoteLeaseRuntime;
use crate::durable_state::worker_prompt_receipts::WorkerPromptReceiptPhase;
use crate::execution_lease::RemoteWorkflowTurnContext;
use crate::transport::relay_peer::{
    RelayPromptAttachment, RemoteGitTurnContext, RequiredRemoteMcp,
    REMOTE_PROVIDER_LAUNCH_CREDENTIAL_REQUIRED_CODE,
};

impl KernelRuntimeState {
    async fn update_relay_leased_prompt_admission_receipt(
        &self,
        leased_agent_id: &str,
        home_prompt_id: &str,
        phase: WorkerPromptReceiptPhase,
        worker_provider_run_id: Option<String>,
    ) -> Result<(), DaemonError> {
        let leased_agent_id = leased_agent_id.to_string();
        let home_prompt_id = home_prompt_id.to_string();
        self.with_app_side_effect(move |app| {
            let mut runtime = RemoteLeaseRuntime::new(app);
            runtime.begin_leased_prompt_receipt(&leased_agent_id, &home_prompt_id)?;
            runtime.update_leased_prompt_receipt(
                &leased_agent_id,
                &home_prompt_id,
                phase,
                worker_provider_run_id.as_deref(),
            )
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn submit_relay_leased_prompt(
        &self,
        leased_agent_id: &str,
        expected_profile: crate::transport::relay_peer::RelayAgentExecutionProfile,
        prompt: &str,
        hidden_system_context: &str,
        attachments: Vec<RelayPromptAttachment>,
        workflow_context: Option<RemoteWorkflowTurnContext>,
        git_context: Option<RemoteGitTurnContext>,
        required_mcps: Vec<RequiredRemoteMcp>,
        required_skills: Option<Vec<crate::transport::relay_peer::RequiredRemoteSkill>>,
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
        provider_launch_credential: Option<
            crate::transport::relay_peer::RemoteProviderLaunchCredential,
        >,
    ) -> Result<(String, crate::session::PromptSubmissionOutcome), DaemonError> {
        let _operation = self.leased_agent_operations.lock(leased_agent_id).await;
        let leased_agent_id = leased_agent_id.to_string();
        let prompt = prompt.to_string();
        let hidden_system_context = hidden_system_context.to_string();
        let home_prompt_id = git_context
            .as_ref()
            .map(|context| context.home_prompt_id.clone())
            .filter(|prompt_id| !prompt_id.trim().is_empty());
        let begin_leased_agent_id = leased_agent_id.clone();
        let begin_home_prompt_id = home_prompt_id.clone();
        let begin_manifest = remote_extension_manifest.clone();
        let existing_receipt = self
            .with_app_side_effect(move |app| {
                let mut runtime = RemoteLeaseRuntime::new(app);
                runtime.consume_leased_agent_authorization(&begin_leased_agent_id)?;
                let Some(home_prompt_id) = begin_home_prompt_id.as_deref() else {
                    return Ok(None);
                };
                // A native provider that has not loaded the current runtime tool
                // catalog rejects before admission. Check it before a receipt
                // exists, so the home's bounded retry can resubmit this same
                // prompt after the refresh instead of meeting a rejection fence.
                if !runtime.leased_prompt_receipt_recorded(&begin_leased_agent_id, home_prompt_id) {
                    runtime.ensure_leased_native_runtime_catalog_loaded(
                        &begin_leased_agent_id,
                        &begin_manifest,
                    )?;
                }
                if runtime.leased_prompt_receipt_recorded(&begin_leased_agent_id, home_prompt_id) {
                    runtime.begin_leased_prompt_receipt(&begin_leased_agent_id, home_prompt_id)
                } else {
                    // Catalog and credential negotiation send no provider input.
                    // Reserve only once that negotiation is complete; otherwise
                    // the home's same-prompt credential retry meets a false fence.
                    Ok(None)
                }
            })
            .await;
        let existing_receipt = match existing_receipt {
            Err(
                error @ DaemonError::LocalTransport {
                    operation: "remote runtime tool catalog reload",
                    ..
                },
            ) if super::remote_prompt_worker_submission_runtime::remote_prompt_error_should_retry_transport(&error) => {
                // Prompt admission also repairs a missed manifest-sync message;
                // this only queues the same idle refresh, never the prompt.
                self.update_relay_leased_agent_remote_extension_manifest(
                    &leased_agent_id,
                    remote_extension_manifest.clone(),
                )
                .await?;
                record_leased_prompt_admission_failure(&error);
                return Err(error);
            }
            result => result?,
        };
        let new_receipt = home_prompt_id.is_some() && existing_receipt.is_none();
        if let Some(receipt) = existing_receipt.as_ref() {
            match receipt.receipt.phase {
                WorkerPromptReceiptPhase::Accepted => {}
                WorkerPromptReceiptPhase::Dispatching => {
                    return Err(DaemonError::LocalTransport {
                        operation: "submit remote leased prompt",
                        message: "earlier worker submission is unresolved; query its receipt before retrying"
                            .to_string(),
                    });
                }
                WorkerPromptReceiptPhase::Rejected => {
                    return Err(DaemonError::LocalTransport {
                        operation: "submit remote leased prompt",
                        message: "worker rejection receipt fences this home prompt from retry"
                            .to_string(),
                    });
                }
            }
        }

        let profile = expected_profile.clone();
        let replay_leased_agent_id = leased_agent_id.clone();
        let replay_git_context = git_context.clone();
        let profile_home_prompt_id = home_prompt_id.clone();
        let profile_receipt_is_new = new_receipt;
        let profile_replay = self
            .with_app_side_effect(move |app| {
                let mut runtime = RemoteLeaseRuntime::new(app);
                // A prior profile acknowledgement may have been lost. Home remains
                // authoritative, including when this is a retry of an active prompt.
                if let Err(error) = runtime.update_leased_agent_profile(
                    &replay_leased_agent_id,
                    profile.provider,
                    profile.account_profile,
                    profile.model,
                    profile.effort,
                ) {
                    if let Some(home_prompt_id) = profile_home_prompt_id
                        .as_deref()
                        .filter(|_| profile_receipt_is_new)
                    {
                        runtime
                            .begin_leased_prompt_receipt(&replay_leased_agent_id, home_prompt_id)?;
                        runtime.update_leased_prompt_receipt(
                            &replay_leased_agent_id,
                            home_prompt_id,
                            WorkerPromptReceiptPhase::Rejected,
                            None,
                        )?;
                    }
                    record_leased_prompt_admission_failure(&error);
                    return Err(error);
                }
                let replayed = match runtime.replay_active_leased_prompt_submission(
                    &replay_leased_agent_id,
                    replay_git_context.as_ref(),
                ) {
                    Ok(replayed) => replayed,
                    Err(error) => {
                        // Replay inspection has sent no new provider input. A
                        // failed lookup must settle its newly reserved receipt.
                        if let Some(home_prompt_id) = profile_home_prompt_id
                            .as_deref()
                            .filter(|_| profile_receipt_is_new)
                        {
                            runtime.begin_leased_prompt_receipt(
                                &replay_leased_agent_id,
                                home_prompt_id,
                            )?;
                            runtime.update_leased_prompt_receipt(
                                &replay_leased_agent_id,
                                home_prompt_id,
                                WorkerPromptReceiptPhase::Rejected,
                                None,
                            )?;
                        }
                        record_leased_prompt_admission_failure(&error);
                        return Err(error);
                    }
                };
                if profile_receipt_is_new {
                    if let (Some(home_prompt_id), Some((worker_provider_run_id, _))) =
                        (profile_home_prompt_id.as_deref(), replayed.as_ref())
                    {
                        runtime
                            .begin_leased_prompt_receipt(&replay_leased_agent_id, home_prompt_id)?;
                        runtime.update_leased_prompt_receipt(
                            &replay_leased_agent_id,
                            home_prompt_id,
                            WorkerPromptReceiptPhase::Accepted,
                            Some(worker_provider_run_id.as_str()),
                        )?;
                    }
                }
                Ok(replayed)
            })
            .await?;
        if let Some(replayed) = profile_replay {
            return Ok(replayed);
        }
        if existing_receipt.is_some() {
            return Err(DaemonError::LocalTransport {
                operation: "submit remote leased prompt",
                message: "accepted worker receipt cannot replay from current live state"
                    .to_string(),
            });
        }

        let refresh_lease = leased_agent_id.clone();
        let refresh_manifest = remote_extension_manifest.clone();
        let leased_agent_for_prepare = leased_agent_id.clone();
        let prompt_for_prepare = prompt.clone();
        let context_for_prepare = home_prompt_id.clone();
        let prepared = match self
            .with_app_side_effect(move |app| {
                RemoteLeaseRuntime::new(app).prepare_leased_prompt_submission(
                    &leased_agent_for_prepare,
                    &prompt_for_prepare,
                    &hidden_system_context,
                    attachments,
                    workflow_context,
                    git_context,
                    required_mcps,
                    required_skills,
                    remote_extension_manifest,
                )
            })
            .await
        {
            Ok(prepared) => prepared,
            Err(error) => {
                if let Some(home_prompt_id) = context_for_prepare.as_deref().filter(|_| new_receipt)
                {
                    self.update_relay_leased_prompt_admission_receipt(
                        &leased_agent_id,
                        home_prompt_id,
                        WorkerPromptReceiptPhase::Rejected,
                        None,
                    )
                    .await?;
                }
                if matches!(
                    &error,
                    DaemonError::LocalTransport {
                        operation: "remote runtime tool catalog reload",
                        ..
                    }
                ) && super::remote_prompt_worker_submission_runtime::remote_prompt_error_should_retry_transport(&error)
                {
                    // Prompt admission also repairs a missed manifest-sync message.
                    // The preceding preparation validated the lease and collisions;
                    // this only queues the same idle refresh, never the prompt.
                    self.update_relay_leased_agent_remote_extension_manifest(
                        &refresh_lease,
                        refresh_manifest,
                    )
                    .await?;
                }
                record_leased_prompt_admission_failure(&error);
                return Err(error);
            }
        };
        if let crate::app::PreparedLeasedProviderRun::LaunchRequired(request) =
            &prepared.provider_run
        {
            if crate::provider::canonical_provider_family(&request.provider) == Some("claude")
                && provider_launch_credential.is_none()
            {
                return Err(DaemonError::LocalTransport {
                        operation: "launch remote provider without credential",
                        message: format!(
                            "{REMOTE_PROVIDER_LAUNCH_CREDENTIAL_REQUIRED_CODE}: the worker must relaunch the selected Claude profile",
                        ),
                    });
            }
        }
        if let Some(home_prompt_id) = home_prompt_id.as_deref().filter(|_| new_receipt) {
            self.update_relay_leased_prompt_admission_receipt(
                &leased_agent_id,
                home_prompt_id,
                WorkerPromptReceiptPhase::Dispatching,
                None,
            )
            .await?;
        }
        let provider_run_id = match &prepared.provider_run {
            crate::app::PreparedLeasedProviderRun::Ready(provider_run_id) => {
                provider_run_id.clone()
            }
            crate::app::PreparedLeasedProviderRun::LaunchRequired(request) => {
                match self
                    .launch_provider_for_remote_lease_detached(
                        request.clone(),
                        provider_launch_credential,
                    )
                    .await
                {
                    Ok(run) => run.id().to_string(),
                    Err(error) => {
                        if let Some(home_prompt_id) =
                            home_prompt_id.as_deref().filter(|_| new_receipt)
                        {
                            self.update_relay_leased_prompt_admission_receipt(
                                &leased_agent_id,
                                home_prompt_id,
                                WorkerPromptReceiptPhase::Rejected,
                                None,
                            )
                            .await?;
                        }
                        record_leased_prompt_admission_failure(&error);
                        return Err(error);
                    }
                }
            }
        };
        let finish_leased_agent_id = leased_agent_id.clone();
        let finish_home_prompt_id = home_prompt_id.clone();
        let finish_provider_run_id = provider_run_id.clone();
        let (provider_run_id, outcome, dispatch) = self
            .with_app_side_effect(move |app| {
                let mut runtime = RemoteLeaseRuntime::new(app);
                let result =
                    runtime.admit_prepared_leased_prompt_submission(prepared, provider_run_id);
                if let Some(home_prompt_id) =
                    finish_home_prompt_id.as_deref().filter(|_| new_receipt)
                {
                    // Admission and its receipt precede provider I/O. An
                    // admitted dispatch failure is then an exact Failed turn,
                    // projected through the usual worker completion path.
                    runtime.update_leased_prompt_receipt(
                        &finish_leased_agent_id,
                        home_prompt_id,
                        if result.is_ok() {
                            WorkerPromptReceiptPhase::Accepted
                        } else {
                            WorkerPromptReceiptPhase::Rejected
                        },
                        result
                            .as_ref()
                            .ok()
                            .map(|_| finish_provider_run_id.as_str()),
                    )?;
                }
                result
            })
            .await?;
        if let Some(dispatch) = dispatch {
            self.spawn_prompt_dispatch(dispatch, self.provider_runtime_lanes.clone());
        }
        Ok((provider_run_id, outcome))
    }
}

// MP-08 / MP-10 / MP-11: report a source operation label, never provider payloads.
fn record_leased_prompt_admission_failure(error: &DaemonError) {
    let operation = match error {
        DaemonError::LocalTransport { operation, .. }
        | DaemonError::RelayTransport { operation, .. }
        | DaemonError::InternalInvariant { operation, .. } => *operation,
        _ => "worker prompt preparation",
    };
    crate::logging::warn_with_fields(
        "daemon.leased_prompt_admission",
        "worker prompt admission failed",
        serde_json::json!({"operation": operation}),
    );
}
