use crate::runtime::browser_controller_process::{
    BrowserControllerProcessStore, CONTROLLER_RESTARTED_BEFORE_OPERATION,
};
use crate::transport::room_browser_controller::{
    BrowserLifecycleOperation, RoomBrowserControllerCommand as Command,
    RoomBrowserControllerResult as Response, SecretObservationDisposition as Disposition,
};

use super::*;

impl KernelRuntimeState {
    /// Send a Room request to its slice worker over the connected relay when one
    /// matches the slice's relay. A temporary connection needs relay metadata
    /// access, which a kernel-scoped Cloud relay token does not carry; the
    /// connected path avoids it while the worker key is pinned or cached (managed
    /// slice workers pin theirs at token refresh).
    pub(super) async fn send_room_slice_peer_request(
        &self,
        config: &crate::config::DaemonConfig,
        target: chariox_relay::protocol::ClientTarget,
        request: RelayPeerRequest,
        timeout: std::time::Duration,
    ) -> Result<RelayPeerResponse, DaemonError> {
        // All Room display/observation routes share worker revocation admission.
        let slice_id = match &request {
            RelayPeerRequest::RoomBrowserController {
                slice_id, command, ..
            } if !matches!(command, Command::ClearSecretObservation { .. }) => Some(slice_id),
            RelayPeerRequest::ObserveRoomComputer { slice_id, .. }
            | RelayPeerRequest::CaptureRoomScreenshot { slice_id, .. }
            | RelayPeerRequest::ReadRoomScreenshotChunk { slice_id, .. }
            | RelayPeerRequest::OpenRoomDisplay { slice_id, .. } => Some(slice_id),
            _ => None,
        };
        if let Some(slice_id) = slice_id {
            self.settle_slice_observation_revocations(slice_id).await?;
        }
        self.send_room_slice_peer_request_unchecked(config, target, request, timeout)
            .await
    }

    pub(super) async fn send_room_slice_peer_request_unchecked(
        &self,
        config: &crate::config::DaemonConfig,
        target: chariox_relay::protocol::ClientTarget,
        request: RelayPeerRequest,
        timeout: std::time::Duration,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let result = match self.connected_relay_state_for_config(config).await {
            Some(relay_state) => {
                crate::transport::relay_client::send_peer_request_via_connected_relay_authorized(
                    config,
                    &relay_state,
                    target,
                    request,
                    timeout,
                    || self.authorize_current_external_command(),
                )
                .await
            }
            None => {
                crate::transport::relay_client::send_peer_request_via_temporary_connection_authorized(
                    config, target, request, timeout, || self.authorize_current_external_command(),
                )
                .await
            }
        };
        result
    }

    pub(crate) fn browser_controller_enabled_for_room(&self, session_id: &str) -> bool {
        self.owned
            .slice_store
            .environment_slice(session_id)
            .is_some()
            || self.browser_controller_process_enabled()
            || self
                .owned
                .config_projection
                .snapshot()
                .room_environment_worker_binding
                .is_some()
    }

    pub(super) async fn room_browser_controller_command(
        &self,
        session_id: &str,
        command: Command,
    ) -> Result<Response, DaemonError> {
        self.room_browser_controller_command_inner(session_id, command, false, false, None)
            .await
    }

    pub(super) async fn room_browser_controller_command_with_admission_deadline(
        &self,
        session_id: &str,
        command: Command,
        deadline: tokio::time::Instant,
    ) -> Result<Response, DaemonError> {
        self.room_browser_controller_command_inner(
            session_id,
            command,
            false,
            false,
            Some(deadline),
        )
        .await
    }

    pub(super) async fn room_browser_controller_recovery_command(
        &self,
        session_id: &str,
        command: Command,
    ) -> Result<Response, DaemonError> {
        self.room_browser_controller_command_inner(session_id, command, true, false, None)
            .await
    }

    pub(super) async fn room_browser_controller_health_probe(
        &self,
        session_id: &str,
        viewport: crate::session::CanonicalViewport,
        browser_bar_visible: bool,
    ) -> Result<Response, DaemonError> {
        self.room_browser_controller_command_inner(
            session_id,
            Command::Reconcile {
                viewport,
                browser_bar_visible,
            },
            false,
            true,
            None,
        )
        .await
    }

