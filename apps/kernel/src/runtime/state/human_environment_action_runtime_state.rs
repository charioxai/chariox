use std::time::{Duration, Instant};

use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::error::DaemonError;
use crate::local::{
    ReadRoomEnvironmentClipboardRequest, RoomEnvironmentClipboardText, RoomEnvironmentHumanAction,
    RoomEnvironmentPointerButton, SubmitRoomEnvironmentActionRequest,
};
use crate::session::{
    ActionAdmission, EnvironmentActionArguments, EnvironmentActionRequest, EnvironmentActionState,
    EnvironmentActionTerminal, EnvironmentActor, EnvironmentError, InputTarget,
    RoomEnvironmentSnapshot,
};
use crate::transport::room_browser_controller::{
    RoomBrowserControllerCommand, RoomBrowserControllerResult, RoomComputerClipboardText,
    RoomComputerInputAction, RoomComputerKeyboardInput, RoomComputerPointerButton,
};

use super::KernelRuntimeState;

const HUMAN_ACTION_QUEUE_POLL_INTERVAL: Duration = Duration::from_millis(10);
const HUMAN_ACTION_QUEUE_WAIT_TIMEOUT: Duration = Duration::from_secs(5);
const HUMAN_ACTION_IDEMPOTENCY_KEY_MAX_BYTES: usize = 128;

