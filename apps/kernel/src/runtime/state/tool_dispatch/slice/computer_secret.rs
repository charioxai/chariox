// MP-08 / MP-11: approved native target and zeroizing stdin only.
use super::*;

pub(crate) async fn run_room_secret_text_input(
    input: crate::transport::room_browser_controller::RoomComputerSecretInput,
    expected_target: crate::transport::room_browser_controller::RoomComputerSecretTarget,
    cancellation: crate::runtime::computer_input_execution::ComputerInputCancellation,
) -> Result<(), DaemonError> {
    if !expected_target.valid() {
        return Err(room_computer_input_error(
            "computer credential input requires an approved display target",
        ));
    }
    let target = serde_json::to_string(&expected_target)
        .map_err(|_| room_computer_input_error("invalid approved display target"))?;
    let secret = input.into_zeroizing();
    let timeout = crate::runtime::computer_input_action::keyboard_text_timeout_ms(&secret);
    let output = run_slice_screen_command_inner_with_cancellation(
        vec!["computer-secret-paste-stdin".to_string(), target],
        Some(secret),
        Some(timeout),
        Some(cancellation),
    )
    .await?;
    if output.success {
        return Ok(());
    }
    // Never forward helper stdout/stderr, which could contain secret material.
    Err(room_computer_input_error(
        if output.status_code == Some(2) {
            "computer credential input aborted: focused control or window changed"
        } else {
            "computer credential input failed; no further keystrokes were sent"
        },
    ))
}

pub(crate) async fn capture_computer_secret_target(
    capture_guard: tokio::sync::OwnedRwLockReadGuard<()>,
) -> Result<crate::transport::room_browser_controller::RoomComputerSecretTarget, DaemonError> {
    let output =
        run_slice_screen_command_with_capture(vec!["computer-secret-target".into()], capture_guard)
            .await?;
    let target: crate::transport::room_browser_controller::RoomComputerSecretTarget =
        serde_json::from_str(output.stdout.as_str())
            .map_err(|_| room_computer_input_error("focused desktop target is unavailable"))?;
    if !output.success || output.stdout_truncated || !target.valid() {
        return Err(room_computer_input_error(
            "focused desktop target is unavailable",
        ));
    }
    Ok(target)
}
