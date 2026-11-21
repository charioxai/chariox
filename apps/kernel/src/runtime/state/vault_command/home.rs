//! An agent on a leased worker reaches the Vault of its home kernel: the worker forwards the
//! Vault tools there and the home runs them for the home agent the run stands for. A capture
//! splits at the value: the worker runs the command or reads the file in its workspace, the
//! home unlocks the Vault before and stores the value after.
use super::capture::{captured_handle, check_capture_request, Captured};
use super::copy::parse_copy_args;
use super::*;
use crate::transport::relay_peer::{
    HomeVaultCapture, RelayPeerRequest, RelayPeerResponse, RemoteCredentialSecretInput,
    RemoteExtensionInvocationContext,
};
use crate::transport::runtime_tools::RuntimeToolResult;

/// The home may wait on an unlock card (300 s) before it answers.
const HOME_VAULT_RESPONSE_TIMEOUT: Duration = Duration::from_secs(345);

impl KernelRuntimeState {
    /// Runs a Vault tool at the home kernel, with the tool's own arguments.
    pub(in crate::runtime::state) async fn forward_vault_tool_to_home(
        &self,
        home: RemoteExtensionInvocationContext,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .send_vault_request_to_home(
                &home,
                RelayPeerRequest::InvokeHomeCredentialTool {
                    context: home.clone(),
                    tool_name: tool_name.to_string(),
                    arguments,
                },
            )
            .await?;
        match response {
            RelayPeerResponse::HomeCredentialToolHandled { result } => Ok(result),
            other => Err(unexpected_home_answer(&other)),
        }
    }

    /// The first half of a capture: the home answers a retry with the committed entry, else
    /// unlocks its Vault and keeps it so until the value arrives.
    pub(super) async fn prepare_home_vault_capture(
        &self,
        home: &RemoteExtensionInvocationContext,
        request_id: &str,
        host: &str,
        use_binding: &crate::config::VaultUseBinding,
    ) -> Result<Option<RuntimeToolResult>, DaemonError> {
        let response = self
            .send_vault_request_to_home(
                home,
                RelayPeerRequest::CaptureForHomeVault {
                    context: home.clone(),
                    capture: HomeVaultCapture::Prepare {
                        request_id: request_id.to_string(),
                        host: host.to_string(),
                        use_binding: use_binding.clone(),
                    },
                },
            )
            .await?;
        match response {
            RelayPeerResponse::HomeVaultCapturePrepared { committed } => Ok(committed),
            other => Err(unexpected_home_answer(&other)),
        }
    }

    /// The second half of a capture: the home stores the value the worker captured.
    pub(super) async fn store_home_vault_capture(
        &self,
        home: &RemoteExtensionInvocationContext,
        captured: Captured,
        secret: &str,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .send_vault_request_to_home(
                home,
                RelayPeerRequest::CaptureForHomeVault {
                    context: home.clone(),
                    capture: HomeVaultCapture::Store {
                        request_id: captured.request_id,
                        host: captured.host,
                        label: captured.label,
                        use_binding: captured.use_binding,
                        secret: RemoteCredentialSecretInput::new(secret.to_string()),
                    },
                },
            )
            .await?;
        match response {
            RelayPeerResponse::HomeCredentialToolHandled { result } => Ok(result),
            other => Err(unexpected_home_answer(&other)),
        }
    }

    async fn send_vault_request_to_home(
        &self,
        home: &RemoteExtensionInvocationContext,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let config = self.owned.config_projection.snapshot();
        let timeout = std::cmp::max(
            Duration::from_millis(config.relay_request_timeout_ms),
            HOME_VAULT_RESPONSE_TIMEOUT,
        );
        self.send_worker_home_runtime_request(&config, &home.home_kernel_id, request, timeout)
            .await
    }