    async fn room_browser_controller_command_inner(
        &self,
        session_id: &str,
        command: Command,
        recovery_authority: bool,
        background_probe: bool,
        admission_deadline: Option<tokio::time::Instant>,
    ) -> Result<Response, DaemonError> {
        self.authorize_current_external_command()?;
        if let Command::ClearSecretObservation { disposition } = &command {
            let protection = &self.owned.room_secret_observations;
            let _guard = protection.barrier(session_id)?.write_owned().await;
            let slice = self.owned.slice_store.environment_slice(session_id);
            if let Some(slice) = &slice {
                protection.defer_revocation(session_id, slice, *disposition)?;
            }
            protection.apply_disposition(session_id, *disposition)?;
            if let Some(slice) =
                slice.filter(|slice| slice.status == crate::slice::SliceStatus::Running)
            {
                // An unavailable worker cannot veto the home mutation. Its durable
                // receipt remains mandatory at the next admission, including after deletion.
                // MP-08/MP-10/MP-11: use the same boxed transport boundary as
                // ordinary Room commands, keeping relay state off every caller's stack.
                let _ = Box::pin(self.route_room_browser_controller_command(
                    session_id, slice, command, false, None,
                ))
                .await;
            }
            return Ok(Response::SecretObservationCleared);
        }
        // Cleanup must remain available while the Room is quarantined, including
        // when the durable store cannot establish that execution is safe.
        if !recovery_authority
            && !matches!(
                &command,
                Command::CancelAction { .. }
                    | Command::CancelCookieImport { .. }
                    | Command::ImportCookies { .. }
                    | Command::RecoverCookieImport { .. }
                    | Command::Release
            )
        {
            self.ensure_browser_import_execution_allowed(session_id)
                .map_err(browser_import_execution_gate::execution_error)?;
        }
        let admitted_mutation_command = matches!(
            &command,
            Command::Action { .. }
                | Command::Upload { .. }
                | Command::ConfigureDownloads { .. }
                | Command::Permission { .. }
                | Command::Tab { .. }
                | Command::History { .. }
                | Command::Dialog { .. }
                | Command::Navigate { .. }
                | Command::ComputerInput { .. }
                | Command::CancelDownload { .. }
                | Command::ImportCookies { .. } | Command::AppView {
                request: crate::runtime::browser_controller_app_view::BrowserAppViewRequest::Open { .. }
                    | crate::runtime::browser_controller_app_view::BrowserAppViewRequest::Reload { .. }
            }
        );
        let protection = &self.owned.room_secret_observations;
        let secret_guard = if super::room_secret_observation::command_secret(&command).is_some() {
            Some(protection.barrier(session_id)?.write_owned().await)
        } else {
            None
        };
        let observation_guard = if secret_guard.is_none() && !is_cancellation_command(&command) {
            Some(protection.barrier(session_id)?.read_owned().await)
        } else {
            None
        };
        protection.register_command(session_id, &command)?;
        let response = if let Some(slice) = self.owned.slice_store.environment_slice(session_id) {
            // Keep the relay client's large future off callers' async stacks. Local
            // controller operations stay allocation-free; only the remote boundary
            // owns this boxed transport future.
            Box::pin(self.route_room_browser_controller_command(
                session_id,
                slice,
                command,
                background_probe,
                admission_deadline,
            ))
            .await?
        } else {
            if self
                .owned
                .config_projection
                .snapshot()
                .room_environment_worker_binding
                .is_some()
            {
                return Err(controller_route_error(
                    "browser_controller_scope_denied: provisioned slice controller requires the home Room relay path",
                ));
            }
            // Cancellation signals the active execution without taking its supervisor
            // lock or waiting behind a secret insertion's observation barrier.
            execute_local(
                self.clone(),
                self.owned.browser_controller_processes.clone(),
                self.owned.computer_input_executions.clone(),
                session_id,
                command,
            )
            .await?
        };
        let response = protection.scrub_response(session_id, response)?;
        drop(observation_guard);
        drop(secret_guard);
        // A completed controller effect must keep its result after grant revocation.
        match response {
            Response::ActionCancelled { controller_fenced } if admitted_mutation_command => {
                Err(DaemonError::BrowserControllerActionCancelled { controller_fenced })
            }
            Response::RecoveryRequired { process } if admitted_mutation_command => {
                Err(DaemonError::BrowserControllerRecoveryRequired {
                    runtime_generation: process.runtime_generation,
                })
            }
            Response::RecoveryRequired { process } => {
                Box::pin(self.recover_browser_controller_after_restart(
                    session_id,
                    process.runtime_generation,
                ))
                .await?;
                Err(controller_route_error(
                    CONTROLLER_RESTARTED_BEFORE_OPERATION,
                ))
            }
            response => Ok(response),
        }
    }

