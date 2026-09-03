use crate::session::{
    CanonicalViewport, EnvironmentActionArguments, EnvironmentError, EnvironmentPointerButton,
};
use crate::transport::room_browser_controller::{
    RoomComputerInputAction, RoomComputerPointerButton, ROOM_COMPUTER_CLIPBOARD_MAX_UTF8_BYTES,
    ROOM_COMPUTER_KEYBOARD_KEY_MAX_REPEAT, ROOM_COMPUTER_KEYBOARD_KEY_MAX_UTF8_BYTES,
    ROOM_COMPUTER_KEYBOARD_TEXT_MAX_UTF8_BYTES, ROOM_COMPUTER_SCROLL_MAX_STEPS,
};

pub(crate) struct ComputerInputActionMetadata {
    pub(crate) kind: &'static str,
    pub(crate) arguments: Option<EnvironmentActionArguments>,
}

pub(crate) fn keyboard_text_timeout_ms(text: &str) -> u64 {
    // Physical typing paces at 40 ms per character. Allow mapping/X11 work
    // and scheduling overhead without imposing a hidden shorter text limit.
    let characters = text
        .chars()
        .count()
        .min(ROOM_COMPUTER_KEYBOARD_TEXT_MAX_UTF8_BYTES);
    5_000 + characters as u64 * 100
}

pub(crate) fn computer_input_action_metadata(
    input: &RoomComputerInputAction,
    viewport_revision: u64,
) -> ComputerInputActionMetadata {
    let (kind, arguments) = match input {
        RoomComputerInputAction::PointerMove { x, y } => (
            "pointer_move",
            Some(EnvironmentActionArguments::PointerMove {
                x: *x,
                y: *y,
                viewport_revision,
            }),
        ),
        RoomComputerInputAction::PointerDrag {
            from_x,
            from_y,
            to_x,
            to_y,
            button,
        } => (
            "pointer_drag",
            Some(EnvironmentActionArguments::PointerDrag {
                from_x: *from_x,
                from_y: *from_y,
                to_x: *to_x,
                to_y: *to_y,
                button: environment_pointer_button(*button),
                viewport_revision,
            }),
        ),
        RoomComputerInputAction::PointerScroll {
            x,
            y,
            horizontal_steps,
            vertical_steps,
        } => (
            "pointer_scroll",
            Some(EnvironmentActionArguments::PointerScroll {
                x: *x,
                y: *y,
                horizontal_steps: *horizontal_steps,
                vertical_steps: *vertical_steps,
                viewport_revision,
            }),
        ),
        RoomComputerInputAction::KeyboardText { input } => (
            "keyboard_text",
            Some(EnvironmentActionArguments::KeyboardText {
                utf8_byte_count: input.as_str().len() as u32,
                character_count: input.as_str().chars().count() as u32,
            }),
        ),
        RoomComputerInputAction::KeyboardKey { repeat, .. } => (
            "keyboard_key",
            Some(EnvironmentActionArguments::KeyboardKey { repeat: *repeat }),
        ),
        RoomComputerInputAction::ClipboardWrite { text } => (
            "clipboard_write",
            Some(EnvironmentActionArguments::ClipboardWrite {
                utf8_byte_count: text.as_str().len() as u32,
                character_count: text.as_str().chars().count() as u32,
            }),
        ),
        RoomComputerInputAction::PointerClick {
            x,
            y,
            button,
            click_count,
        } => (
            "pointer_click",
            Some(EnvironmentActionArguments::PointerClick {
                x: *x,
                y: *y,
                button: environment_pointer_button(*button),
                click_count: *click_count,
                viewport_revision,
            }),
        ),
        RoomComputerInputAction::SecretText { .. } => ("secret_input", None),
    };
    ComputerInputActionMetadata { kind, arguments }
}