impl KernelRuntimeState {
    pub(crate) async fn execute_human_room_environment_action(
        &self,
        request: SubmitRoomEnvironmentActionRequest,
        actor: EnvironmentActor,
    ) -> Result<(String, RoomEnvironmentSnapshot), DaemonError> {
        let _execution_guard = self
            .owned
            .environment_execution_gates
            .for_room(&request.session_id)
            .read_owned()
            .await;
        let environment = self
            .room_environment_snapshot(&request.session_id)
            .map_err(human_action_environment_error)?;
        validate_human_action_idempotency_key(&request).map_err(human_action_environment_error)?;

        let (action_kind, input_action, arguments) =
            computer_input_action(&request.action, request.viewport_revision);
        let mut action_request = EnvironmentActionRequest::computer_mutation(
            &actor.actor_id,
            request.runtime_generation,
            action_kind,
            environment.focused_tab_id.as_deref(),
        )
        .with_idempotency_key(request.idempotency_key.trim())
        .with_arguments(arguments);
        if let Some(fingerprint) = self.computer_input_idempotency_fingerprint(&request.action) {
            action_request = action_request.with_idempotency_fingerprint(fingerprint);
        }
        if let Some(ActionAdmission::Existing { action_id, .. }) = self
            .existing_room_environment_action(&request.session_id, &action_request)
            .map_err(human_action_environment_error)?
        {
            return Ok((action_id, environment));
        }
        validate_human_action_authority(&environment, &actor.actor_id)
            .map_err(human_action_environment_error)?;
        validate_human_action_freshness(&environment, &request, &input_action)
            .map_err(human_action_environment_error)?;
        let (admission, environment) = self
            .submit_room_environment_action(&request.session_id, action_request)
            .map_err(human_action_environment_error)?;
        let action_id = match admission {
            ActionAdmission::Accepted { action_id } => action_id,
            ActionAdmission::Existing { action_id, .. } => {
                return Ok((action_id, environment));
            }
            ActionAdmission::Queued { action_id, .. } => {
                self.wait_for_human_action_admission(&request.session_id, &actor, &action_id)
                    .await?;
                action_id
            }
            ActionAdmission::RejectedSaturated { capacity } => {
                return Err(human_action_dispatch_error(
                    "environment_action_queue_saturated",
                    format!("human Action queue reached its capacity of {capacity}"),
                ));
            }
            ActionAdmission::RejectedBusy {
                target,
                active_action_id,
            } => {
                return Err(human_action_dispatch_error(
                    "environment_action_busy",
                    format!("Action target {target:?} is reserved by `{active_action_id}`"),
                ));
            }
            ActionAdmission::RejectedTakeover {
                target,
                human_actor_id,
            } => {
                return Err(human_action_dispatch_error(
                    "environment_input_takeover_required",
                    format!("Action target {target:?} belongs to `{human_actor_id}`"),
                ));
            }
        };

        let current = self
            .room_environment_snapshot(&request.session_id)
            .map_err(human_action_environment_error)?;
        if let Err(error) = validate_human_action_authority(&current, &actor.actor_id)
            .and_then(|_| validate_human_action_freshness(&current, &request, &input_action))
        {
            let _ = self.finish_room_environment_action(
                &request.session_id,
                &action_id,
                EnvironmentActionTerminal::Failed,
            );
            return Err(human_action_environment_error(error));
        }

        let command = RoomBrowserControllerCommand::ComputerInput {
            action_id: action_id.clone(),
            actor_id: actor.actor_id,
            runtime_generation: request.runtime_generation,
            viewport_revision: request.viewport_revision,
            desktop_pixel_width: current.viewport.desktop_pixel_width,
            desktop_pixel_height: current.viewport.desktop_pixel_height,
            action: input_action,
        };
        let execution = self
            .await_cancellable_browser_action(
                &request.session_id,
                &action_id,
                &action_id,
                self.room_browser_controller_command(&request.session_id, command),
            )
            .await;
        let terminal = match &execution {
            Ok(RoomBrowserControllerResult::ComputerInputApplied {
                action_id: returned_action_id,
            }) if returned_action_id == &action_id => EnvironmentActionTerminal::Completed,
            Ok(RoomBrowserControllerResult::ActionCancelled { .. })
            | Err(DaemonError::BrowserControllerActionCancelled { .. }) => {
                EnvironmentActionTerminal::Cancelled
            }
            _ => EnvironmentActionTerminal::Failed,
        };
        let environment = self
            .finish_room_environment_action(&request.session_id, &action_id, terminal)
            .map_err(human_action_environment_error)?;
        match execution {
            Ok(RoomBrowserControllerResult::ComputerInputApplied {
                action_id: returned_action_id,
            }) if returned_action_id == action_id => {
                // Physical input can navigate Chromium without a structured
                // Browser action. Refresh the shared tab registry only after
                // the input has finished; Room reads must remain nonblocking
                // while an Action is running.
                let environment = if self.browser_controller_enabled_for_room(&request.session_id) {
                    self.reconcile_browser_controller_environment(&request.session_id)
                        .await
                        .unwrap_or(environment)
                } else {
                    environment
                };
                Ok((action_id, environment))
            }
            Ok(RoomBrowserControllerResult::ActionCancelled { controller_fenced }) => {
                Err(DaemonError::BrowserControllerActionCancelled { controller_fenced })
            }
            Ok(_) => Err(human_action_dispatch_error(
                "environment_input_response_mismatch",
                "bound worker returned a mismatched Computer input response".to_string(),
            )),
            Err(error @ DaemonError::BrowserControllerActionCancelled { .. }) => Err(error),
            Err(error) => Err(human_action_dispatch_error(
                "environment_input_execution_failed",
                error.to_string(),
            )),
        }
    }