    pub(super) async fn route_room_browser_controller_command(
        &self,
        session_id: &str,
        slice: crate::slice::SliceRecord,
        command: Command,
        background_probe: bool,
        admission_deadline: Option<tokio::time::Instant>,
    ) -> Result<Response, DaemonError> {
        let (slice, _guard) = self
            .admit_room_browser_controller_route(
                session_id,
                &slice.id,
                &command,
                background_probe,
                admission_deadline,
            )
            .await?;
        if matches!(command, Command::ClearSecretObservation { .. }) {
            self.settle_slice_observation_revocations(&slice.id).await?;
            return Ok(Response::SecretObservationCleared);
        }
        let config = self.owned.config_projection.snapshot();
        let slice_relay = config.slice_relay_override(&slice);
        let private_slice_relay = slice_relay.is_some()
            && slice
                .relay_endpoint
                .as_ref()
                .is_none_or(|endpoint| endpoint.private);
        let config = slice_relay.unwrap_or(config);
        let target = ClientTarget {
            daemon_id: slice.worker_kernel_id.clone(),
            daemon_alias: slice
                .worker_kernel_id
                .is_none()
                .then(|| slice.worker_kernel_ref.clone()),
        };
        if matches!(command, Command::NoteObservation { .. }) {
            // MD-N2: Ping is understood by old workers; only send the new variant
            // after a current, authenticated response from this exact target.
            let value = format!("md-notes-protocol:{:016x}", rand::random::<u64>());
            let response = Box::pin(self.send_room_slice_peer_request(
                &config,
                target.clone(),
                RelayPeerRequest::Ping {
                    value: value.clone(),
                },
                Duration::from_secs(15),
            ))
            .await?;
            let version = match response {
                RelayPeerResponse::Pong {
                    value: received,
                    relay_peer_protocol_version,
                    ..
                } if received == value => relay_peer_protocol_version,
                _ => None,
            };
            super::room_browser_controller_admission::require_notes_worker_protocol(version)?;
        }
        let recovery = receipt_recovery_command(&command);
        let request = |command| RelayPeerRequest::RoomBrowserController {
            session_id: session_id.to_string(),
            slice_id: slice.id.clone(),
            command,
        };
        let send = |target, command| async {
            let timeout = match &command {
                // Outlast the worker's verified display/layout application and
                // rollback; a home timeout must not race that physical work.
                Command::Reconcile { .. } => Duration::from_secs(60),
                Command::ComputerInput {
                    action: crate::transport::room_browser_controller::RoomComputerInputAction::KeyboardText { input },
                    ..
                } => Duration::from_millis(
                    crate::runtime::computer_input_action::keyboard_text_timeout_ms(input.as_str()) + 10_000,
                ),
                Command::ComputerInput {
                    action: crate::transport::room_browser_controller::RoomComputerInputAction::SecretText { input, .. }, ..
                } => Duration::from_millis(
                    crate::runtime::computer_input_action::keyboard_text_timeout_ms(input.as_str()) + 10_000,
                ),
                Command::ComputerInput { action:
                    crate::transport::room_browser_controller::RoomComputerInputAction::KeyboardHold { duration_ms, .. }
                    | crate::transport::room_browser_controller::RoomComputerInputAction::PointerHold { duration_ms, .. }, ..
                } => Duration::from_millis(u64::from(*duration_ms) + 15_000),
                _ => Duration::from_secs(15),
            };
            self.send_room_slice_peer_request(&config, target, request(command), timeout)
                .await
        };
        let first = send(target.clone(), command.clone())
            .await
            .map_err(|error| room_slice_unreachable(&slice.name, private_slice_relay, error));
        let response = match first {
            Ok(response) => response,
            Err(first_error) if recovery.is_some() => {
                self.authorize_current_external_command()?;
                send(target, recovery.expect("action recovery command"))
                .await.map_err(|retry_error| controller_route_error(&format!(
                    "browser action result remained unavailable after non-mutating receipt recovery: {retry_error}; initial delivery error: {first_error}"
                )))?
            }
            Err(error) => return Err(error),
        };
        match response {
            RelayPeerResponse::RoomBrowserController {
                session_id: returned_room,
                slice_id,
                result,
            } if returned_room == session_id && slice_id == slice.id => Ok(result),
            _ => Err(controller_route_error(
                "worker returned a mismatched controller response",
            )),
        }
    }