pub(crate) fn validate_computer_input_action(
    viewport: &CanonicalViewport,
    input: &RoomComputerInputAction,
) -> Result<(), EnvironmentError> {
    let validate_point = |x: u32, y: u32| {
        if x >= viewport.desktop_pixel_width || y >= viewport.desktop_pixel_height {
            Err(EnvironmentError::PointerOutOfBounds {
                x,
                y,
                width: viewport.desktop_pixel_width,
                height: viewport.desktop_pixel_height,
            })
        } else {
            Ok(())
        }
    };

    match input {
        RoomComputerInputAction::PointerMove { x, y } => validate_point(*x, *y),
        RoomComputerInputAction::PointerDrag {
            from_x,
            from_y,
            to_x,
            to_y,
            ..
        } => {
            validate_point(*from_x, *from_y)?;
            validate_point(*to_x, *to_y)
        }
        RoomComputerInputAction::PointerScroll {
            x,
            y,
            horizontal_steps,
            vertical_steps,
        } => {
            validate_point(*x, *y)?;
            if (*horizontal_steps == 0 && *vertical_steps == 0)
                || horizontal_steps.unsigned_abs() > ROOM_COMPUTER_SCROLL_MAX_STEPS
                || vertical_steps.unsigned_abs() > ROOM_COMPUTER_SCROLL_MAX_STEPS
            {
                Err(EnvironmentError::InvalidScrollSteps {
                    horizontal_steps: *horizontal_steps,
                    vertical_steps: *vertical_steps,
                    max_steps: ROOM_COMPUTER_SCROLL_MAX_STEPS,
                })
            } else {
                Ok(())
            }
        }
        RoomComputerInputAction::KeyboardText { input } => {
            let utf8_byte_count = input.as_str().len();
            if utf8_byte_count == 0 || utf8_byte_count > ROOM_COMPUTER_KEYBOARD_TEXT_MAX_UTF8_BYTES
            {
                Err(EnvironmentError::InvalidKeyboardText {
                    utf8_byte_count,
                    max_utf8_bytes: ROOM_COMPUTER_KEYBOARD_TEXT_MAX_UTF8_BYTES,
                })
            } else {
                Ok(())
            }
        }
        RoomComputerInputAction::KeyboardKey { input, repeat } => {
            let key = input.as_str();
            if key.is_empty()
                || key.len() > ROOM_COMPUTER_KEYBOARD_KEY_MAX_UTF8_BYTES
                || key.starts_with('-')
                || !key.bytes().all(|byte| byte.is_ascii_graphic())
            {
                return Err(EnvironmentError::InvalidKeyboardKey);
            }
            if *repeat == 0 || *repeat > ROOM_COMPUTER_KEYBOARD_KEY_MAX_REPEAT {
                Err(EnvironmentError::InvalidKeyboardRepeat {
                    repeat: *repeat,
                    max_repeat: ROOM_COMPUTER_KEYBOARD_KEY_MAX_REPEAT,
                })
            } else {
                Ok(())
            }
        }
        RoomComputerInputAction::ClipboardWrite { text } => {
            let utf8_byte_count = text.as_str().len();
            if utf8_byte_count > ROOM_COMPUTER_CLIPBOARD_MAX_UTF8_BYTES {
                Err(EnvironmentError::InvalidClipboardText {
                    utf8_byte_count,
                    max_utf8_bytes: ROOM_COMPUTER_CLIPBOARD_MAX_UTF8_BYTES,
                })
            } else {
                Ok(())
            }
        }
        RoomComputerInputAction::PointerClick {
            x, y, click_count, ..
        } => {
            validate_point(*x, *y)?;
            if matches!(*click_count, 1 | 2) {
                Ok(())
            } else {
                Err(EnvironmentError::InvalidClickCount {
                    click_count: *click_count,
                })
            }
        }
        RoomComputerInputAction::SecretText { .. } => Ok(()),
    }
}

fn environment_pointer_button(button: RoomComputerPointerButton) -> EnvironmentPointerButton {
    match button {
        RoomComputerPointerButton::Left => EnvironmentPointerButton::Left,
        RoomComputerPointerButton::Middle => EnvironmentPointerButton::Middle,
        RoomComputerPointerButton::Right => EnvironmentPointerButton::Right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::CanonicalViewport;

    fn viewport() -> CanonicalViewport {
        CanonicalViewport::new(1280, 800, 1, 1280, 800).expect("valid viewport")
    }

    #[test]
    fn shared_validator_rejects_invalid_provider_computer_input_before_execution() {
        assert!(matches!(
            validate_computer_input_action(
                &viewport(),
                &RoomComputerInputAction::PointerScroll {
                    x: 640,
                    y: 400,
                    horizontal_steps: 0,
                    vertical_steps: 0,
                },
            ),
            Err(crate::session::EnvironmentError::InvalidScrollSteps { .. })
        ));
        assert!(matches!(
            validate_computer_input_action(
                &viewport(),
                &RoomComputerInputAction::KeyboardKey {
                    input:
                        crate::transport::room_browser_controller::RoomComputerKeyboardInput::new(
                            "ctrl+p".to_string(),
                        ),
                    repeat: 33,
                },
            ),
            Err(crate::session::EnvironmentError::InvalidKeyboardRepeat { .. })
        ));
        assert!(matches!(
            validate_computer_input_action(
                &viewport(),
                &RoomComputerInputAction::PointerMove { x: 1280, y: 0 },
            ),
            Err(crate::session::EnvironmentError::PointerOutOfBounds { .. })
        ));
    }
}