    pub(crate) async fn read_human_room_environment_clipboard(
        &self,
        request: ReadRoomEnvironmentClipboardRequest,
        actor: EnvironmentActor,
    ) -> Result<RoomEnvironmentClipboardText, DaemonError> {
        let environment = self
            .room_environment_snapshot(&request.session_id)
            .map_err(human_clipboard_read_environment_error)?;
        validate_human_action_authority(&environment, &actor.actor_id)
            .map_err(human_clipboard_read_environment_error)?;
        if request.runtime_generation != environment.runtime_generation {
            return Err(human_clipboard_read_environment_error(
                EnvironmentError::StaleRuntimeGeneration {
                    expected: environment.runtime_generation,
                    actual: request.runtime_generation,
                },
            ));
        }
        if !matches!(
            environment.lifecycle,
            crate::session::EnvironmentLifecycle::Ready
                | crate::session::EnvironmentLifecycle::Degraded
        ) {
            return Err(human_clipboard_read_environment_error(
                EnvironmentError::EnvironmentNotReady {
                    lifecycle: environment.lifecycle,
                },
            ));
        }
        let result = self
            .room_browser_controller_command(
                &request.session_id,
                RoomBrowserControllerCommand::ComputerClipboardRead {
                    actor_id: actor.actor_id,
                    runtime_generation: request.runtime_generation,
                },
            )
            .await;
        match result {
            Ok(RoomBrowserControllerResult::ComputerClipboard { content }) => {
                validate_human_clipboard_read_content(content)
            }
            Ok(_) => Err(human_clipboard_read_dispatch_error(
                "environment_clipboard_response_mismatch",
                "bound worker returned a mismatched Computer clipboard response".to_string(),
            )),
            Err(error) => Err(human_clipboard_read_dispatch_error(
                "environment_clipboard_read_failed",
                error.to_string(),
            )),
        }
    }

    pub(super) async fn wait_for_human_action_admission(
        &self,
        session_id: &str,
        actor: &EnvironmentActor,
        action_id: &str,
    ) -> Result<(), DaemonError> {
        let started = Instant::now();
        loop {
            if let Err(error) = self.ensure_browser_import_execution_allowed(session_id) {
                let _ = self.cancel_unstarted_import_blocked_action(session_id, action_id);
                return Err(human_action_environment_error(error));
            }
            let environment = self
                .room_environment_snapshot(session_id)
                .map_err(human_action_environment_error)?;
            let action = environment
                .actions
                .iter()
                .find(|action| action.action_id == action_id)
                .ok_or_else(|| {
                    human_action_dispatch_error(
                        "environment_action_missing",
                        format!("queued human Action `{action_id}` disappeared"),
                    )
                })?;
            match action.state {
                EnvironmentActionState::Running => return Ok(()),
                EnvironmentActionState::Queued
                    if started.elapsed() < HUMAN_ACTION_QUEUE_WAIT_TIMEOUT =>
                {
                    tokio::time::sleep(HUMAN_ACTION_QUEUE_POLL_INTERVAL).await;
                }
                EnvironmentActionState::Queued => {
                    let _ = self.cancel_room_environment_action_as_actor(
                        session_id,
                        actor.clone(),
                        action_id,
                    );
                    return Err(human_action_dispatch_error(
                        "environment_action_busy",
                        format!("queued human Action `{action_id}` timed out"),
                    ));
                }
                state => {
                    return Err(human_action_dispatch_error(
                        "environment_action_terminal",
                        format!("queued human Action `{action_id}` became {state:?}"),
                    ));
                }
            }
        }
    }

    fn computer_input_idempotency_fingerprint(
        &self,
        action: &RoomEnvironmentHumanAction,
    ) -> Option<[u8; 32]> {
        let (domain, value) = match action {
            RoomEnvironmentHumanAction::KeyboardText { text } => {
                (b"text".as_slice(), text.as_str())
            }
            RoomEnvironmentHumanAction::KeyboardKey { key, .. } => {
                (b"key".as_slice(), key.as_str())
            }
            RoomEnvironmentHumanAction::ClipboardWrite { text } => {
                (b"clipboard".as_slice(), text.as_str())
            }
            _ => return None,
        };
        let config = self.owned.config_projection.snapshot();
        let mut fingerprint = Hmac::<Sha256>::new_from_slice(config.relay_private_key.as_bytes())
            .expect("HMAC accepts relay identity keys of any length");
        fingerprint.update(b"chariox-room-computer-keyboard-idempotency-v1\0");
        fingerprint.update(domain);
        fingerprint.update(b"\0");
        fingerprint.update(value.as_bytes());
        Some(fingerprint.finalize().into_bytes().into())
    }
}