    pub(crate) async fn execute_bound_room_browser_controller(
        &self,
        authenticated_kernel_id: &str,
        authenticated_public_key: &str,
        session_id: &str,
        slice_id: &str,
        command: Command,
    ) -> Result<Response, DaemonError> {
        let config = self.owned.config_projection.snapshot();
        let permitted = config
            .room_environment_worker_binding
            .as_ref()
            .is_some_and(|binding| {
                binding.permits(
                    authenticated_kernel_id,
                    authenticated_public_key,
                    session_id,
                    slice_id,
                )
            });
        if !permitted {
            return Err(controller_route_error(
                "browser_controller_scope_denied: peer or Room does not match the provisioned slice binding",
            ));
        }
        let protection = &self.owned.room_secret_observations;
        if let Command::ClearSecretObservation { disposition } = &command {
            let _guard = protection.barrier(session_id)?.write_owned().await;
            protection.apply_disposition(session_id, *disposition)?;
            if *disposition != Disposition::Retire {
                self.owned
                    .browser_controller_processes
                    .release(session_id)
                    .map_err(|error| controller_route_error(&error))?;
            }
            return Ok(Response::SecretObservationCleared);
        }
        let secret_guard = if super::room_secret_observation::command_secret(&command).is_some() {
            Some(protection.barrier(session_id)?.write_owned().await)
        } else {
            None
        };
        let _observation_guard = if secret_guard.is_none() && !is_cancellation_command(&command) {
            Some(protection.barrier(session_id)?.read_owned().await)
        } else {
            None
        };
        if matches!(&command, Command::Artifact { .. }) {
            protection.require(session_id, true)?;
        }
        protection.register_command(session_id, &command)?;
        if !matches!(
            &command,
            Command::ComputerInput { .. }
                | Command::ComputerClipboardRead { .. }
                | Command::ComputerSecretTarget
        ) && !self.browser_controller_process_enabled()
        {
            return Err(controller_route_error(
                "browser_controller_unavailable: slice has no configured controller",
            ));
        }
        let response = execute_local(
            self.clone(),
            self.owned.browser_controller_processes.clone(),
            self.owned.computer_input_executions.clone(),
            session_id,
            command,
        )
        .await
        .map_err(|error| protection.scrub_error(session_id, error))?;
        protection.scrub_response(session_id, response)
    }
}

fn is_cancellation_command(command: &Command) -> bool {
    matches!(
        command,
        Command::CancelAction { .. } | Command::CancelCookieImport { .. }
    )
}

fn receipt_recovery_command(command: &Command) -> Option<Command> {
    match command {
        Command::Tab {
            execution_id,
            target_id,
            document_id,
            action,
        } => Some(lifecycle_recovery(
            execution_id,
            target_id,
            document_id,
            BrowserLifecycleOperation::Tab { action: *action },
        )),
        Command::History {
            execution_id,
            target_id,
            document_id,
            action,
        } => Some(lifecycle_recovery(
            execution_id,
            target_id,
            document_id,
            BrowserLifecycleOperation::History { action: *action },
        )),
        Command::Navigate {
            execution_id,
            target_id,
            document_id,
            url,
        } => Some(lifecycle_recovery(
            execution_id,
            target_id,
            document_id,
            BrowserLifecycleOperation::Navigate { url: url.clone() },
        )),
        Command::Dialog {
            execution_id,
            target_id,
            document_id,
            action,
        } => Some(lifecycle_recovery(
            execution_id,
            target_id,
            document_id,
            BrowserLifecycleOperation::Dialog {
                action: action.clone(),
            },
        )),
        Command::ConfigureDownloads {
            execution_id,
            target_id,
            document_id,
        } => Some(Command::RecoverDownloadConfiguration {
            execution_id: execution_id.clone(),
            target_id: target_id.clone(),
            document_id: document_id.clone(),
        }),
        Command::Permission {
            execution_id,
            target_id,
            document_id,
            permission,
            setting,
        } => Some(Command::RecoverPermission {
            execution_id: execution_id.clone(),
            target_id: target_id.clone(),
            document_id: document_id.clone(),
            permission: *permission,
            setting: *setting,
        }),
        Command::Upload {
            execution_id,
            target_id,
            document_id,
            node_ref,
            files,
        } => Some(Command::RecoverUpload {
            execution_id: execution_id.clone(),
            target_id: target_id.clone(),
            document_id: document_id.clone(),
            node_ref: node_ref.clone(),
            files: files.clone(),
        }),
        Command::Action {
            execution_id,
            target_id,
            document_id,
            node_ref,
            action,
            timeout_ms,
        } => Some(Command::RecoverAction {
            execution_id: execution_id.clone(),
            target_id: target_id.clone(),
            document_id: document_id.clone(),
            node_ref: node_ref.clone(),
            action: action.clone(),
            timeout_ms: *timeout_ms,
        }),
        _ => None,
    }
}

fn lifecycle_recovery(
    execution_id: &str,
    target_id: &str,
    document_id: &str,
    operation: BrowserLifecycleOperation,
) -> Command {
    Command::RecoverLifecycle {
        execution_id: execution_id.to_string(),
        target_id: target_id.to_string(),
        document_id: document_id.to_string(),
        operation,
    }
}