    /// Who a Vault change at this home is made for, once the home agrees the leased run is its own.
    fn home_vault_actor(
        &self,
        home: &RemoteExtensionInvocationContext,
    ) -> Result<VaultActor, DaemonError> {
        let agent = self.authorize_home_credential_context(home)?;
        let session = self
            .owned
            .session_store
            .get_session(&home.home_session_id)?;
        if self.provider_account_authority_owner_user_id(session.owner_user_id())
            != crate::session::DEFAULT_LOCAL_USER_ID
        {
            return Err(error(
                "this Vault operation needs the owner's own kernel: the owner adds or removes Vault entries there",
            ));
        }
        Ok(VaultActor {
            agent_id: agent.id().to_string(),
            session_id: home.home_session_id.clone(),
            owner_user_id: session.owner_user_id().to_string(),
            provider_run_id: home.worker_provider_run_id.clone(),
        })
    }

    /// The home end of a forwarded Vault tool that needs nothing from the worker's workspace.
    pub(crate) async fn dispatch_forwarded_home_vault_tool_call(
        &self,
        home: RemoteExtensionInvocationContext,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let actor = self.home_vault_actor(&home)?;
        // The home owns settlement even if the worker's request goes away.
        let state = self.clone();
        tokio::spawn(async move {
            let recheck = || state.authorize_home_credential_context(&home).map(|_| ());
            match tool_name.as_str() {
                super::super::vault_generate::VAULT_GENERATE => {
                    state.generate_in_vault(&actor, &recheck, arguments).await
                }
                VAULT_REMOVE_CAPTURE => state.remove_from_vault(&actor, &recheck, arguments).await,
                VAULT_COPY_FOR_USE => {
                    state
                        .copy_in_vault(&actor, &recheck, &|_| Ok(()), parse_copy_args(arguments)?)
                        .await
                }
                other => Err(error(format!("`{other}` is not a forwarded Vault tool"))),
            }
        })
        .await
        .map_err(|_| error("the Vault runner stopped unexpectedly"))?
    }

    /// The home end of a capture on a leased worker.
    pub(crate) async fn handle_forwarded_home_vault_capture(
        &self,
        home: RemoteExtensionInvocationContext,
        capture: HomeVaultCapture,
    ) -> Result<HomeVaultCaptureAnswer, DaemonError> {
        let actor = self.home_vault_actor(&home)?;
        let state = self.clone();
        tokio::spawn(async move {
            let recheck = || state.authorize_home_credential_context(&home).map(|_| ());
            match capture {
                HomeVaultCapture::Prepare {
                    request_id,
                    host,
                    use_binding,
                } => {
                    check_capture_request(&request_id, None)?;
                    let committed = state
                        .prepare_capture_in_vault(
                            &actor,
                            &recheck,
                            &request_id,
                            &host,
                            &use_binding,
                        )
                        .await?;
                    Ok(HomeVaultCaptureAnswer::Prepared { committed })
                }
                HomeVaultCapture::Store {
                    request_id,
                    host,
                    label,
                    use_binding,
                    secret,
                } => {
                    check_capture_request(&request_id, label.as_deref())?;
                    use_binding.target.validate()?;
                    let host = service_host(&host)?;
                    let secret = secret.into_zeroizing();
                    let result = state
                        .store_capture_in_vault(
                            &actor,
                            &recheck,
                            Captured {
                                handle: captured_handle(&actor, &request_id),
                                host,
                                label,
                                use_binding,
                                request_id,
                            },
                            &secret,
                        )
                        .await?;
                    Ok(HomeVaultCaptureAnswer::Stored { result })
                }
            }
        })
        .await
        .map_err(|_| error("the Vault runner stopped unexpectedly"))?
    }
}

/// What the home answers a capture request with.
pub(crate) enum HomeVaultCaptureAnswer {
    Prepared {
        committed: Option<RuntimeToolResult>,
    },
    Stored {
        result: RuntimeToolResult,
    },
}

fn unexpected_home_answer(response: &RelayPeerResponse) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "remote Vault proxy",
        message: format!("unexpected home Vault response: {response:?}"),
    }
}
