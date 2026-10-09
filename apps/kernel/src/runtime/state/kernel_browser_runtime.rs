//! MD-2/MD-3: routing and focused-agent MCP adapter for the host service.
use super::*;
use crate::local::KernelBrowserCommand;
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};

/// MD-3: no serde impl. Display owns the serialized protocol adapter/version.
pub(crate) enum KernelBrowserDisplayRequest {
    Capture {
        tab_id: String,
        generation: u64,
    },
    CaptureRegion {
        tab_id: String,
        generation: u64,
    },
    // Keep the protected legacy native stream seam for display adapters.
    #[allow(dead_code)]
    Subscribe {
        tab_id: String,
        generation: u64,
    },
    Input {
        binding: crate::runtime::kernel_browser_host::KernelBrowserDocumentBinding,
        input: crate::local::KernelBrowserInput,
    },
    Takeover {
        tab_id: String,
        generation: u64,
    },
    Release {
        tab_id: String,
        generation: u64,
    },
    Actors,
    // MD-DISPLAY-04: codec adapters on the same protected capture/subscribe seam.
    NegotiatedSubscribe {
        tab_id: String,
        generation: u64,
        codecs: Vec<String>,
        bitrate: u32,
        device_scale_factor: u32,
    },
    EncodedCapture {
        subscription_id: String,
        generation: u64,
        after_sequence: u64,
    },
    Attach {
        subscription_id: String,
        generation: u64,
    },
}