async fn execute_local(
    state: KernelRuntimeState,
    processes: BrowserControllerProcessStore,
    computer_input_executions: crate::runtime::computer_input_execution::ComputerInputExecutionStore,
    session_id: &str,
    command: Command,
) -> Result<Response, DaemonError> {
    state.authorize_current_external_command()?;
    let controller_authorizer = state.clone();
    let processes = processes.with_authorizer(Arc::new(move || {
        controller_authorizer
            .authorize_current_external_command()
            .map_err(|error| error.to_string())
    }));
    // Updating masks takes the supervisor lock, just like controller I/O. Keep
    // that wait off the async executor and recheck authority after it completes.
    if !is_cancellation_command(&command) {
        let protected_processes = processes.clone();
        let protected_room = session_id.to_string();
        let values = state
            .owned
            .room_secret_observations
            .controller_values(session_id)
            .unwrap_or_default();
        tokio::task::spawn_blocking(move || {
            protected_processes.protect_observation_values(&protected_room, values)
        })
        .await
        .map_err(|error| controller_route_error(&error.to_string()))?
        .map_err(|error| controller_route_error(&error))?;
    }
    let input_authorizer = state.clone();
    let authorize_input: Arc<dyn Fn() -> Result<(), DaemonError> + Send + Sync> =
        Arc::new(move || input_authorizer.authorize_current_external_command());
    let command = match command {
        Command::ComputerInput {
            action_id,
            actor_id,
            runtime_generation,
            viewport_revision,
            desktop_pixel_width,
            desktop_pixel_height,
            action,
        } => {
            if action_id.trim().is_empty()
                || actor_id.trim().is_empty()
                || runtime_generation == 0
                || viewport_revision == 0
            {
                return Err(controller_route_error(
                    "environment_input_invalid_authority_context",
                ));
            }
            // MP-11 R1: refuse unfenced agent mutations before a physical
            // helper is spawned; human and approved Vault actors retain their paths.
            if actor_id.starts_with("agent:") {
                if crate::runtime::computer_input_action::agent_native_input_is_unfenced(&action) {
                    return Err(crate::error::HostFailure::Refused(
                        crate::error::UserDomainRefusalReason::SensitiveRequiresFocus,
                    )
                    .into_daemon("room_computer"));
                }
                // MP-11 review R2: registered/unknown protection fences both
                // text and shortcuts. Never pass registry values to a keyboard.
                // #904 review @61b0a4ac6: clicks and AT-SPI actions can activate
                // Paste, and a registered value in an ordinary native field has
                // no password role for clipboard-owner admission to see.
                if matches!(
                    &action,
                    crate::transport::room_browser_controller::RoomComputerInputAction::KeyboardKey { .. }
                    | crate::transport::room_browser_controller::RoomComputerInputAction::KeyboardText { .. }
                    | crate::transport::room_browser_controller::RoomComputerInputAction::PointerClick { .. }
                    | crate::transport::room_browser_controller::RoomComputerInputAction::TargetAction { .. }
                ) {
                    let policy = state.owned.room_secret_observations.capture_policy(session_id)?;
                    let policy: serde_json::Value = serde_json::from_str(&policy)
                        .map_err(|_| controller_route_error("protection unavailable"))?;
                    if policy["unknown"] == true
                        || policy["values"].as_array().is_none_or(|values| !values.is_empty())
                        || policy["targets"].as_array().is_none_or(|targets| !targets.is_empty())
                    {
                        return Err(crate::error::HostFailure::Refused(
                            crate::error::UserDomainRefusalReason::SensitiveRequiresFocus,
                        ).into_daemon("room_computer"));
                    }
                }
            }
            let execution = computer_input_executions
                .begin(session_id, &action_id)
                .map_err(controller_route_error)?;
            if matches!(
                &action,
                crate::transport::room_browser_controller::RoomComputerInputAction::SecretText { .. }
            ) {
                execution.withhold_capture().await;
            }
            let cancellation = execution
                .cancellation()
                .with_authorizer(authorize_input.clone())
                .with_agent_input(actor_id.starts_with("agent:"));
            let input_result = match action {
                crate::transport::room_browser_controller::RoomComputerInputAction::TargetAction { tree_revision, target_id, action } => {
                    let policy = state.owned.room_secret_observations.capture_policy(session_id)?;
                    let policy: serde_json::Value = serde_json::from_str(&policy).map_err(|_| controller_route_error("protection unavailable"))?;
                    let controller = processes.clone();
                    let room = session_id.to_string();
                    let authorize = authorize_input.clone();
                    tokio::task::spawn_blocking(move || {
                        authorize().map_err(|e| crate::error::HostFailure::Other(e.to_string()))?;
                        controller.room_computer_action(&room, serde_json::json!({"observer":actor_id,"tree_revision":tree_revision,"target_id":target_id,"action":action,"policy":policy}), move || cancellation.requested() || cancellation.authorize().is_err())
                    }).await.map_err(|_| controller_route_error("accessibility action task failed"))?
                        .map(|_| ()).map_err(|error| error.into_daemon("room_computer"))
                }
                crate::transport::room_browser_controller::RoomComputerInputAction::PointerMove {
                    x,
                    y,
                } => super::tool_dispatch::run_room_pointer_move(
                    x,
                    y,
                    desktop_pixel_width,
                    desktop_pixel_height,
                    cancellation,
                )
                .await,
                crate::transport::room_browser_controller::RoomComputerInputAction::PointerDrag {
                    from_x,
                    from_y,
                    to_x,
                    to_y,
                    button,
                } => {
                    super::tool_dispatch::run_room_pointer_drag(
                        from_x,
                        from_y,
                        to_x,
                        to_y,
                        button,
                        desktop_pixel_width,
                        desktop_pixel_height,
                        cancellation,
                    )
                    .await
                }
                crate::transport::room_browser_controller::RoomComputerInputAction::PointerScroll {
                    x,
                    y,
                    horizontal_steps,
                    vertical_steps,
                } => {
                    super::tool_dispatch::run_room_pointer_scroll(
                        x,
                        y,
                        horizontal_steps,
                        vertical_steps,
                        desktop_pixel_width,
                        desktop_pixel_height,
                        cancellation,
                    )
                    .await
                }
                crate::transport::room_browser_controller::RoomComputerInputAction::KeyboardText {
                    input,
                } => super::tool_dispatch::run_room_keyboard_text(input, cancellation).await,
                crate::transport::room_browser_controller::RoomComputerInputAction::KeyboardKey {
                    input,
                    repeat,
                } => {
                    super::tool_dispatch::run_room_keyboard_key(input, repeat, cancellation).await
                }
                action @ (crate::transport::room_browser_controller::RoomComputerInputAction::KeyboardHold { .. }
                    | crate::transport::room_browser_controller::RoomComputerInputAction::PointerHold { .. }) => {
                    super::tool_dispatch::run_room_computer_hold(action, desktop_pixel_width, desktop_pixel_height, cancellation).await
                }
                crate::transport::room_browser_controller::RoomComputerInputAction::ClipboardWrite {
                    text,
                } => super::tool_dispatch::run_room_clipboard_write(text, cancellation).await,
                crate::transport::room_browser_controller::RoomComputerInputAction::PointerClick {
                    x,
                    y,
                    button,
                    click_count,
                } => {
                    super::tool_dispatch::run_room_pointer_click(
                        x,
                        y,
                        button,
                        click_count,
                        desktop_pixel_width,
                        desktop_pixel_height,
                        cancellation,
                    )
                    .await
                }
                crate::transport::room_browser_controller::RoomComputerInputAction::SecretText {
                    input, expected_target,
                } => super::tool_dispatch::run_room_secret_text_input(input, expected_target, cancellation).await,
            };
            if matches!(
                input_result,
                Err(DaemonError::BrowserControllerActionCancelled { .. })
            ) {
                super::tool_dispatch::reset_room_computer_input().await?;
                return Ok(Response::ActionCancelled {
                    controller_fenced: false,
                });
            }
            input_result?;
            return Ok(Response::ComputerInputApplied { action_id });
        }
        Command::ComputerSecretTarget => {
            let capture_guard = computer_input_executions
                .capture_guard()
                .map_err(controller_route_error)?;
            let target =
                super::tool_dispatch::capture_computer_secret_target(capture_guard).await?;
            return Ok(Response::ComputerSecretTarget { target });
        }
        Command::ComputerClipboardRead {
            actor_id,
            runtime_generation,
        } => {
            if actor_id.trim().is_empty() || runtime_generation == 0 {
                return Err(controller_route_error(
                    "environment_clipboard_invalid_authority_context",
                ));
            }
            let content =
                super::tool_dispatch::run_room_clipboard_read_authorized(Some(authorize_input))
                    .await?;
            return Ok(Response::ComputerClipboard { content });
        }
        command => command,
    };
    let session_id = session_id.to_string();
    let recovery_processes = processes.clone();
    let result = authorized_controller_task(state, move || match command {
        Command::CancelAction { execution_id } => {
            let accepted = computer_input_executions.cancel(&session_id, &execution_id)
                || processes.cancel_browser_action(&session_id, &execution_id);
            Ok(Response::CancellationRequested { accepted })
        }
        Command::CancelCookieImport { request_id } => Ok(Response::CancellationRequested {
            accepted: processes.cancel_browser_import_and_wait(&session_id, &request_id),
        }),
        Command::Acquire => processes
            .acquire(&session_id)
            .map(|snapshot| Response::Process { snapshot }),
        Command::Release => processes
            .release(&session_id)
            .map(|snapshot| Response::Process { snapshot }),
        Command::Reconcile {
            viewport,
            browser_bar_visible,
        } => processes
            .reconcile_browser(&session_id, &viewport, browser_bar_visible)
            .map(|reconciliation| Response::Reconciled { reconciliation }),
        Command::Artifact { request } => processes
            .browser_artifact(&session_id, &request)
            .map(|capture| Response::Artifact { capture }),
        Command::NoteObservation {
            target_id,
            document_id,
            quote,
        } => processes
            .note_observation(&session_id, &target_id, &document_id, quote.as_ref())
            .map(|observation| Response::NoteObservation { observation }),
        Command::Snapshot {
            target_id,
            document_id,
        } => processes
            .capture_browser_snapshot(&session_id, &target_id, &document_id)
            .map(|snapshot| Response::Snapshot { snapshot }),
        Command::Tab {
            execution_id,
            target_id,
            document_id,
            action,
        } => processes.perform_cancellable_browser_lifecycle(
            &session_id,
            &execution_id,
            &target_id,
            &document_id,
            &BrowserLifecycleOperation::Tab { action },
        ),
        Command::History {
            execution_id,
            target_id,
            document_id,
            action,
        } => processes.perform_cancellable_browser_lifecycle(
            &session_id,
            &execution_id,
            &target_id,
            &document_id,
            &BrowserLifecycleOperation::History { action },
        ),
        Command::Navigate {
            execution_id,
            target_id,
            document_id,
            url,
        } => processes.perform_cancellable_browser_lifecycle(
            &session_id,
            &execution_id,
            &target_id,
            &document_id,
            &BrowserLifecycleOperation::Navigate { url },
        ),
        Command::Wait {
            target_id,
            document_id,
            wait,
            timeout_ms,
        } => processes
            .wait_for_browser(&session_id, &target_id, &document_id, &wait, timeout_ms)
            .map(|result| Response::Wait { result }),
        Command::Dialog {
            execution_id,
            target_id,
            document_id,
            action,
        } => processes.perform_cancellable_browser_lifecycle(
            &session_id,
            &execution_id,
            &target_id,
            &document_id,
            &BrowserLifecycleOperation::Dialog { action },
        ),
        Command::RecoverLifecycle {
            execution_id,
            target_id,
            document_id,
            operation,
        } => processes.recover_cancellable_browser_lifecycle(
            &session_id,
            &execution_id,
            &target_id,
            &document_id,
            &operation,
        ),
        Command::ConfigureDownloads {
            execution_id,
            target_id,
            document_id,
        } => processes.perform_cancellable_browser_configuration(
            &session_id,
            &execution_id,
            &target_id,
            &document_id,
            crate::runtime::browser_controller_process::BrowserConfiguration::Downloads,
        ),
        Command::RecoverDownloadConfiguration {
            execution_id,
            target_id,
            document_id,
        } => processes.recover_cancellable_browser_configuration(
            &session_id,
            &execution_id,
            &target_id,
            &document_id,
            crate::runtime::browser_controller_process::BrowserConfiguration::Downloads,
        ),
        Command::CancelDownload { cancellation } => processes
            .cancel_browser_download(&session_id, &cancellation)
            .map(|result| Response::DownloadCancellation { result }),
        Command::Upload {
            execution_id,
            target_id,
            document_id,
            node_ref,
            files,
        } => processes.perform_cancellable_browser_upload(
            &session_id,
            &execution_id,
            &target_id,
            &document_id,
            &node_ref,
            &files,
        ),
        Command::RecoverUpload {
            execution_id,
            target_id,
            document_id,
            node_ref,
            files,
        } => processes.recover_cancellable_browser_upload(
            &session_id,
            &execution_id,
            &target_id,
            &document_id,
            &node_ref,
            &files,
        ),
        Command::Permission {
            execution_id,
            target_id,
            document_id,
            permission,
            setting,
        } => processes.perform_cancellable_browser_configuration(
            &session_id,
            &execution_id,
            &target_id,
            &document_id,
            crate::runtime::browser_controller_process::BrowserConfiguration::Permission {
                name: permission,
                setting,
            },
        ),
        Command::RecoverPermission {
            execution_id,
            target_id,
            document_id,
            permission,
            setting,
        } => processes.recover_cancellable_browser_configuration(
            &session_id,
            &execution_id,
            &target_id,
            &document_id,
            crate::runtime::browser_controller_process::BrowserConfiguration::Permission {
                name: permission,
                setting,
            },
        ),
        Command::AppView { request } => processes
            .app_view(&session_id, &request)
            .map(|result| Response::AppView { result }),
        Command::PollEvents {
            browser_generation,
            cursor,
            limit,
        } => processes
            .poll_browser_events(&session_id, browser_generation, cursor, limit)
            .map(|batch| Response::Events { batch }),
        Command::Action {
            execution_id,
            target_id,
            document_id,
            node_ref,
            action,
            timeout_ms,
        } => processes.perform_cancellable_browser_action(
            &session_id,
            &execution_id,
            &target_id,
            &document_id,
            &node_ref,
            &action,
            timeout_ms,
        ),
        Command::RecoverAction {
            execution_id,
            target_id,
            document_id,
            node_ref,
            action,
            timeout_ms,
        } => processes.recover_cancellable_browser_action(
            &session_id,
            &execution_id,
            &target_id,
            &document_id,
            &node_ref,
            &action,
            timeout_ms,
        ),
        Command::ComputerSecretTarget => {
            unreachable!("Computer focus executes before the blocking controller path")
        }
        Command::ComputerInput { .. } => {
            unreachable!("Computer input executes before the blocking controller path")
        }
        Command::ClearSecretObservation { .. } => {
            unreachable!("observation revocation executes before controller path")
        }
        Command::ComputerClipboardRead { .. } => {
            unreachable!("Computer clipboard reads execute before the blocking controller path")
        }
        Command::ImportCookies {
            binding,
            browser_generation,
            target_id,
            document_id,
            source_store_id,
            domains,
            partition_sites,
            overwrite,
            payload,
        } => processes.perform_cancellable_browser_import(
            &session_id,
            &binding,
            browser_generation,
            &target_id,
            &document_id,
            &source_store_id,
            &domains,
            &partition_sites,
            overwrite,
            &payload,
        ),
        Command::RecoverCookieImport { binding, target_id } => processes
            .recover_browser_cookie_import(&session_id, &binding, &target_id)
            .and_then(|result| {
                result.ok_or_else(|| "browser controller is unavailable".to_string())
            })
            .map(|()| Response::CookieImportRecovered),
    })
    .await?;
    match result {
        Err(message) if message == CONTROLLER_RESTARTED_BEFORE_OPERATION => {
            let process = recovery_processes
                .snapshot()
                .map_err(|error| controller_route_error(&error))?
                .ok_or_else(|| controller_route_error(&message))?;
            Ok(Response::RecoveryRequired { process })
        }
        result => result.map_err(|message| controller_route_error(&message)),
    }
}

