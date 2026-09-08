use super::types::{BrowserError, BrowserInput, BrowserReference, ScreencastFrame};
use base64::Engine;
use serde_json::{json, Value};

pub(super) fn input(input: &BrowserInput) -> (&'static str, Value) {
    match input {
        BrowserInput::Pointer { x, y, pressed } => (
            "Input.dispatchMouseEvent",
            json!({
                "type": if *pressed { "mousePressed" } else { "mouseReleased" },
                "x":x, "y":y, "button":"left", "clickCount":1,
            }),
        ),
        BrowserInput::Text(text) => ("Input.insertText", json!({"text":text})),
        BrowserInput::Key { key, pressed } => (
            "Input.dispatchKeyEvent",
            json!({
                "type":if *pressed {"keyDown"} else {"keyUp"}, "key":key,
                "windowsVirtualKeyCode": match key.as_str() {
                    "Enter"=>13, "Tab"=>9, "Escape"=>27, "Backspace"=>8, "Delete"=>46,
                    "ArrowLeft"=>37, "ArrowUp"=>38, "ArrowRight"=>39, "ArrowDown"=>40,
                    "Home"=>36, "End"=>35, "PageUp"=>33, "PageDown"=>34, _=>0,
                },
            }),
        ),
    }
}

pub(super) fn main_document(value: &Value) -> Result<(String, String), BrowserError> {
    let frame = &value["frameTree"]["frame"];
    Ok((identifier(&frame["id"])?, identifier(&frame["loaderId"])?))
}
pub(super) fn identifier(value: &Value) -> Result<String, BrowserError> {
    value
        .as_str()
        .filter(|s| super::types::identifier(s))
        .map(str::to_owned)
        .ok_or(BrowserError::Protocol)
}
pub(super) fn accessibility(value: Value) -> Result<Value, BrowserError> {
    let nodes = value["nodes"].as_array().ok_or(BrowserError::Protocol)?;
    if nodes.len() > 4096 {
        return Err(BrowserError::Protocol);
    }
    // Preserve Chromium's roles, names, relationships and live/focus state;
    // this is an observation, never an alternative App-to-kernel channel.
    for node in nodes {
        if !node.is_object() || node["nodeId"].as_str().is_none_or(|id| id.len() > 128) {
            return Err(BrowserError::Protocol);
        }
    }
    Ok(value)
}
pub(super) fn frame(
    value: &Value,
    reference: BrowserReference,
) -> Result<(ScreencastFrame, u64), BrowserError> {
    let session = value["sessionId"]
        .as_u64()
        .filter(|id| *id <= i32::MAX as u64)
        .ok_or(BrowserError::Protocol)?;
    let encoded = value["data"]
        .as_str()
        .filter(|data| data.len() <= 1536 * 1024)
        .ok_or(BrowserError::Protocol)?;
    let jpeg = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| BrowserError::Protocol)?;
    if jpeg.len() < 4 || !jpeg.starts_with(&[0xff, 0xd8]) {
        return Err(BrowserError::Protocol);
    }
    let dimension = |key| {
        value["metadata"][key]
            .as_f64()
            .filter(|n| n.is_finite() && *n >= 1.0 && *n <= 4096.0 && n.fract() == 0.0)
            .map(|n| n as u32)
            .ok_or(BrowserError::Protocol)
    };
    Ok((
        ScreencastFrame {
            reference,
            jpeg,
            css_width: dimension("deviceWidth")?,
            css_height: dimension("deviceHeight")?,
        },
        session,
    ))
}
