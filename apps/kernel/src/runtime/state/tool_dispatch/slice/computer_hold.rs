//! MP-08/MP-10/MP-11: bounded physical holds keep Room ownership until release.
use crate::error::DaemonError;
use crate::runtime::computer_input_execution::ComputerInputCancellation;
use crate::transport::room_browser_controller::RoomComputerInputAction;

pub(crate) async fn run_room_computer_hold(
    action: RoomComputerInputAction,
    width: u32,
    height: u32,
    cancellation: ComputerInputCancellation,
) -> Result<(), DaemonError> {
    let viewport = crate::session::CanonicalViewport::new(width, height, 1, width, height)
        .map_err(|_| super::room_computer_input_error("environment_invalid_viewport"))?;
    crate::runtime::computer_input_action::validate_computer_input_action(&viewport, &action)
        .map_err(|error| super::room_computer_input_error(error.code()))?;
    let (args, stdin, duration_ms) = match action {
        RoomComputerInputAction::KeyboardHold { input, duration_ms } => (
            vec!["computer-key-hold-stdin".into(), duration_ms.to_string()],
            Some(input.into_zeroizing()), duration_ms,
        ),
        RoomComputerInputAction::PointerHold { x, y, button, duration_ms } => (
            vec!["pointer-hold".into(), x.to_string(), y.to_string(),
                match button {
                    crate::transport::room_browser_controller::RoomComputerPointerButton::Left => "left",
                    crate::transport::room_browser_controller::RoomComputerPointerButton::Middle => "middle",
                    crate::transport::room_browser_controller::RoomComputerPointerButton::Right => "right",
                }.into(), duration_ms.to_string()], None, duration_ms,
        ),
        _ => return Err(super::room_computer_input_error("environment_invalid_hold_action")),
    };
    let helper_lifecycle = cancellation.clone();
    let output = super::run_slice_screen_command_inner_with_cancellation(
        args,
        stdin,
        Some(u64::from(duration_ms) + super::ROOM_COMPUTER_INPUT_TIMEOUT_MS),
        Some(cancellation),
    )
    .await;
    let output = match output {
        Ok(output) => output,
        // Cancellation is reset by the controller before acknowledging it.
        Err(error @ DaemonError::BrowserControllerActionCancelled { .. }) => return Err(error),
        Err(error) => {
            if helper_lifecycle.process_group_was_terminated() {
                super::reset_room_computer_input().await?;
            }
            return Err(error);
        }
    };
    // Sanitized helper denials release their own input. An abrupt death may not.
    if !output.success {
        if output.status_code.is_none() || output.status_code == Some(137) {
            super::reset_room_computer_input().await?;
        }
        return Err(super::room_computer_input_error(
            "computer hold helper failed",
        ));
    }
    Ok(())
}