fn validate_human_action_authority(
    environment: &RoomEnvironmentSnapshot,
    actor_id: &str,
) -> Result<(), EnvironmentError> {
    if !environment
        .input_ownership
        .iter()
        .any(|ownership| ownership.target == InputTarget::Desktop && ownership.actor_id == actor_id)
    {
        return Err(EnvironmentError::InputTakeoverRequired {
            actor_id: actor_id.to_string(),
        });
    }
    Ok(())
}

fn validate_human_action_idempotency_key(
    request: &SubmitRoomEnvironmentActionRequest,
) -> Result<(), EnvironmentError> {
    let idempotency_key = request.idempotency_key.trim();
    if idempotency_key.is_empty() || idempotency_key.len() > HUMAN_ACTION_IDEMPOTENCY_KEY_MAX_BYTES
    {
        return Err(EnvironmentError::InvalidIdempotencyKey);
    }
    Ok(())
}

fn validate_human_action_freshness(
    environment: &RoomEnvironmentSnapshot,
    request: &SubmitRoomEnvironmentActionRequest,
    input: &RoomComputerInputAction,
) -> Result<(), EnvironmentError> {
    if request.runtime_generation != environment.runtime_generation {
        return Err(EnvironmentError::StaleRuntimeGeneration {
            expected: environment.runtime_generation,
            actual: request.runtime_generation,
        });
    }
    if request.viewport_revision != environment.viewport.revision {
        return Err(EnvironmentError::StaleViewportRevision {
            expected: environment.viewport.revision,
            actual: request.viewport_revision,
        });
    }
    crate::runtime::computer_input_action::validate_computer_input_action(
        &environment.viewport,
        input,
    )
}

fn computer_input_action(
    action: &RoomEnvironmentHumanAction,
    viewport_revision: u64,
) -> (
    &'static str,
    RoomComputerInputAction,
    EnvironmentActionArguments,
) {
    let input = match action {
        RoomEnvironmentHumanAction::PointerMove { x, y } => {
            RoomComputerInputAction::PointerMove { x: *x, y: *y }
        }
        RoomEnvironmentHumanAction::PointerDrag {
            from_x,
            from_y,
            to_x,
            to_y,
            button,
        } => RoomComputerInputAction::PointerDrag {
            from_x: *from_x,
            from_y: *from_y,
            to_x: *to_x,
            to_y: *to_y,
            button: computer_pointer_button(*button),
        },
        RoomEnvironmentHumanAction::PointerScroll {
            x,
            y,
            horizontal_steps,
            vertical_steps,
        } => RoomComputerInputAction::PointerScroll {
            x: *x,
            y: *y,
            horizontal_steps: *horizontal_steps,
            vertical_steps: *vertical_steps,
        },
        RoomEnvironmentHumanAction::KeyboardText { text } => {
            RoomComputerInputAction::KeyboardText {
                input: RoomComputerKeyboardInput::new(text.as_str().to_string()),
            }
        }
        RoomEnvironmentHumanAction::KeyboardKey { key, repeat } => {
            RoomComputerInputAction::KeyboardKey {
                input: RoomComputerKeyboardInput::new(key.as_str().to_string()),
                repeat: *repeat,
            }
        }
        RoomEnvironmentHumanAction::ClipboardWrite { text } => {
            RoomComputerInputAction::ClipboardWrite {
                text: RoomComputerClipboardText::new(text.as_str().to_string()),
            }
        }
        RoomEnvironmentHumanAction::PointerClick {
            x,
            y,
            button,
            click_count,
        } => RoomComputerInputAction::PointerClick {
            x: *x,
            y: *y,
            button: computer_pointer_button(*button),
            click_count: *click_count,
        },
    };
    let metadata = crate::runtime::computer_input_action::computer_input_action_metadata(
        &input,
        viewport_revision,
    );
    (
        metadata.kind,
        input,
        metadata
            .arguments
            .expect("human Computer Actions always have redacted arguments"),
    )
}