const ROOM_SLICE_UNREACHABLE: &str = "room_slice_unreachable";

/// Only a refused connection to the selected private slice relay can be treated
/// as a missing slice. Shared relay outages and timeouts retain stop failures.
fn room_slice_unreachable(
    slice: &str,
    private_slice_relay: bool,
    error: DaemonError,
) -> DaemonError {
    match &error {
        DaemonError::LocalTransport { operation, message }
            if private_slice_relay
                && (operation.starts_with("connect relay")
                    || operation == &"connect temporary relay peer socket")
                && message.contains("Connection refused") =>
        {
            controller_route_error(&format!(
                "{ROOM_SLICE_UNREACHABLE}: the Room's slice `{slice}` is not reachable ({message}); start the slice and retry"
            ))
        }
        _ => error,
    }
}

pub(super) fn is_room_slice_unreachable(error: &DaemonError) -> bool {
    matches!(error, DaemonError::LocalTransport { message, .. } if message.starts_with(ROOM_SLICE_UNREACHABLE))
}

async fn authorized_controller_task(
    state: KernelRuntimeState,
    task: impl FnOnce() -> Result<Response, String> + Send + 'static,
) -> Result<Result<Response, String>, DaemonError> {
    tokio::task::spawn_blocking(move || {
        state
            .authorize_current_external_command()
            .map_err(|error| error.to_string())?;
        task()
    })
    .await
    .map_err(|error| controller_route_error(&error.to_string()))
}

