use std::time::{Duration, Instant};

use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::error::DaemonError;
use crate::local::{
    RoomEnvironmentHumanAction, RoomEnvironmentPointerButton, SubmitRoomEnvironmentActionRequest,
};
use crate::session::{
    ActionAdmission, EnvironmentActionArguments, EnvironmentActionRequest, EnvironmentActionState,
    EnvironmentActionTerminal, EnvironmentActor, EnvironmentError, InputTarget,
    RoomEnvironmentSnapshot,
};
use crate::transport::room_browser_controller::{
    RoomBrowserControllerCommand, RoomBrowserControllerResult, RoomComputerInputAction,
    RoomComputerKeyboardInput, RoomComputerPointerButton, ROOM_COMPUTER_KEYBOARD_KEY_MAX_REPEAT,
    ROOM_COMPUTER_KEYBOARD_KEY_MAX_UTF8_BYTES, ROOM_COMPUTER_KEYBOARD_TEXT_MAX_UTF8_BYTES,
    ROOM_COMPUTER_SCROLL_MAX_STEPS,
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
        if let Some(fingerprint) = self.keyboard_input_idempotency_fingerprint(&request.action) {
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
        validate_human_action_freshness(&environment, &request)
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
            .and_then(|_| validate_human_action_freshness(&current, &request))
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
            .room_browser_controller_command(&request.session_id, command)
            .await;
        let terminal = match &execution {
            Ok(RoomBrowserControllerResult::ComputerInputApplied {
                action_id: returned_action_id,
            }) if returned_action_id == &action_id => EnvironmentActionTerminal::Completed,
            _ => EnvironmentActionTerminal::Failed,
        };
        let environment = self
            .finish_room_environment_action(&request.session_id, &action_id, terminal)
            .map_err(human_action_environment_error)?;
        match execution {
            Ok(RoomBrowserControllerResult::ComputerInputApplied {
                action_id: returned_action_id,
            }) if returned_action_id == action_id => Ok((action_id, environment)),
            Ok(_) => Err(human_action_dispatch_error(
                "environment_input_response_mismatch",
                "bound worker returned a mismatched Computer input response".to_string(),
            )),
            Err(error) => Err(human_action_dispatch_error(
                "environment_input_execution_failed",
                error.to_string(),
            )),
        }
    }

    async fn wait_for_human_action_admission(
        &self,
        session_id: &str,
        actor: &EnvironmentActor,
        action_id: &str,
    ) -> Result<(), DaemonError> {
        let started = Instant::now();
        loop {
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

    fn keyboard_input_idempotency_fingerprint(
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
    match &request.action {
        RoomEnvironmentHumanAction::PointerMove { x, y } => {
            if *x >= environment.viewport.desktop_pixel_width
                || *y >= environment.viewport.desktop_pixel_height
            {
                return Err(EnvironmentError::PointerOutOfBounds {
                    x: *x,
                    y: *y,
                    width: environment.viewport.desktop_pixel_width,
                    height: environment.viewport.desktop_pixel_height,
                });
            }
        }
        RoomEnvironmentHumanAction::PointerDrag {
            from_x,
            from_y,
            to_x,
            to_y,
            ..
        } => {
            for (x, y) in [(*from_x, *from_y), (*to_x, *to_y)] {
                if x >= environment.viewport.desktop_pixel_width
                    || y >= environment.viewport.desktop_pixel_height
                {
                    return Err(EnvironmentError::PointerOutOfBounds {
                        x,
                        y,
                        width: environment.viewport.desktop_pixel_width,
                        height: environment.viewport.desktop_pixel_height,
                    });
                }
            }
        }
        RoomEnvironmentHumanAction::PointerScroll {
            x,
            y,
            horizontal_steps,
            vertical_steps,
        } => {
            if *x >= environment.viewport.desktop_pixel_width
                || *y >= environment.viewport.desktop_pixel_height
            {
                return Err(EnvironmentError::PointerOutOfBounds {
                    x: *x,
                    y: *y,
                    width: environment.viewport.desktop_pixel_width,
                    height: environment.viewport.desktop_pixel_height,
                });
            }
            if (*horizontal_steps == 0 && *vertical_steps == 0)
                || horizontal_steps.unsigned_abs() > ROOM_COMPUTER_SCROLL_MAX_STEPS
                || vertical_steps.unsigned_abs() > ROOM_COMPUTER_SCROLL_MAX_STEPS
            {
                return Err(EnvironmentError::InvalidScrollSteps {
                    horizontal_steps: *horizontal_steps,
                    vertical_steps: *vertical_steps,
                    max_steps: ROOM_COMPUTER_SCROLL_MAX_STEPS,
                });
            }
        }
        RoomEnvironmentHumanAction::KeyboardText { text } => {
            let utf8_byte_count = text.as_str().len();
            if utf8_byte_count == 0 || utf8_byte_count > ROOM_COMPUTER_KEYBOARD_TEXT_MAX_UTF8_BYTES
            {
                return Err(EnvironmentError::InvalidKeyboardText {
                    utf8_byte_count,
                    max_utf8_bytes: ROOM_COMPUTER_KEYBOARD_TEXT_MAX_UTF8_BYTES,
                });
            }
        }
        RoomEnvironmentHumanAction::KeyboardKey { key, repeat } => {
            let value = key.as_str();
            if value.is_empty()
                || value.len() > ROOM_COMPUTER_KEYBOARD_KEY_MAX_UTF8_BYTES
                || value.starts_with('-')
                || !value.bytes().all(|byte| byte.is_ascii_graphic())
            {
                return Err(EnvironmentError::InvalidKeyboardKey);
            }
            if *repeat == 0 || *repeat > ROOM_COMPUTER_KEYBOARD_KEY_MAX_REPEAT {
                return Err(EnvironmentError::InvalidKeyboardRepeat {
                    repeat: *repeat,
                    max_repeat: ROOM_COMPUTER_KEYBOARD_KEY_MAX_REPEAT,
                });
            }
        }
        RoomEnvironmentHumanAction::PointerClick {
            x, y, click_count, ..
        } => {
            if *x >= environment.viewport.desktop_pixel_width
                || *y >= environment.viewport.desktop_pixel_height
            {
                return Err(EnvironmentError::PointerOutOfBounds {
                    x: *x,
                    y: *y,
                    width: environment.viewport.desktop_pixel_width,
                    height: environment.viewport.desktop_pixel_height,
                });
            }
            if !matches!(*click_count, 1 | 2) {
                return Err(EnvironmentError::InvalidClickCount {
                    click_count: *click_count,
                });
            }
        }
    }
    Ok(())
}

fn computer_input_action(
    action: &RoomEnvironmentHumanAction,
    viewport_revision: u64,
) -> (
    &'static str,
    RoomComputerInputAction,
    EnvironmentActionArguments,
) {
    match action {
        RoomEnvironmentHumanAction::PointerMove { x, y } => (
            "pointer_move",
            RoomComputerInputAction::PointerMove { x: *x, y: *y },
            EnvironmentActionArguments::PointerMove {
                x: *x,
                y: *y,
                viewport_revision,
            },
        ),
        RoomEnvironmentHumanAction::PointerDrag {
            from_x,
            from_y,
            to_x,
            to_y,
            button,
        } => {
            let transport_button = computer_pointer_button(*button);
            (
                "pointer_drag",
                RoomComputerInputAction::PointerDrag {
                    from_x: *from_x,
                    from_y: *from_y,
                    to_x: *to_x,
                    to_y: *to_y,
                    button: transport_button,
                },
                EnvironmentActionArguments::PointerDrag {
                    from_x: *from_x,
                    from_y: *from_y,
                    to_x: *to_x,
                    to_y: *to_y,
                    button: *button,
                    viewport_revision,
                },
            )
        }
        RoomEnvironmentHumanAction::PointerScroll {
            x,
            y,
            horizontal_steps,
            vertical_steps,
        } => (
            "pointer_scroll",
            RoomComputerInputAction::PointerScroll {
                x: *x,
                y: *y,
                horizontal_steps: *horizontal_steps,
                vertical_steps: *vertical_steps,
            },
            EnvironmentActionArguments::PointerScroll {
                x: *x,
                y: *y,
                horizontal_steps: *horizontal_steps,
                vertical_steps: *vertical_steps,
                viewport_revision,
            },
        ),
        RoomEnvironmentHumanAction::KeyboardText { text } => (
            "keyboard_text",
            RoomComputerInputAction::KeyboardText {
                input: RoomComputerKeyboardInput::new(text.as_str().to_string()),
            },
            EnvironmentActionArguments::KeyboardText {
                utf8_byte_count: text.as_str().len() as u32,
                character_count: text.as_str().chars().count() as u32,
            },
        ),
        RoomEnvironmentHumanAction::KeyboardKey { key, repeat } => (
            "keyboard_key",
            RoomComputerInputAction::KeyboardKey {
                input: RoomComputerKeyboardInput::new(key.as_str().to_string()),
                repeat: *repeat,
            },
            EnvironmentActionArguments::KeyboardKey { repeat: *repeat },
        ),
        RoomEnvironmentHumanAction::PointerClick {
            x,
            y,
            button,
            click_count,
        } => {
            let transport_button = computer_pointer_button(*button);
            (
                "pointer_click",
                RoomComputerInputAction::PointerClick {
                    x: *x,
                    y: *y,
                    button: transport_button,
                    click_count: *click_count,
                },
                EnvironmentActionArguments::PointerClick {
                    x: *x,
                    y: *y,
                    button: *button,
                    click_count: *click_count,
                    viewport_revision,
                },
            )
        }
    }
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