fn computer_pointer_button(button: RoomEnvironmentPointerButton) -> RoomComputerPointerButton {
    match button {
        RoomEnvironmentPointerButton::Left => RoomComputerPointerButton::Left,
        RoomEnvironmentPointerButton::Middle => RoomComputerPointerButton::Middle,
        RoomEnvironmentPointerButton::Right => RoomComputerPointerButton::Right,
    }
}

fn human_action_environment_error(error: EnvironmentError) -> DaemonError {
    human_action_dispatch_error(error.code(), format!("{error:?}"))
}

fn human_action_dispatch_error(code: &'static str, message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "environment.action.submit",
        message: format!("{code}: {message}"),
    }
}

fn human_clipboard_read_environment_error(error: EnvironmentError) -> DaemonError {
    human_clipboard_read_dispatch_error(error.code(), format!("{error:?}"))
}

fn validate_human_clipboard_read_content(
    content: RoomComputerClipboardText,
) -> Result<RoomEnvironmentClipboardText, DaemonError> {
    let utf8_byte_count = content.as_str().len();
    let max_utf8_bytes =
        crate::transport::room_browser_controller::ROOM_COMPUTER_CLIPBOARD_MAX_UTF8_BYTES;
    if utf8_byte_count > max_utf8_bytes {
        return Err(human_clipboard_read_environment_error(
            EnvironmentError::InvalidClipboardText {
                utf8_byte_count,
                max_utf8_bytes,
            },
        ));
    }
    Ok(RoomEnvironmentClipboardText::from_zeroizing(
        content.into_zeroizing(),
    ))
}