pub(super) fn controller_route_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "browser_controller.route",
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_recovery_preserves_identity_and_never_redispatches_upload() {
        let command = Command::Upload {
            execution_id: "00000000000000000000000000000001".into(),
            target_id: "target-1".into(),
            document_id: "doc-1".into(),
            node_ref: "backend:1".into(),
            files: crate::runtime::browser_controller_file_transfer::BrowserUploadFiles::new(vec![
                "/workspace/report.txt".into(),
            ])
            .unwrap(),
        };
        let recovery = receipt_recovery_command(&command).unwrap();
        let mut expected = serde_json::to_value(&command).unwrap();
        expected["kind"] = "recover_upload".into();
        assert_eq!(serde_json::to_value(&recovery).unwrap(), expected);
        assert_eq!(receipt_recovery_command(&recovery), None);
    }

    #[test]
    fn physical_computer_input_has_no_transport_replay_command() {
        let command = Command::ComputerInput {
            action_id: "action-1".to_string(),
            actor_id: "user:owner-1".to_string(),
            runtime_generation: 1,
            viewport_revision: 1,
            desktop_pixel_width: 1280,
            desktop_pixel_height: 800,
            action:
                crate::transport::room_browser_controller::RoomComputerInputAction::PointerClick {
                    x: 20,
                    y: 30,
                    button:
                        crate::transport::room_browser_controller::RoomComputerPointerButton::Left,
                    click_count: 1,
                },
        };

        assert_eq!(receipt_recovery_command(&command), None);
    }
}
