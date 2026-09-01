use std::time::{Duration, Instant};

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
    RoomComputerPointerButton,
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

        let (input_action, arguments) =
            computer_input_action(&request.action, request.viewport_revision);
        let action_request = EnvironmentActionRequest::computer_mutation(
            &actor.actor_id,
            request.runtime_generation,
            "pointer_click",
            environment.focused_tab_id.as_deref(),
        )
        .with_idempotency_key(request.idempotency_key.trim())
        .with_arguments(arguments);
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
) -> (RoomComputerInputAction, EnvironmentActionArguments) {
    match action {
        RoomEnvironmentHumanAction::PointerClick {
            x,
            y,
            button,
            click_count,
        } => {
            let transport_button = match button {
                RoomEnvironmentPointerButton::Left => RoomComputerPointerButton::Left,
                RoomEnvironmentPointerButton::Middle => RoomComputerPointerButton::Middle,
                RoomEnvironmentPointerButton::Right => RoomComputerPointerButton::Right,
            };
            (
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

fn human_action_environment_error(error: EnvironmentError) -> DaemonError {
    human_action_dispatch_error(error.code(), format!("{error:?}"))
}

fn human_action_dispatch_error(code: &'static str, message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "environment.action.submit",
        message: format!("{code}: {message}"),
    }
}
