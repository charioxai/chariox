//! MD-2/MD-3: routing and focused-agent MCP adapter for the host service.
use super::*;
use crate::local::KernelBrowserCommand;
use crate::runtime::kernel_browser_features as features;
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
        frame_width: u32,
        frame_height: u32,
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
    RegionInput {
        binding: crate::runtime::kernel_browser_host::KernelBrowserDocumentBinding,
        binding_id: String,
        sequence: u64,
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
        target: Option<crate::local::KernelBrowserDisplayTarget>,
        codecs: Vec<String>,
        bitrate: u32,
        device_scale_factor: u32,
        initial_push: bool,
        tab_display: bool,
    },
    EncodedCapture {
        subscription_id: String,
        generation: u64,
        after_sequence: u64,
        /// MP-08/MP-10: set only by the kernel push pump.
        push: Option<KernelBrowserPushCredit>,
    },
    Attach {
        subscription_id: String,
        generation: u64,
    },
    Suspend {
        subscription_id: String,
        generation: u64,
    },
    Park {
        subscription_id: String,
        generation: u64,
        parked: bool,
    },
    Resume {
        subscription_id: String,
        generation: u64,
    },
}

/// MP-08/MP-10: a kernel push-pump credit. `reset` retires the stream's
/// reference chain (lost frame or lifted gate); `congested` reports a closed
/// ACK gate to the motion bitrate budget.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct KernelBrowserPushCredit {
    pub(crate) reset: bool,
    pub(crate) congested: bool,
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
        .map_err(|error| host_error(super::kernel_browser_command_error::public_reason(&error)))?;
    if matches!(
        &request.command,
        KernelBrowserCommand::GrantLeasedComputer { .. }
            | KernelBrowserCommand::RevokeLeasedComputer { .. }
            | KernelBrowserCommand::GrantRoomComputer { .. }
            | KernelBrowserCommand::ListGrants
            | KernelBrowserCommand::SubscribeGrants { .. }
            | KernelBrowserCommand::RevokeGrants { .. }
            | KernelBrowserCommand::MirrorSubscribe { .. }
            | KernelBrowserCommand::MirrorNext { .. }
            | KernelBrowserCommand::MirrorClose { .. }
            | KernelBrowserCommand::MirrorInput { .. }
            | KernelBrowserCommand::DisplayCapture { .. }
            | KernelBrowserCommand::DisplaySubscribe { .. }
            | KernelBrowserCommand::DisplayNext { .. }
            | KernelBrowserCommand::DisplaySuspend { .. }
            | KernelBrowserCommand::DisplayResume { .. }
            | KernelBrowserCommand::DisplayPark { .. }
            | KernelBrowserCommand::DisplayAck { .. }
            | KernelBrowserCommand::DisplayInput { .. }
            | KernelBrowserCommand::DisplayTakeover { .. }
            | KernelBrowserCommand::DisplayRelease { .. }
            | KernelBrowserCommand::DisplayActors
            | KernelBrowserCommand::Computer { .. }
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
        if let KernelBrowserCommand::Unsubscribe {
            subscription_id,
            generation,
        } = &command
        {
            if self.is_room_display(subscription_id) {
                return self
                    .room_display_control(
                        &caller.caller,
                        subscription_id,
                        *generation,
                        crate::transport::room_shared_display::RoomDisplayCommand::Close {},
                    )
                    .await;
            }
        }
        if let KernelBrowserCommand::Computer { command } = command {
            return self.kernel_computer_terminal_request(caller, command).await;
        }
        if let KernelBrowserCommand::StateWait { after, wait_ms } = command {
            return self
                .kernel_browser_state_wait(caller, &after, wait_ms)
                .await;
        }
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
                target,
                codecs,
                bitrate,
                device_scale_factor,
                initial_push,
                tab_display,
            } => KernelBrowserDisplayRequest::NegotiatedSubscribe {
                tab_id,
                generation,
                target,
                codecs,
                bitrate,
                device_scale_factor,
                initial_push,
                tab_display,
            },
            KernelBrowserCommand::DisplayNext {
                subscription_id,
                generation,
                after_sequence,
            } => KernelBrowserDisplayRequest::EncodedCapture {
                subscription_id,
                generation,
                after_sequence,
                push: None,
            },
            KernelBrowserCommand::DisplaySuspend {
                subscription_id,
                generation,
            } => KernelBrowserDisplayRequest::Suspend {
                subscription_id,
                generation,
            },
            KernelBrowserCommand::DisplayResume {
                subscription_id,
                generation,
            } => KernelBrowserDisplayRequest::Resume {
                subscription_id,
                generation,
            },
            KernelBrowserCommand::DisplayPark {
                subscription_id,
                generation,
                parked,
            } => KernelBrowserDisplayRequest::Park {
                subscription_id,
                generation,
                parked,
            },
            // MP-08/MP-10: acknowledgements belong to a relay display
            // subscription's push pump; no other transport owns one.
            KernelBrowserCommand::DisplayAck { .. } => {
                return Err(host_error(
                    "MP-08: display acknowledgements require a relay display subscription".into(),
                ))
            }
            KernelBrowserCommand::DisplayInput {
                tab_id,
                generation,
                document_id,
                binding_id,
                sequence,
                input,
            } => {
                let binding = crate::runtime::kernel_browser_host::KernelBrowserDocumentBinding {
                    tab_id,
                    generation,
                    document_id,
                };
                match (binding_id, sequence) {
                    (None, None) => KernelBrowserDisplayRequest::Input { binding, input },
                    (Some(binding_id), Some(sequence))
                        if !binding_id.is_empty() && binding_id.len() <= 256 && sequence > 0 =>
                    {
                        KernelBrowserDisplayRequest::RegionInput {
                            binding,
                            binding_id,
                            sequence,
                            input,
                        }
                    }
                    _ => {
                        return Err(host_error(
                            "MP-11: region input requires its binding and epoch".into(),
                        ))
                    }
                }
            }
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
                    KernelBrowserCommand::GrantLeasedComputer { agent_id, target } => {
                        return self
                            .grant_leased_computer(&user, agent_id, target.clone())
                            .await;
                    }
                    KernelBrowserCommand::RevokeLeasedComputer { agent_id } => {
                        self.revoke_leased_computer(&user, agent_id).await?;
                        let kernel = self.owned.config_projection.snapshot().daemon_id;
                        return self.room_computer_grant_snapshot(&user, &kernel);
                    }
                    KernelBrowserCommand::GrantRoomComputer { .. }
                    | KernelBrowserCommand::ListGrants
                    | KernelBrowserCommand::RevokeGrants { .. }
                    | KernelBrowserCommand::SubscribeGrants { .. } => {
                        if let KernelBrowserCommand::GrantRoomComputer { agent_id } = &command {
                            self.set_room_computer_access(&user, agent_id.as_deref(), true)?;
                        }
                        if let KernelBrowserCommand::RevokeGrants { agent_id } = &command {
                            self.set_room_computer_access(&user, agent_id.as_deref(), false)?;
                            self.owned
                                .kernel_browser_host
                                .revoke_grants(&user, agent_id.as_deref());
                            self.revoke_user_leased_computer(&user, agent_id.as_deref())
                                .await?;
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
                        return self.room_computer_grant_snapshot(&user, &kernel);
                    }
                    _ => {}
                }
                #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
                if matches!(command, KernelBrowserCommand::Stop) {
                    let views = self.app_control().user_views();
                    for owner in self.user_domain_owner_aliases(&user) {
                        for view in views.browser_views(&owner) {
                            self.suspend_user_app_view(&owner, &view.view_id);
                        }
                    }
                }
                #[allow(unused_variables)]
                let stop = matches!(command, KernelBrowserCommand::Stop);
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
                // MP-08/MP-11: the owner's live browser brings back its App Tabs.
                #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
                if !stop {
                    self.restore_user_app_tabs(&user);
                }
                return Ok(result);
            }
        };
        let room_control = match &request {
            KernelBrowserDisplayRequest::Suspend {
                subscription_id, ..
            }
            | KernelBrowserDisplayRequest::Resume {
                subscription_id, ..
            } => self.is_room_display(subscription_id),
            _ => false,
        };
        let mirror_control = features::mirror_enabled()
            && matches!(
                &request,
                KernelBrowserDisplayRequest::Takeover { .. }
                    | KernelBrowserDisplayRequest::Release { .. }
                    | KernelBrowserDisplayRequest::Actors
            );
        if !(room_control || features::display_enabled() || mirror_control) {
            return Err(host_error("MD-DISPLAY: display disabled".into()));
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
    /// MP-08/MP-10/MP-11:535 registration keeps525 as the sole resume authority.
    pub(crate) async fn kernel_browser_display_resume_attach(
        &self,
        caller: &crate::runtime::command::KernelCommand,
        subscription_id: String,
        generation: u64,
    ) -> Result<serde_json::Value, DaemonError> {
        if self.is_room_display(&subscription_id) {
            return self
                .room_display_control(
                    &caller.caller,
                    &subscription_id,
                    generation,
                    crate::transport::room_shared_display::RoomDisplayCommand::ResumeAttach {},
                )
                .await;
        }
        let binding = self
            .kernel_browser_display_request(
                caller,
                KernelBrowserDisplayRequest::Resume {
                    subscription_id,
                    generation,
                },
            )
            .await?;
        let id = binding
            .get("subscription_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| host_error("MP-11: missing resumed binding".into()))?;
        let generation = binding
            .get("generation")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| host_error("MP-11: missing resumed generation".into()))?;
        self.kernel_browser_display_attach(caller, id.into(), generation)
            .await?;
        Ok(binding)
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
    /// L3: the direct worker frame channel for this user's native displays.
    pub(crate) fn kernel_browser_direct_frames(
        &self,
        caller: &crate::runtime::command::KernelCommand,
    ) -> Option<std::sync::Arc<crate::runtime::browser_controller_process::DirectFrames>> {
        let user = self.provider_account_authority_owner_user_id(
            &crate::runtime::command::command_caller_user_id(caller),
        );
        self.owned.kernel_browser_host.direct_frames(&user)
    }

    pub(crate) async fn kernel_browser_display_request(
        &self,
        caller: &crate::runtime::command::KernelCommand,
        request: KernelBrowserDisplayRequest,
    ) -> Result<serde_json::Value, DaemonError> {
        let room_control = match &request {
            KernelBrowserDisplayRequest::Attach {
                subscription_id,
                generation,
            } => Some((
                subscription_id,
                *generation,
                crate::transport::room_shared_display::RoomDisplayCommand::Attach {},
            )),
            KernelBrowserDisplayRequest::Suspend {
                subscription_id,
                generation,
            } => Some((
                subscription_id,
                *generation,
                crate::transport::room_shared_display::RoomDisplayCommand::Suspend {},
            )),
            KernelBrowserDisplayRequest::Resume {
                subscription_id,
                generation,
            } => Some((
                subscription_id,
                *generation,
                crate::transport::room_shared_display::RoomDisplayCommand::Resume {},
            )),
            _ => None,
        };
        if let Some((id, generation, command)) = room_control {
            if self.is_room_display(id) {
                return self
                    .room_display_control(&caller.caller, id, generation, command)
                    .await;
            }
        }
        use crate::session::{EnvironmentActor, EnvironmentActorKind};
        // MP-08/MP-10/MP-11: ingress-to-admission timing contains no binding.
        let input_at = matches!(
            &request,
            KernelBrowserDisplayRequest::Input { .. }
                | KernelBrowserDisplayRequest::RegionInput { .. }
        )
        .then(std::time::Instant::now);
        let (user, actor) = self.kernel_browser_terminal_context(caller)?;
        let host = &self.owned.kernel_browser_host;
        let admission =
            host.admit_terminal(&user, caller.terminal_lifetime.clone().unwrap_or_default());
        let mut params = match request {
            KernelBrowserDisplayRequest::NegotiatedSubscribe {
                tab_id,
                generation,
                target,
                codecs,
                bitrate,
                device_scale_factor,
                initial_push,
                tab_display,
            } => {
                let mut params = serde_json::json!({"op":"display_subscribe","tab_id":tab_id,"generation":generation,"codecs":codecs,"bitrate":bitrate,"device_scale_factor":device_scale_factor});
                if tab_display {
                    params["tab_display"] = true.into();
                }
                if let Some(target) = target {
                    params["target"] = serde_json::to_value(target)
                        .map_err(|_| host_error("MP-11: invalid display target".into()))?;
                }
                if initial_push {
                    params["initial_push"] = true.into();
                }
                params
            }
            KernelBrowserDisplayRequest::EncodedCapture {
                subscription_id,
                generation,
                after_sequence,
                push,
            } => {
                let mut params = serde_json::json!({"op":"screenshot","display_subscription_id":subscription_id,"generation":generation,"after_sequence":after_sequence,"bound_frames":true});
                if let Some(push) = push {
                    params["push"] =
                        serde_json::json!({"reset":push.reset,"congested":push.congested});
                }
                params
            }
            KernelBrowserDisplayRequest::Attach {
                subscription_id,
                generation,
            } => {
                serde_json::json!({"op":"display_attach","subscription_id":subscription_id,"generation":generation})
            }
            KernelBrowserDisplayRequest::Suspend {
                subscription_id,
                generation,
            } => {
                serde_json::json!({"op":"display_suspend","subscription_id":subscription_id,"generation":generation})
            }
            KernelBrowserDisplayRequest::Resume {
                subscription_id,
                generation,
            } => {
                serde_json::json!({"op":"display_resume","subscription_id":subscription_id,"generation":generation})
            }
            KernelBrowserDisplayRequest::Park {
                subscription_id,
                generation,
                parked,
            } => {
                serde_json::json!({"op":"display_park","subscription_id":subscription_id,"generation":generation,"parked":parked})
            }
            KernelBrowserDisplayRequest::Capture { tab_id, generation } => {
                serde_json::json!({"op":"screenshot","tab_id":tab_id,"generation":generation,"bound_frames":true})
            }
            KernelBrowserDisplayRequest::CaptureRegion {
                tab_id,
                generation,
                frame_width,
                frame_height,
            } => {
                serde_json::json!({"op":"screenshot","tab_id":tab_id,"generation":generation,"bound_frames":true,"_capture_protection":true,"_capture_viewport":{"width":frame_width,"height":frame_height}})
            }
            KernelBrowserDisplayRequest::Subscribe { tab_id, generation } => {
                serde_json::json!({"op":"subscribe","tab_id":tab_id,"generation":generation,"bound_frames":true})
            }
            KernelBrowserDisplayRequest::Input { binding, input } => {
                if binding.document_id.is_empty() || binding.document_id.len() > 256 {
                    return Err(host_error("MD-3: observed document required".into()));
                }
                // MP-08/MP-10/MP-11: only this admitted human display path may
                // use asynchronous wheel dispatch; agent calls keep their ack.
                serde_json::json!({"op":"input","_display_input":true,"tab_id":binding.tab_id,"generation":binding.generation,"document_id":binding.document_id,"input":input})
            }
            KernelBrowserDisplayRequest::RegionInput {
                binding,
                binding_id,
                sequence,
                input,
            } => {
                if binding.document_id.is_empty() || binding.document_id.len() > 256 {
                    return Err(host_error(
                        "MP-11: observed region document required".into(),
                    ));
                }
                serde_json::json!({"op":"input","_display_input":true,"tab_id":binding.tab_id,"generation":binding.generation,"document_id":binding.document_id,"binding_id":binding_id,"sequence":sequence,"input":input})
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
        if let Some(at) = input_at {
            crate::transport::kernel_browser_display::timing("input_admission", at);
        }
        let mut result = self
            .kernel_browser_operation_admitted(&user, Some(admission), "host.browser", params)
            .await?;
        // PERF_AUDIT 4.2.2/4.2.6: the window a person's Chrome UI input left is read after the
        // serial input lane is free (a read inside it queued every later key) and joins this reply.
        // The relay's input turn for this lane is given back too: a typed URL otherwise waited one
        // omnibox read per character, and its Enter reached Chrome 0.3-0.5 s late.
        let follow = input_at
            .and(result.as_object_mut())
            .and_then(|reply| reply.remove("_follow"));
        if let Some(follow) = follow {
            crate::transport::relay_client::release_input_turn();
            let admission =
                host.admit_terminal(&user, caller.terminal_lifetime.clone().unwrap_or_default());
            let params = serde_json::json!({"op":"chrome_window_follow","tab_id":follow["tab_id"],"switches":follow["switches"],"observed_by":actor});
            if let Ok(followed) = self
                .kernel_browser_operation_admitted(&user, Some(admission), "host.browser", params)
                .await
            {
                if let Some(window) = followed.get("chrome_window") {
                    result["chrome_window"] = window.clone();
                }
            }
        }
        Ok(result)
    }
    /// MD-2/MD-5: appviews retains admission; observations use the same protection.
    pub(crate) async fn kernel_browser_app_view(
        &self,
        user: String,
        request: crate::runtime::browser_controller_app_view::BrowserAppViewRequest,
        generation: Option<u64>,
    ) -> Result<serde_json::Value, DaemonError> {
        let mut params = request.params();
        if matches!(
            request,
            crate::runtime::browser_controller_app_view::BrowserAppViewRequest::Calls
        ) {
            // MP-08/MP-11: the controller retains calls for opening targets until
            // their owner binding is published. Room drains remain unfiltered.
            let instances = self.app_control().user_views();
            let targets: Vec<String> = instances
                .browser_views(&user)
                .iter()
                .filter(|view| {
                    view.browser
                        .as_ref()
                        .is_some_and(|b| Some(b.generation) == generation)
                })
                .filter_map(|view| instances.browser_target(&user, &view.view_id))
                .collect();
            params["target_ids"] = serde_json::json!(targets);
        }
        let user = self.provider_account_authority_owner_user_id(&user);
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
        let [run] = runs else {
            return Vec::new();
        };
        let loader = RuntimeToolSpec { name: LOADER.into(),
            description: "MD-3: load the user's kernel browser tools. Use only when the user asked you to use their browser in this turn, or after they focused you. The grant covers tabs you open (not their existing tabs) for lifetime_hours (default 8, max 24) and is retained until expiry, idle lapse or revocation. If effective=after_provider_reload, end this turn; Chariox refreshes the tools and automatically continues the request.".into(),
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
            tools.extend(self.owner_handoff_tool_spec());
            if self.room_agent_tools_enabled() {
                tools.push(RuntimeToolSpec { name: SHARE.into(),
                    description: "MP-08: give your direct child agent an explicit subset of the browser resources you were granted. It cannot open new tabs and is revoked with your grant.".into(),
                    input_schema: serde_json::json!({"type":"object","properties":{"agent":{"type":"string","minLength":1},"resources":{"type":"array","minItems":1,"maxItems":64,"items":{"type":"object","properties":{"kind":{"type":"string","enum":["browser_tab","app_view"]},"tab_id":{"type":"string"},"view_id":{"type":"string"}},"required":["kind"]}}},"required":["agent","resources"],"additionalProperties":false}), });
            }
            tools.push(RuntimeToolSpec { name: BROWSER.into(),
                description: "MD-3: control the user's kernel browser outside sessions/slices. Read state/snapshot/screenshot to obtain tab IDs, generation and document_id; input requires the observed document_id beside command and accepts click/text/key/scroll; click/scroll x,y are CSS pixels inside the state viewport (screenshot pixels / device_scale_factor). Pending dialogs appear in state.dialogs; answer with dialog using its dialog_id, tab_id, document_id, generation and action accept/dismiss (optional prompt_text, empty allowed). Vault input is separate.".into(),
                input_schema: serde_json::json!({"type":"object","properties":{"document_id":{"type":"string","minLength":1,"maxLength":256},"command":{"type":"object","properties":{"op":{"type":"string","enum":["start","state","stop","open","close","navigate","snapshot","input","dialog","screenshot","subscribe","poll","unsubscribe"]},"tab_id":{"type":"string"},"generation":{"type":"integer","minimum":1},"url":{"type":"string"},"subscription_id":{"type":"string"},"document_id":{"type":"string","minLength":1,"maxLength":256},"dialog_id":{"type":"string","minLength":1,"maxLength":256},"action":{"type":"string","enum":["accept","dismiss"]},"prompt_text":{"type":"string","maxLength":2048},"input":{"type":"object","properties":{"kind":{"type":"string","enum":["click","text","key","scroll"]},"x":{"type":"integer","minimum":0,"maximum":1279},"y":{"type":"integer","minimum":0,"maximum":799},"text":{"type":"string","maxLength":16384},"key":{"type":"string","minLength":1,"maxLength":16,"description":"Named navigation/editing key or a single printable character; validated by the shared browser input path."},"modifiers":{"type":"integer","minimum":0,"maximum":15,"description":"Optional bounded CDP keyboard bitset: Alt=1, Ctrl=2, Meta=4, Shift=8; omitted means zero."},"delta_x":{"type":"integer","minimum":-10000,"maximum":10000},"delta_y":{"type":"integer","minimum":-10000,"maximum":10000}},"required":["kind"],"additionalProperties":false}},"required":["op"],"additionalProperties":false}},"required":["command"],"additionalProperties":false}), });
        }
        tools
    }
    pub(super) async fn try_kernel_browser_tool(
        &self,
        token: &str,
        name: &str,
        arguments: serde_json::Value,
    ) -> Option<Result<RuntimeToolResult, DaemonError>> {
        if let Some(result) = self
            .try_kernel_computer_tool(token, name, arguments.clone())
            .await
        {
            return Some(result);
        }
        if ![
            LOADER,
            BROWSER,
            PASTE,
            SHARE,
            super::owner_handoff_runtime::HANDOFF,
        ]
        .contains(&name)
        {
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
        let authority = self.clone();
        let auth_token = token.to_string();
        let run_id = run.id().to_string();
        let admission = host
            .admit(&user, agent.id())
            .map_err(host_error)?
            .with_agent_label(agent.alias())
            .with_authority(move || {
                let runs = authority
                    .owned
                    .provider_store
                    .get_runs_by_runtime_mcp_auth_token(&auth_token);
                let [current] = runs.as_slice() else {
                    return false;
                };
                current.id() == run_id && authority.kernel_browser_agent(current).is_some()
            });
        if name == SHARE {
            return self.kernel_browser_share(&agent, arguments, &admission);
        }
        if name == super::owner_handoff_runtime::HANDOFF {
            return self
                .kernel_browser_handoff(&agent, arguments, admission)
                .await;
        }
        if name == PASTE {
            let scope = crate::runtime::kernel_browser_host::KernelBrowserHost::profile_key(
                &self.provider_account_authority_owner_user_id(agent.owner_user_id()),
            );
            return self
                .kernel_browser_paste_secret(run.session_id(), &agent, arguments, admission)
                .await
                .map_err(|error| {
                    self.owned
                        .kernel_browser_secret_observations
                        .scrub_error(&scope, error)
                });
        }
        let mut params = browser_tool_params(arguments)?;
        self.claim_app_view_tab(&user, &admission, &mut params)?;
        let payload = self
            .kernel_browser_operation_admitted(&user, Some(admission), "host.browser", params)
            .await?;
        Ok(RuntimeToolResult { ok: true, payload })
    }
}
impl KernelRuntimeState {
    /// MP-08/MP-11: an agent drives an owner's App Tab (input or Vault fill)
    /// only after claiming that App view in its own grant; the host then runs
    /// the Tab's bridge calls as the agent.
    #[cfg_attr(
        not(any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))),
        allow(unused_variables)
    )]
    pub(super) fn claim_app_view_tab(
        &self,
        user: &str,
        admission: &crate::runtime::kernel_browser_host::KernelBrowserAdmission,
        params: &mut serde_json::Value,
    ) -> Result<(), DaemonError> {
        #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
        if let Some(tab) = params["tab_id"].as_str().map(str::to_owned) {
            let views = self.app_control().user_views();
            for view in self
                .user_domain_owner_aliases(user)
                .iter()
                .flat_map(|owner| views.browser_views(owner))
            {
                if view
                    .browser
                    .as_ref()
                    .is_some_and(|browser| browser.tab_id == tab)
                {
                    self.owned
                        .kernel_browser_host
                        .claim_resource(
                            Some(admission),
                            crate::local::UserDomainResource::AppView {
                                view_id: view.view_id,
                            },
                            false,
                        )
                        .map_err(host_error)?;
                    params["_app_view_tab"] = tab.clone().into();
                }
            }
        }
        Ok(())
    }
    /// MP-08/MP-11: focus or the owner's running request loads browser tools;
    /// otherwise only an already-loaded grant is retained. Agents cannot self-grant.
    async fn kernel_browser_loader(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let arguments: LoaderArguments = serde_json::from_value(arguments)
            .map_err(|_| host_error("MD-3: loader accepts only lifetime_hours".into()))?;
        self.refresh_user_domain_grants();
        // MP-08/MP-11: placement refuses leased and remote runs with a typed
        // NotGranted before the Room epoch check can mask it.
        let agent = self.user_domain_agent_placement(run)?;
        self.authorize_current_external_command()?;
        let host = &self.owned.kernel_browser_host;
        let user = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        let previous_catalog = self.runtime_catalog_signature_for_agent(&agent);
        let already_loaded = host.is_loaded(&user, agent.id());
        if !host.is_focused(&user, agent.id()) && !already_loaded {
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
            return Ok(self.runtime_catalog_loader_result(
                &agent,
                &previous_catalog,
                already_loaded,
                serde_json::json!({"loaded": BROWSER, "granted": granted, "prompt_id": prompt}),
            ));
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
        Ok(self.runtime_catalog_loader_result(
            &agent,
            &previous_catalog,
            already_loaded,
            serde_json::json!({"loaded": BROWSER}),
        ))
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
    fn mirror_and_display_requests_are_admitted_without_environment() {
        // MP-08/MP-11: a plain kernel serves the mirror and display; "0" is the kill switch.
        crate::test_support::isolated_env_test!();
        use crate::runtime::command::{KernelCommand, KernelCommandSource};
        use crate::runtime::router::CommandRouter;
        std::env::remove_var("CHARIOX_KERNEL_BROWSER_MIRROR");
        std::env::remove_var("CHARIOX_KERNEL_BROWSER_DISPLAY");
        std::env::set_var(
            "CHARIOX_KERNEL_BROWSER_SCRIPT",
            "/nonexistent/kernel-browser-host.mjs",
        );
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
            let caller = KernelCommand::from_local_request_with_caller(
                "gate-check",
                KernelCommandSource::LocalCli,
                router
                    .local_terminal_caller(KernelCommandSource::LocalCli, "gate-connection")
                    .await,
                None,
                None,
                &request,
            );
            let refusal = |command| {
                let (state, caller) = (&state, &caller);
                async move {
                    state
                        .kernel_browser_terminal_request(caller, command)
                        .await
                        .err()
                        .map(|error| error.to_string())
                        .unwrap_or_default()
                }
            };
            let mirror = || KernelBrowserCommand::MirrorClose {
                subscription_id: "gate".into(),
                generation: 1,
            };
            let display = || KernelBrowserCommand::DisplayCapture {
                tab_id: "gate".into(),
                generation: 1,
            };
            for command in [mirror(), display()] {
                let error = refusal(command).await;
                assert!(!error.contains("disabled"), "default must admit: {error}");
            }
            std::env::set_var("CHARIOX_KERNEL_BROWSER_MIRROR", "0");
            std::env::set_var("CHARIOX_KERNEL_BROWSER_DISPLAY", "0");
            assert!(refusal(mirror()).await.contains("mirroring disabled"));
            assert!(refusal(display()).await.contains("display disabled"));
        });
    }

    #[test]
    fn mcp_browser_command_errors_explain_safe_fields_without_reflecting_values() {
        // MP-08/MP-10/MP-11: the provider schema lists shared parameters, but
        // each strict command has its own required and accepted fields.
        for (arguments, reason) in [
            (
                serde_json::json!({"command":{"op":"open","url":"https://example.com","generation":1}}),
                "unexpected field generation",
            ),
            (
                serde_json::json!({"command":{"op":"navigate","url":"https://example.com","tab_id":"tab"}}),
                "missing required field generation",
            ),
            (
                serde_json::json!({"command":{"op":"PRIVATE_VALUE"}}),
                "unsupported operation or input kind",
            ),
            (
                serde_json::json!({"command":{"op":"open","url":2}}),
                "invalid parameter type",
            ),
            (
                serde_json::json!({"command":{"op":"open","url":"https://example.com","PRIVATE_FIELD":"PRIVATE_VALUE"}}),
                "unexpected parameter",
            ),
        ] {
            let error = browser_tool_params(arguments).unwrap_err().to_string();
            assert!(
                error.contains(reason),
                "MP-10: missing safe decode reason: {error}"
            );
            assert!(
                !error.contains("PRIVATE"),
                "MP-11: never reflect arbitrary parameters or values"
            );
        }
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
    #[test]
    fn mp11_provider_tools_cannot_suspend_or_resume_human_display_leases() {
        for op in ["display_suspend", "display_resume", "display_park"] {
            assert!(browser_tool_params(serde_json::json!({"command":{
                "op":op,"subscription_id":"human-display","generation":7
            }}))
            .is_err());
        }
    }
}
