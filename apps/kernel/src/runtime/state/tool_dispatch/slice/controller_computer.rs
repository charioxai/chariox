use crate::error::DaemonError;
use crate::runtime::state::KernelRuntimeState;
use crate::transport::room_browser_controller::{
    RoomComputerClipboardText, RoomComputerInputAction, RoomComputerKeyboardInput,
    RoomComputerPointerButton,
};
use crate::transport::runtime_tools::{
    RuntimeToolResult, SliceClipboardWriteArgs, SliceKeyboardArgs, SliceMouseArgs,
};

impl KernelRuntimeState {
    pub(super) async fn controller_computer_mouse_tool_result(
        &self,
        session_id: &str,
        slice_id: &str,
        agent_id: &str,
        args: SliceMouseArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let action = room_computer_mouse_action(args)?;
        self.controller_computer_input_tool_result(session_id, slice_id, agent_id, action)
            .await
    }

    pub(super) async fn controller_computer_keyboard_tool_result(
        &self,
        session_id: &str,
        slice_id: &str,
        agent_id: &str,
        args: SliceKeyboardArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let action = room_computer_keyboard_action(args)?;
        self.controller_computer_input_tool_result(session_id, slice_id, agent_id, action)
            .await
    }

    pub(super) async fn controller_computer_clipboard_write_tool_result(
        &self,
        session_id: &str,
        slice_id: &str,
        agent_id: &str,
        args: SliceClipboardWriteArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let action = room_computer_clipboard_write_action(args);
        self.controller_computer_input_tool_result(session_id, slice_id, agent_id, action)
            .await
    }

    pub(super) async fn controller_computer_input_tool_result(
        &self,
        session_id: &str,
        slice_id: &str,
        agent_id: &str,
        action: RoomComputerInputAction,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let execution = self
            .execute_computer_input_as_agent(session_id, agent_id, action)
            .await?;
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "source": "computer_controller",
                "slice_id": slice_id,
                "agent_id": agent_id,
                "actor_id": execution.actor_id,
                "action_id": execution.action_id,
                "action_kind": execution.action_kind,
                "session_id": session_id,
                "environment_id": execution.environment_id,
                "runtime_generation": execution.runtime_generation,
            }),
        })
    }
}

fn room_computer_mouse_action(
    args: SliceMouseArgs,
) -> Result<RoomComputerInputAction, DaemonError> {
    let button = room_computer_pointer_button(args.button.as_deref())?;
    match args.action.as_str() {
        "hold" => Ok(RoomComputerInputAction::PointerHold {
            x: coordinate(args.x, "x")?,
            y: coordinate(args.y, "y")?,
            button,
            duration_ms: required_duration(args.duration_ms)?,
        }),
        "move" => Ok(RoomComputerInputAction::PointerMove {
            x: coordinate(args.x, "x")?,
            y: coordinate(args.y, "y")?,
        }),
        "click" => Ok(RoomComputerInputAction::PointerClick {
            x: coordinate(args.x, "x")?,
            y: coordinate(args.y, "y")?,
            button,
            click_count: 1,
        }),
        "double_click" => Ok(RoomComputerInputAction::PointerClick {
            x: coordinate(args.x, "x")?,
            y: coordinate(args.y, "y")?,
            button,
            click_count: 2,
        }),
        "drag" => Ok(RoomComputerInputAction::PointerDrag {
            from_x: coordinate(args.x, "x")?,
            from_y: coordinate(args.y, "y")?,
            to_x: coordinate(args.to_x, "to_x")?,
            to_y: coordinate(args.to_y, "to_y")?,
            button,
        }),
        "scroll" => Ok(RoomComputerInputAction::PointerScroll {
            x: coordinate(args.x, "x")?,
            y: coordinate(args.y, "y")?,
            horizontal_steps: scroll_steps(args.horizontal_steps.unwrap_or(0))?,
            vertical_steps: scroll_steps(args.amount.unwrap_or(
                if args.horizontal_steps.is_some() {
                    0
                } else {
                    1
                },
            ))?,
        }),
        other => Err(computer_tool_error(
            "unsupported_action",
            format!("unsupported mouse action `{other}`"),
        )),
    }
}

fn room_computer_keyboard_action(
    args: SliceKeyboardArgs,
) -> Result<RoomComputerInputAction, DaemonError> {
    match args.action.as_str() {
        "hold" => {
            if args.repeat.is_some() || args.text.is_some() {
                return Err(computer_tool_error(
                    "invalid_hold",
                    "hold accepts key and duration_ms only".into(),
                ));
            }
            Ok(RoomComputerInputAction::KeyboardHold {
                input: RoomComputerKeyboardInput::new(required_text(args.key, "key")?),
                duration_ms: required_duration(args.duration_ms)?,
            })
        }
        "type" => Ok(RoomComputerInputAction::KeyboardText {
            input: RoomComputerKeyboardInput::new(required_text(args.text, "text")?),
        }),
        "key" => Ok(RoomComputerInputAction::KeyboardKey {
            input: RoomComputerKeyboardInput::new(required_text(args.key, "key")?),
            repeat: args.repeat.unwrap_or(1),
        }),
        other => Err(computer_tool_error(
            "unsupported_action",
            format!("unsupported keyboard action `{other}`"),
        )),
    }
}

fn room_computer_clipboard_write_action(args: SliceClipboardWriteArgs) -> RoomComputerInputAction {
    RoomComputerInputAction::ClipboardWrite {
        text: RoomComputerClipboardText::from_zeroizing(args.into_zeroizing()),
    }
}