const LOADER: &str = "chariox.load_kernel_browser";
pub(super) const PASTE: &str = "chariox.kernel_browser_paste_secret";
const BROWSER: &str = "chariox.kernel_browser";
const SHARE: &str = "chariox.kernel_browser_share";

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LoaderArguments {
    #[serde(default)]
    lifetime_hours: Option<u64>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserToolArguments {
    command: KernelBrowserCommand,
    #[serde(default)]
    document_id: Option<String>,
}
fn browser_tool_params(arguments: serde_json::Value) -> Result<serde_json::Value, DaemonError> {
    let request: BrowserToolArguments = serde_json::from_value(arguments)
        .map_err(|_| host_error("MD-3: invalid browser command".into()))?;
    if matches!(
        &request.command,
        KernelBrowserCommand::ListGrants
            | KernelBrowserCommand::SubscribeGrants { .. }
            | KernelBrowserCommand::RevokeGrants { .. }
            | KernelBrowserCommand::MirrorSubscribe { .. }
            | KernelBrowserCommand::MirrorNext { .. }
            | KernelBrowserCommand::MirrorClose { .. }
            | KernelBrowserCommand::MirrorInput { .. }
            | KernelBrowserCommand::DisplayCapture { .. }
            | KernelBrowserCommand::DisplaySubscribe { .. }
            | KernelBrowserCommand::DisplayNext { .. }
            | KernelBrowserCommand::DisplayInput { .. }
            | KernelBrowserCommand::DisplayTakeover { .. }
            | KernelBrowserCommand::DisplayRelease { .. }
            | KernelBrowserCommand::DisplayActors
    ) {
        return Err(host_error(
            "MD-DISPLAY: terminal display adapter required".into(),
        ));
    }
    let input = matches!(&request.command, KernelBrowserCommand::Input { .. });
    if input
        && request
            .document_id
            .as_ref()
            .is_none_or(|id| id.is_empty() || id.len() > 256)
    {
        return Err(host_error(
            "MD-3: input requires document_id from the observed tab/snapshot/screenshot".into(),
        ));
    }
    let mut params = serde_json::to_value(request.command)
        .map_err(|_| host_error("MD-3: invalid command".into()))?;
    if input {
        params["document_id"] = serde_json::json!(request.document_id);
    }
    params["focused_agent"] = serde_json::json!(true);
    Ok(params)
}

impl KernelRuntimeState {
    #[cfg(test)]
    pub(crate) fn kernel_browser_profile_root(&self, user: &str) -> std::path::PathBuf {
        self.owned.kernel_browser_host.profile_root(user)
    }

    pub(crate) async fn kernel_browser_terminal_request(
        &self,
        caller: &crate::runtime::command::KernelCommand,
        command: KernelBrowserCommand,
    ) -> Result<serde_json::Value, DaemonError> {
        if matches!(
            &command,
            KernelBrowserCommand::MirrorSubscribe { .. }
                | KernelBrowserCommand::MirrorNext { .. }
                | KernelBrowserCommand::MirrorClose { .. }
                | KernelBrowserCommand::MirrorInput { .. }
        ) {
            return self.kernel_browser_mirror_request(caller, command).await;
        }
        let request = match command {
            KernelBrowserCommand::DisplayCapture { tab_id, generation } => {
                KernelBrowserDisplayRequest::Capture { tab_id, generation }
            }
            KernelBrowserCommand::DisplaySubscribe {
                tab_id,
                generation,
                codecs,
                bitrate,
                device_scale_factor,
            } => KernelBrowserDisplayRequest::NegotiatedSubscribe {
                tab_id,
                generation,
                codecs,
                bitrate,
                device_scale_factor,
            },
            KernelBrowserCommand::DisplayNext {
                subscription_id,
                generation,
                after_sequence,
            } => KernelBrowserDisplayRequest::EncodedCapture {
                subscription_id,
                generation,
                after_sequence,
            },
            KernelBrowserCommand::DisplayInput {
                tab_id,
                generation,
                document_id,
                input,
            } => KernelBrowserDisplayRequest::Input {
                binding: crate::runtime::kernel_browser_host::KernelBrowserDocumentBinding {
                    tab_id,
                    generation,
                    document_id,
                },
                input,
            },
            KernelBrowserCommand::DisplayTakeover { tab_id, generation } => {
                KernelBrowserDisplayRequest::Takeover { tab_id, generation }
            }
            KernelBrowserCommand::DisplayRelease { tab_id, generation } => {
                KernelBrowserDisplayRequest::Release { tab_id, generation }
            }
            KernelBrowserCommand::DisplayActors => KernelBrowserDisplayRequest::Actors,
            command => {
                let (user, actor) = self.kernel_browser_terminal_context(caller)?;
                self.refresh_user_domain_grants();
                match &command {
                    KernelBrowserCommand::ListGrants
                    | KernelBrowserCommand::RevokeGrants { .. }
                    | KernelBrowserCommand::SubscribeGrants { .. } => {
                        if let KernelBrowserCommand::RevokeGrants { agent_id } = &command {
                            self.owned
                                .kernel_browser_host
                                .revoke_grants(&user, agent_id.as_deref());
                        }
                        if let KernelBrowserCommand::SubscribeGrants { after, wait_ms } = command {
                            if wait_ms > 25_000 {
                                return Err(host_error(
                                    "MP-08: grant subscription wait exceeds 25000 ms".into(),
                                ));
                            }
                            let deadline = tokio::time::Instant::now()
                                + std::time::Duration::from_millis(wait_ms.into());
                            while self.owned.kernel_browser_host.grant_snapshot(&user, "")["cursor"]
                                .as_u64()
                                == Some(after)
                                && tokio::time::Instant::now() < deadline
                            {
                                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                                self.kernel_browser_terminal_context(caller)?;
                                self.refresh_user_domain_grants();
                            }
                        }
                        let kernel = self.owned.config_projection.snapshot().daemon_id;
                        return Ok(self
                            .owned
                            .kernel_browser_host
                            .grant_snapshot(&user, &kernel));
                    }
                    _ => {}
                }
                #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
                if matches!(command, KernelBrowserCommand::Stop) {
                    let views = self.app_control().user_views();
                    for view in self
                        .user_domain_owner_aliases(&user)
                        .iter()
                        .flat_map(|owner| views.browser_views(owner))
                    {
                        self.forget_user_app_view(&user, &view.view_id);
                    }
                }
                let mut params = serde_json::to_value(command)
                    .map_err(|_| host_error("MD-2: invalid command".into()))?;
                params["observed_by"] = serde_json::json!(actor);
                let admission = self
                    .owned
                    .kernel_browser_host
                    .admit_terminal(&user, caller.terminal_lifetime.clone().unwrap_or_default());
                let mut result = self
                    .kernel_browser_operation_admitted(
                        &user,
                        Some(admission),
                        "host.browser",
                        params,
                    )
                    .await?;
                if let Some(object) = result.as_object_mut() {
                    object.extend(
                        self.user_domain_window_projection(&user)
                            .as_object()
                            .unwrap()
                            .clone(),
                    );
                }
                return Ok(result);
            }
        };
        if std::env::var("CHARIOX_KERNEL_BROWSER_DISPLAY").as_deref() != Ok("1")
            && !(std::env::var("CHARIOX_KERNEL_BROWSER_MIRROR").as_deref() == Ok("1")
                && matches!(
                    &request,
                    KernelBrowserDisplayRequest::Takeover { .. }
                        | KernelBrowserDisplayRequest::Release { .. }
                        | KernelBrowserDisplayRequest::Actors
                ))
        {
            return Err(host_error(
                "MD-DISPLAY: experimental display disabled".into(),
            ));
        }
        Box::pin(self.kernel_browser_display_request(caller, request)).await
    }
    pub(crate) async fn kernel_browser_display_attach(
        &self,
        caller: &crate::runtime::command::KernelCommand,
        subscription_id: String,
        generation: u64,
    ) -> Result<serde_json::Value, DaemonError> {
        self.kernel_browser_display_request(
            caller,
            KernelBrowserDisplayRequest::Attach {
                subscription_id,
                generation,
            },
        )
        .await
    }
    pub(crate) fn kernel_browser_terminal_context(
        &self,
        caller: &crate::runtime::command::KernelCommand,
    ) -> Result<(String, String), DaemonError> {
        use sha2::{Digest, Sha256};
        if self.slice_kernel_id().is_some() {
            return Err(host_error(
                "MD-3: the home kernel owns the user-domain browser".into(),
            ));
        }
        if caller
            .terminal_lifetime
            .as_ref()
            .is_some_and(|lifetime| !lifetime.is_live())
        {
            return Err(host_error("MD-3: terminal connection closed".into()));
        }
        if !caller.is_terminal_caller() {
            return Err(host_error("MD-3: authenticated terminal required".into()));
        }
        let user = self.provider_account_authority_owner_user_id(
            &crate::runtime::command::command_caller_user_id(caller),
        );
        let bytes = serde_json::to_vec(&caller.caller)
            .map_err(|_| host_error("MD-3: invalid caller".into()))?;
        Ok((user, format!("terminal:{:x}", Sha256::digest(bytes))))
    }
    pub(crate) fn kernel_browser_terminal_disconnected(&self, user: &str, actor: &str) {
        self.owned
            .kernel_browser_host
            .disconnect_terminal(user, actor);
    }

    /// MD-3/display: typed internal seam for the coordinator-owned public adapter.
    /// Retains the capture/Vault barrier; never accepts a client-supplied user/actor.
    pub(crate) async fn kernel_browser_display_request(
        &self,
        caller: &crate::runtime::command::KernelCommand,
        request: KernelBrowserDisplayRequest,
    ) -> Result<serde_json::Value, DaemonError> {
        use crate::session::{EnvironmentActor, EnvironmentActorKind};
        let (user, actor) = self.kernel_browser_terminal_context(caller)?;
        let host = &self.owned.kernel_browser_host;
        let admission =
            host.admit_terminal(&user, caller.terminal_lifetime.clone().unwrap_or_default());
        let mut params = match request {
            KernelBrowserDisplayRequest::NegotiatedSubscribe {
                tab_id,
                generation,
                codecs,
                bitrate,
                device_scale_factor,
            } => {
                serde_json::json!({"op":"display_subscribe","tab_id":tab_id,"generation":generation,"codecs":codecs,"bitrate":bitrate,"device_scale_factor":device_scale_factor})
            }
            KernelBrowserDisplayRequest::EncodedCapture {
                subscription_id,
                generation,
                after_sequence,
            } => {
                serde_json::json!({"op":"screenshot","display_subscription_id":subscription_id,"generation":generation,"after_sequence":after_sequence,"bound_frames":true})
            }
            KernelBrowserDisplayRequest::Attach {
                subscription_id,
                generation,
            } => {
                serde_json::json!({"op":"display_attach","subscription_id":subscription_id,"generation":generation})
            }
            KernelBrowserDisplayRequest::Capture { tab_id, generation } => {
                serde_json::json!({"op":"screenshot","tab_id":tab_id,"generation":generation,"bound_frames":true})
            }
            KernelBrowserDisplayRequest::CaptureRegion { tab_id, generation } => {
                serde_json::json!({"op":"screenshot","tab_id":tab_id,"generation":generation,"bound_frames":true,"_capture_protection":true})
            }
            KernelBrowserDisplayRequest::Subscribe { tab_id, generation } => {
                serde_json::json!({"op":"subscribe","tab_id":tab_id,"generation":generation,"bound_frames":true})
            }
            KernelBrowserDisplayRequest::Input { binding, input } => {
                if binding.document_id.is_empty() || binding.document_id.len() > 256 {
                    return Err(host_error("MD-3: observed document required".into()));
                }
                serde_json::json!({"op":"input","tab_id":binding.tab_id,"generation":binding.generation,"document_id":binding.document_id,"input":input})
            }
            KernelBrowserDisplayRequest::Takeover { tab_id, generation } => {
                let outcome = host
                    .request_takeover_admitted(
                        &user,
                        EnvironmentActor::new(actor, EnvironmentActorKind::Human, "Human"),
                        &tab_id,
                        generation,
                        Some(&admission),
                    )
                    .map_err(host_error)?;
                return serde_json::to_value(outcome)
                    .map_err(|_| host_error("MD-3: invalid takeover outcome".into()));
            }
            KernelBrowserDisplayRequest::Release { tab_id, generation } => {
                host.release_input(&user, &actor, &tab_id, generation)
                    .map_err(host_error)?;
                return Ok(serde_json::json!({"released":true}));
            }
            KernelBrowserDisplayRequest::Actors => {
                return host.actor_snapshot(&user).map_err(host_error)
            }
        };
        params["observed_by"] = serde_json::json!(actor);
        self.kernel_browser_operation_admitted(&user, Some(admission), "host.browser", params)
            .await
    }
    /// MD-2/MD-5: appviews retains admission; observations use the same protection.
    pub(crate) async fn kernel_browser_app_view(
        &self,
        user: String,
        request: crate::runtime::browser_controller_app_view::BrowserAppViewRequest,
        generation: Option<u64>,
    ) -> Result<serde_json::Value, DaemonError> {
        let user = self.provider_account_authority_owner_user_id(&user);
        let mut params = request.params();
        if let Some(generation) = generation {
            params["_host_generation"] = generation.into();
        }
        self.kernel_browser_operation(&user, None, request.method(), params)
            .await
    }
    pub(super) fn kernel_browser_agent(
        &self,
        run: &crate::provider::RuntimeProviderRun,
    ) -> Option<crate::agent::AgentInstance> {
        self.user_domain_agent_authority(run).ok()
    }

    pub(super) fn kernel_browser_tool_specs(
        &self,
        runs: &[crate::provider::RuntimeProviderRun],
    ) -> Vec<RuntimeToolSpec> {
        self.refresh_user_domain_grants();
        let [run] = runs else {
            return Vec::new();
        };
        let loader = RuntimeToolSpec { name: LOADER.into(),
            description: "MD-3: load the user's kernel browser tools. Use only when the user asked you to use their browser in this turn, or after they focused you. The grant covers tabs you open (not their existing tabs) for lifetime_hours (default 8, max 24) and is retained until expiry, idle lapse or revocation.".into(),
            input_schema: serde_json::json!({"type":"object","properties":{"lifetime_hours":{"type":"integer","minimum":1,"maximum":24}},"additionalProperties":false}), };
        let Some(agent) = self.kernel_browser_agent(run) else {
            // MP-08/MP-11: an owner-requested turn may acquire access without focus.
            if self.room_agent_tools_enabled() && self.user_domain_agent_placement(run).is_ok() {
                return vec![loader];
            }
            return Vec::new();
        };
        let mut tools = vec![loader];
        if self.owned.kernel_browser_host.is_loaded(
            &self.provider_account_authority_owner_user_id(agent.owner_user_id()),
            agent.id(),
        ) {
            tools.push(super::kernel_browser_secret_runtime::paste_spec());
            if self.room_agent_tools_enabled() {
                tools.push(RuntimeToolSpec { name: SHARE.into(),
                    description: "MP-08: give your direct child agent an explicit subset of the browser/notes resources you were granted. It cannot open new tabs and is revoked with your grant.".into(),
                    input_schema: serde_json::json!({"type":"object","properties":{"agent":{"type":"string","minLength":1},"resources":{"type":"array","minItems":1,"maxItems":64,"items":{"type":"object","properties":{"kind":{"type":"string","enum":["browser_tab","note","app_view"]},"tab_id":{"type":"string"},"note_id":{"type":"string"},"view_id":{"type":"string"}},"required":["kind"]}}},"required":["agent","resources"],"additionalProperties":false}), });
            }
            tools.push(RuntimeToolSpec { name: BROWSER.into(),
                description: "MD-3: control the user's kernel browser outside sessions/slices. Read state/snapshot/screenshot to obtain tab IDs, generation and document_id; input requires the observed document_id beside command and accepts click/text/key/scroll. Vault input is separate.".into(),
                input_schema: serde_json::json!({"type":"object","properties":{"document_id":{"type":"string","minLength":1,"maxLength":256},"command":{"type":"object","properties":{"op":{"type":"string","enum":["start","state","stop","open","close","navigate","snapshot","input","screenshot","subscribe","poll","unsubscribe"]},"tab_id":{"type":"string"},"generation":{"type":"integer","minimum":1},"url":{"type":"string"},"subscription_id":{"type":"string"},"input":{"type":"object","properties":{"kind":{"type":"string","enum":["click","text","key","scroll"]},"x":{"type":"integer","minimum":0,"maximum":1279},"y":{"type":"integer","minimum":0,"maximum":799},"text":{"type":"string","maxLength":16384},"key":{"type":"string","enum":["Tab","Enter","Escape","Backspace","Delete","ArrowLeft","ArrowRight","ArrowUp","ArrowDown","Home","End"]},"delta_x":{"type":"integer","minimum":-10000,"maximum":10000},"delta_y":{"type":"integer","minimum":-10000,"maximum":10000}},"required":["kind"],"additionalProperties":false}},"required":["op"],"additionalProperties":false}},"required":["command"],"additionalProperties":false}), });
        }
        tools.extend(self.notes_tool_specs(run, &agent));
        tools
    }
    pub(super) async fn try_kernel_browser_tool(
        &self,
        token: &str,
        name: &str,
        arguments: serde_json::Value,
    ) -> Option<Result<RuntimeToolResult, DaemonError>> {
        if Self::is_note_tool(name) {
            return Some(Box::pin(self.notes_tool(token, name, arguments)).await);
        }
        if ![LOADER, BROWSER, PASTE, SHARE].contains(&name) {
            return None;
        }
        Some(self.kernel_browser_tool(token, name, arguments).await)
    }
    async fn kernel_browser_tool(
        &self,
        token: &str,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(token);
        let [run] = runs.as_slice() else {
            return Err(host_error(
                "MD-3: admitted local provider run required".into(),
            ));
        };
        let host = self.owned.kernel_browser_host.clone();
        if name == LOADER {
            // MP-08/MP-11 R2: keep the authenticated caller across the owner wait.
            return self
                .with_room_provider_origin(run.agent_instance_id(), Some(run.id()))
                .kernel_browser_loader(run, arguments)
                .await;
        }
        let agent = self.user_domain_tool_agent(run).await?;
        let user = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        // MP-08/MP-10/MP-11 A06: retain the exact sudo turn across waits.
        // Its browser grant permits unfocused input only while that turn is current.
        let sudo_turn = if name == PASTE && !host.is_focused(&user, agent.id()) {
            Some(self.sudo_for_auth_token(token).map_err(|_| host_error("MP-11: sensitive_requires_focus: Vault fill requires focus or human approval; ask the user to focus this agent".into()))?)
        } else {
            None
        };
        let authority = self.clone();
        let auth_token = token.to_string();
        let run_id = run.id().to_string();
        let sensitive = name == PASTE;
        let authority_turn = sudo_turn.clone();
        let authority_owner = user.clone();
        let authority_agent = agent.id().to_string();
        let mut admission = host
            .admit(&user, agent.id())
            .map_err(host_error)?
            .with_agent_label(agent.alias().unwrap_or(agent.agent_ref()))
            .with_authority(move || {
                let runs = authority
                    .owned
                    .provider_store
                    .get_runs_by_runtime_mcp_auth_token(&auth_token);
                let [current] = runs.as_slice() else {
                    return false;
                };
                current.id() == run_id
                    && authority.kernel_browser_agent(current).is_some()
                    && (!sensitive
                        || match &authority_turn {
                            Some(turn) => authority.sudo_turn_live(turn),
                            None => authority
                                .owned
                                .kernel_browser_host
                                .is_focused(&authority_owner, &authority_agent),
                        })
            });
        if sudo_turn.is_some() {
            admission = admission.elevated();
        }
        if name == SHARE {
            return self.kernel_browser_share(&agent, arguments, &admission);
        }
        if name == PASTE {
            let scope = crate::runtime::kernel_browser_host::KernelBrowserHost::profile_key(
                &self.provider_account_authority_owner_user_id(agent.owner_user_id()),
            );
            return self
                .kernel_browser_paste_secret(
                    run.session_id(),
                    &agent,
                    arguments,
                    admission,
                    sudo_turn.as_ref(),
                )
                .await
                .map_err(|error| {
                    self.owned
                        .kernel_browser_secret_observations
                        .scrub_error(&scope, error)
                });
        }
        let params = browser_tool_params(arguments)?;
        #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
        if let Some(tab) = params["tab_id"].as_str() {
            let views = self.app_control().user_views();
            for view in self
                .user_domain_owner_aliases(&user)
                .iter()
                .flat_map(|owner| views.browser_views(owner))
            {
                if view
                    .browser
                    .as_ref()
                    .is_some_and(|browser| browser.tab_id == tab)
                {
                    host.claim_resource(
                        Some(&admission),
                        crate::local::UserDomainResource::AppView {
                            view_id: view.view_id,
                        },
                        false,
                    )
                    .map_err(host_error)?;
                }
            }
        }
        let payload = self
            .kernel_browser_operation_admitted(&user, Some(admission), "host.browser", params)
            .await?;
        Ok(RuntimeToolResult { ok: true, payload })
    }
}
impl KernelRuntimeState {
    /// MP-08/MP-11: focus or the owner's running request loads browser tools;
    /// otherwise only an already-loaded grant is retained. Agents cannot self-grant.
    async fn kernel_browser_loader(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let arguments: LoaderArguments = serde_json::from_value(arguments)
            .map_err(|_| host_error("MD-3: loader accepts only lifetime_hours".into()))?;
        self.authorize_current_external_command()?;
        self.refuse_leased_user_domain_run(run).await?;
        self.refresh_user_domain_grants();
        let agent = self.user_domain_agent_placement(run)?;
        let host = &self.owned.kernel_browser_host;
        let user = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        if !host.is_focused(&user, agent.id()) && !host.is_loaded(&user, agent.id()) {
            let prompt = self.owner_requested_prompt(&agent).ok_or_else(|| {
                host_error(crate::runtime::user_domain_access::NOT_REQUESTED_LOAD.into())
            })?;
            let lifetime = crate::runtime::kernel_browser_host::KernelBrowserHost::grant_lifetime(
                arguments.lifetime_hours,
            )
            .map_err(host_error)?;
            let delivery = self.app_prompt_delivery(&agent);
            if delivery
                .as_ref()
                .is_none_or(|delivery| !delivery.belongs_to(run.id()))
            {
                return Err(super::capability_grant_runtime::refused(
                    crate::error::UserDomainRefusalReason::NotGranted,
                ));
            }
            let fence = host.acquisition_fence(&user, agent.id());
            if self
                .confirm_capability_request(&agent, "new tabs in your kernel browser", || {
                    host.acquisition_fence(&user, agent.id()) == fence
                        && self.app_prompt_delivery(&agent) == delivery
                })
                .await?
                != prompt
            {
                return Err(super::capability_grant_runtime::refused(
                    crate::error::UserDomainRefusalReason::NotRequested,
                ));
            }
            self.authorize_current_external_command()?;
            self.user_domain_agent_placement(run)?;
            if host.acquisition_fence(&user, agent.id()) != fence {
                return Err(super::capability_grant_runtime::refused(
                    crate::error::UserDomainRefusalReason::NotGranted,
                ));
            }
            let granted = host
                .request_grant(
                    &user,
                    agent.id(),
                    &prompt,
                    lifetime,
                    Some(fence),
                    agent.session_id(),
                )
                .map_err(host_error)?;
            return Ok(RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({"loaded": BROWSER, "granted": granted, "prompt_id": prompt}),
            });
        }
        if arguments.lifetime_hours.is_some() {
            return Err(host_error(
                "MP-08: lifetime_hours applies only to a new user-requested grant".into(),
            ));
        }
        if !host.is_loaded(&user, agent.id()) {
            self.user_domain_agent_authority(run)?;
            host.load(&user, agent.id()).map_err(host_error)?;
        }
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({"loaded": BROWSER}),
        })
    }
}