fn human_clipboard_read_dispatch_error(code: &'static str, message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "environment.clipboard.read",
        message: format!("{code}: {message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::browser_controller_action_execution_runtime_state::
        computer_input_reconcile_test_support::{install_screen_tool, TestRoom, TestTools};
    use tokio::sync::oneshot;

    fn human_actor() -> EnvironmentActor {
        EnvironmentActor::new(
            "human:computer-input-test",
            crate::session::EnvironmentActorKind::Human,
            "Operator",
        )
    }

    fn grant_desktop_to_human(room: &TestRoom, actor: EnvironmentActor) {
        let (outcome, _) = room
            .runtime
            .request_room_environment_takeover_as_actor(
                &room.session_id,
                actor,
                InputTarget::Desktop,
            )
            .expect("human should take the idle desktop");
        assert!(matches!(outcome, crate::session::TakeoverOutcome::Granted));
    }

    fn human_pointer_move(room: &TestRoom, key: &str) -> SubmitRoomEnvironmentActionRequest {
        let environment = room
            .runtime
            .room_environment_snapshot(&room.session_id)
            .expect("Room snapshot should be available");
        SubmitRoomEnvironmentActionRequest {
            session_id: room.session_id.clone(),
            runtime_generation: environment.runtime_generation,
            viewport_revision: environment.viewport.revision,
            idempotency_key: key.to_string(),
            action: RoomEnvironmentHumanAction::PointerMove { x: 10, y: 10 },
        }
    }

    #[tokio::test]
    async fn completed_human_computer_input_reconciles_enabled_browser_controller_tabs() {
        let tools = TestTools::new("human-success");
        let _screen_tool = install_screen_tool(&tools.screen_tool);
        let mut room = TestRoom::new("human-success");
        room.enable_browser_controller(&tools).await;
        let actor = human_actor();
        grant_desktop_to_human(&room, actor.clone());

        let result = room
            .runtime
            .execute_human_room_environment_action(
                human_pointer_move(&room, "human-success"),
                actor,
            )
            .await;
        room.stop_browser_controller().await;

        let (_, environment) = result.expect("fake ComputerInputApplied should complete");
        assert_eq!(environment.tabs[0].title, "After input");
        assert_eq!(
            environment.actions.last().unwrap().state,
            EnvironmentActionState::Completed
        );
    }

    #[tokio::test]
    async fn human_computer_input_skips_reconcile_when_browser_controller_is_disabled() {
        let tools = TestTools::new("human-disabled");
        let _screen_tool = install_screen_tool(&tools.screen_tool);
        let room = TestRoom::new("human-disabled");
        let actor = human_actor();
        grant_desktop_to_human(&room, actor.clone());

        let result = room
            .runtime
            .execute_human_room_environment_action(
                human_pointer_move(&room, "human-disabled"),
                actor,
            )
            .await;

        let (_, environment) = result.expect("fake ComputerInputApplied should complete");
        assert_eq!(environment.tabs[0].title, "Before input");
        assert_eq!(
            environment.actions.last().unwrap().state,
            EnvironmentActionState::Completed
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn human_computer_input_skips_reconcile_while_another_room_action_is_running() {
        let tools = TestTools::new("human-busy");
        let _screen_tool = install_screen_tool(&tools.screen_tool);
        let mut room = TestRoom::new("human-busy");
        room.enable_browser_controller(&tools).await;
        let initial = room
            .runtime
            .room_environment_snapshot(&room.session_id)
            .expect("Room snapshot should be available");
        let tab = initial.tabs[0].clone();
        let (action_started_tx, action_started_rx) = oneshot::channel();
        let (release_action_tx, release_action_rx) = oneshot::channel();
        let runtime = room.runtime.clone();
        let session_id = room.session_id.clone();
        let agent_id = room.agent_id.clone();
        let tab_id = tab.tab_id.clone();
        let document_revision = tab.document_revision;
        let action = tokio::spawn(async move {
            runtime
                .execute_browser_mutation_as_agent(
                    &session_id,
                    &agent_id,
                    &tab_id,
                    document_revision,
                    "blocking-click",
                    None,
                    async move {
                        action_started_tx.send(()).ok();
                        release_action_rx
                            .await
                            .expect("blocking Room action should release");
                        Ok::<_, DaemonError>(())
                    },
                )
                .await
        });
        action_started_rx
            .await
            .expect("other action should be running");
        let actor = human_actor();
        grant_desktop_to_human(&room, actor.clone());

        let result = room
            .runtime
            .execute_human_room_environment_action(human_pointer_move(&room, "human-busy"), actor)
            .await;
        let after_input = room
            .runtime
            .room_environment_snapshot(&room.session_id)
            .expect("Room snapshot should remain available");
        let another_action_running = after_input
            .actions
            .iter()
            .any(|action| action.state == EnvironmentActionState::Running);
        let tab_title_after_input = after_input.tabs[0].title.clone();

        release_action_tx
            .send(())
            .expect("blocking action should release");
        let action_result = action.await.expect("other action task should join");
        room.stop_browser_controller().await;

        result.expect("fake ComputerInputApplied should complete");
        action_result.expect("other action should complete");
        assert!(
            another_action_running,
            "Room action must remain active at input completion"
        );
        assert_eq!(tab_title_after_input, "Before input");
    }

    #[test]
    fn oversized_worker_clipboard_result_fails_closed_without_exposing_content() {
        let secret = "s".repeat(
            crate::transport::room_browser_controller::ROOM_COMPUTER_CLIPBOARD_MAX_UTF8_BYTES + 1,
        );
        let error =
            validate_human_clipboard_read_content(RoomComputerClipboardText::new(secret.clone()))
                .expect_err("an oversized worker result must not cross the home-kernel boundary");
        let diagnostic = error.to_string();
        assert!(diagnostic.contains("environment_invalid_clipboard_text"));
        assert!(!diagnostic.contains(&secret));
    }
}