fn required_duration(value: Option<u32>) -> Result<u32, DaemonError> {
    let value = value.ok_or_else(|| {
        computer_tool_error("missing_duration", "hold requires duration_ms".into())
    })?;
    crate::runtime::computer_input_action::validate_hold_duration(value).map_err(|error| {
        computer_tool_error(
            error.code(),
            "hold duration must be between 1 and 10000 ms".into(),
        )
    })?;
    Ok(value)
}

fn coordinate(value: Option<i64>, field: &'static str) -> Result<u32, DaemonError> {
    let value = value.ok_or_else(|| {
        computer_tool_error("missing_coordinate", format!("missing required `{field}`"))
    })?;
    u32::try_from(value).map_err(|_| {
        computer_tool_error(
            "invalid_coordinate",
            format!("`{field}` must be a non-negative 32-bit coordinate"),
        )
    })
}

fn scroll_steps(value: i64) -> Result<i16, DaemonError> {
    i16::try_from(value).map_err(|_| {
        computer_tool_error(
            "invalid_scroll_steps",
            "scroll steps must fit a signed 16-bit integer".to_string(),
        )
    })
}

fn required_text(value: Option<String>, field: &'static str) -> Result<String, DaemonError> {
    value
        .filter(|value| !value.is_empty())
        .ok_or_else(|| computer_tool_error("missing_input", format!("missing required `{field}`")))
}

fn room_computer_pointer_button(
    value: Option<&str>,
) -> Result<RoomComputerPointerButton, DaemonError> {
    match value.unwrap_or("left") {
        "left" => Ok(RoomComputerPointerButton::Left),
        "middle" => Ok(RoomComputerPointerButton::Middle),
        "right" => Ok(RoomComputerPointerButton::Right),
        other => Err(computer_tool_error(
            "invalid_button",
            format!("unsupported pointer button `{other}`"),
        )),
    }
}

fn computer_tool_error(code: &'static str, message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "runtime_tool_room_computer_input",
        message: format!("{code}: {message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_adapter_preserves_unicode_without_exposing_a_read_tool() {
        let action = room_computer_clipboard_write_action(SliceClipboardWriteArgs {
            text: "Clipboard Grüße 世界".to_string(),
        });
        assert_eq!(
            action,
            RoomComputerInputAction::ClipboardWrite {
                text: crate::transport::room_browser_controller::RoomComputerClipboardText::new(
                    "Clipboard Grüße 世界".to_string(),
                ),
            }
        );
        assert!(
            !crate::transport::runtime_tools::slice_runtime_tool_specs()
                .iter()
                .any(|spec| spec.name.contains("clipboard_read")),
            "agent runtime tools must not expose clipboard reads"
        );
    }

    #[test]
    fn mouse_adapter_preserves_buttons_scroll_axes_and_coordinates() {
        assert_eq!(
            room_computer_mouse_action(SliceMouseArgs {
                duration_ms: None,
                action: "scroll".to_string(),
                x: Some(640),
                y: Some(400),
                to_x: None,
                to_y: None,
                amount: Some(5),
                horizontal_steps: Some(-3),
                button: None,
            })
            .expect("scroll should adapt"),
            RoomComputerInputAction::PointerScroll {
                x: 640,
                y: 400,
                horizontal_steps: -3,
                vertical_steps: 5,
            }
        );
        assert!(room_computer_mouse_action(SliceMouseArgs {
            duration_ms: None,
            action: "click".to_string(),
            x: Some(-1),
            y: Some(0),
            to_x: None,
            to_y: None,
            amount: None,
            horizontal_steps: None,
            button: Some("left".to_string()),
        })
        .is_err());
    }

    #[test]
    fn mouse_adapter_preserves_the_legacy_default_scroll_step() {
        assert_eq!(
            room_computer_mouse_action(SliceMouseArgs {
                duration_ms: None,
                action: "scroll".to_string(),
                x: Some(640),
                y: Some(400),
                to_x: None,
                to_y: None,
                amount: None,
                horizontal_steps: None,
                button: None,
            })
            .expect("omitted scroll amount should keep the legacy default"),
            RoomComputerInputAction::PointerScroll {
                x: 640,
                y: 400,
                horizontal_steps: 0,
                vertical_steps: 1,
            }
        );
    }
    #[test]
    fn mp08_mp10_mp11_provider_hold_requires_an_explicit_bounded_duration() {
        for wire in [
            serde_json::json!({"action":"hold", "key":"shift+Left", "duration_ms":750}),
            serde_json::json!({"action":"hold", "x":12, "y":24, "button":"right", "duration_ms":750}),
        ] {
            let action = if wire.get("key").is_some() {
                room_computer_keyboard_action(serde_json::from_value(wire).unwrap()).unwrap()
            } else {
                room_computer_mouse_action(serde_json::from_value(wire).unwrap()).unwrap()
            };
            assert_eq!(
                crate::runtime::computer_input_action::computer_input_action_metadata(&action, 1)
                    .arguments
                    .unwrap(),
                match action {
                    RoomComputerInputAction::KeyboardHold { .. } =>
                        crate::session::EnvironmentActionArguments::KeyboardHold {
                            duration_ms: 750
                        },
                    _ => crate::session::EnvironmentActionArguments::PointerHold {
                        x: 12,
                        y: 24,
                        button: crate::session::EnvironmentPointerButton::Right,
                        duration_ms: 750,
                        viewport_revision: 1
                    },
                }
            );
        }
        for wire in [
            serde_json::json!({"action":"hold", "key":"Left"}),
            serde_json::json!({"action":"hold", "key":"Left", "duration_ms":0}),
            serde_json::json!({"action":"hold", "key":"Left", "duration_ms":10001}),
            serde_json::json!({"action":"hold", "key":"Left", "duration_ms":10, "repeat":1}),
        ] {
            assert!(room_computer_keyboard_action(serde_json::from_value(wire).unwrap()).is_err());
        }
    }
}