pub(super) fn host_error(message: String) -> DaemonError {
    crate::error::HostFailure::from(message).into_daemon("kernel_browser")
}

#[cfg(test)]
mod document_tests {
    use super::*;
    #[test]
    fn md3_local_terminals_have_distinct_stable_browser_actors() {
        use crate::runtime::command::{KernelCommand, KernelCommandSource};
        use crate::runtime::router::CommandRouter;
        let app =
            crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let state = router.runtime_state();
            let request = crate::local::LocalDaemonRequest::KernelBrowser(
                crate::local::KernelBrowserRequest {
                    command: KernelBrowserCommand::State,
                },
            );
            let context = |caller| {
                KernelCommand::from_local_request_with_caller(
                    "same-retry-id",
                    KernelCommandSource::LocalCli,
                    caller,
                    None,
                    None,
                    &request,
                )
            };
            let a = context(
                router
                    .local_terminal_caller(KernelCommandSource::LocalCli, "kernel-connection-a")
                    .await,
            );
            let b = context(
                router
                    .local_terminal_caller(KernelCommandSource::LocalCli, "kernel-connection-b")
                    .await,
            );
            let retry = context(
                router
                    .local_terminal_caller(KernelCommandSource::LocalCli, "kernel-connection-a")
                    .await,
            );
            let (user_a, actor_a) = state.kernel_browser_terminal_context(&a).unwrap();
            let (user_b, actor_b) = state.kernel_browser_terminal_context(&b).unwrap();
            assert_eq!(user_a, user_b, "MD-3: terminals share their user profile");
            assert_ne!(
                actor_a, actor_b,
                "MD-3: one terminal must not overwrite another observation or release takeover"
            );
            assert_eq!(
                state.kernel_browser_terminal_context(&retry).unwrap().1,
                actor_a,
                "MD-3: in-connection retry keeps its actor"
            );
            assert_ne!(
                a.caller, b.caller,
                "MD-3: caller-bound retry receipts must distinguish connections"
            );
            assert_eq!(a.caller, retry.caller);
        });
    }

    #[test]
    fn mcp_input_requires_an_observed_document_without_changing_terminal_protocol() {
        let input = serde_json::json!({"command":{"op":"input","tab_id":"host-tab-a","generation":1,"input":{"kind":"click","x":1,"y":2}}});
        assert!(browser_tool_params(input.clone()).is_err());
        let mut observed = input;
        observed["document_id"] = serde_json::json!("observed-loader");
        let params = browser_tool_params(observed).unwrap();
        assert_eq!(params["document_id"], "observed-loader");
        assert_eq!(params["focused_agent"], true);
        assert!(browser_tool_params(serde_json::json!({"command":{"op":"state"}})).is_ok());
        assert!(serde_json::from_value::<crate::local::KernelBrowserRequest>(serde_json::json!({"command":{"op":"input","tab_id":"host-tab-a","generation":1,"document_id":"caller-cannot-inject-private-fields","input":{"kind":"click","x":1,"y":2}}})).is_err());
    }
}
