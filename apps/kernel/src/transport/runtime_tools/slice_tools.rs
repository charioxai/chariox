use super::*;

pub fn slice_runtime_tool_specs() -> Vec<RuntimeToolSpec> {
    let canonical = vec![
        RuntimeToolSpec {
            name: SLICE_SCREEN_STATUS_TOOL.to_string(),
            description: "Return the Arroba slice display status, including screen size and the local noVNC viewer URL when available.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SLICE_SCREENSHOT_TOOL.to_string(),
            description: "Capture the current Arroba slice screen to a PNG file. Use return_image_base64 only when the image bytes are needed in the tool result.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "return_image_base64": {"type": "boolean"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SLICE_OCR_TOOL.to_string(),
            description: "Extract visible text from a slice screenshot with the slice OCR engine. If image_path is omitted, Arroba captures a fresh screenshot first.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "image_path": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SLICE_FIND_TEXT_TOOL.to_string(),
            description: "Locate text on the slice screen and return its bounding box and center point. If image_path is omitted, Arroba captures a fresh screenshot first.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": {"type": "string"},
                    "image_path": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SLICE_MOUSE_TOOL.to_string(),
            description: "Control the Arroba slice virtual mouse. Actions: move, click, double_click, scroll, drag.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["move", "click", "double_click", "scroll", "drag"]
                    },
                    "x": {"type": "integer"},
                    "y": {"type": "integer"},
                    "to_x": {"type": "integer"},
                    "to_y": {"type": "integer"},
                    "amount": {"type": "integer"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SLICE_KEYBOARD_TOOL.to_string(),
            description: "Control the Arroba slice virtual keyboard. Use action=type with text or action=key with an xdotool-compatible key name.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["type", "key"]
                    },
                    "text": {"type": "string"},
                    "key": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SLICE_OPEN_URL_TOOL.to_string(),
            description: "Open a URL in the Arroba slice browser.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["url"],
                "properties": {
                    "url": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
    ];
    let aliases = canonical
        .iter()
        .filter_map(slice_alias_spec)
        .collect::<Vec<_>>();
    let mut specs = canonical;
    specs.extend(aliases);
    specs
}

fn slice_alias_spec(spec: &RuntimeToolSpec) -> Option<RuntimeToolSpec> {
    let alias = match spec.name.as_str() {
        SLICE_SCREEN_STATUS_TOOL => SLICE_SCREEN_STATUS_TOOL_ALIAS,
        SLICE_SCREENSHOT_TOOL => SLICE_SCREENSHOT_TOOL_ALIAS,
        SLICE_OCR_TOOL => SLICE_OCR_TOOL_ALIAS,
        SLICE_FIND_TEXT_TOOL => SLICE_FIND_TEXT_TOOL_ALIAS,
        SLICE_MOUSE_TOOL => SLICE_MOUSE_TOOL_ALIAS,
        SLICE_KEYBOARD_TOOL => SLICE_KEYBOARD_TOOL_ALIAS,
        SLICE_OPEN_URL_TOOL => SLICE_OPEN_URL_TOOL_ALIAS,
        _ => return None,
    };
    let mut spec = spec.clone();
    spec.name = alias.to_string();
    spec.description = format!(
        "{} Alias for `{}`.",
        spec.description,
        canonical_slice_tool_name(alias).unwrap_or(alias)
    );
    Some(spec)
}

pub fn canonical_slice_tool_name(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        SLICE_SCREEN_STATUS_TOOL
        | SLICE_SCREEN_STATUS_TOOL_ALIAS
        | "arroba_slice_screen_status"
        | "mcp__arroba__slice_screen_status"
        | "mcp__arroba__arroba_slice_screen_status" => Some(SLICE_SCREEN_STATUS_TOOL),
        SLICE_SCREENSHOT_TOOL
        | SLICE_SCREENSHOT_TOOL_ALIAS
        | "arroba_slice_screenshot"
        | "mcp__arroba__slice_screenshot"
        | "mcp__arroba__arroba_slice_screenshot" => Some(SLICE_SCREENSHOT_TOOL),
        SLICE_OCR_TOOL
        | SLICE_OCR_TOOL_ALIAS
        | "arroba_slice_ocr"
        | "mcp__arroba__slice_ocr"
        | "mcp__arroba__arroba_slice_ocr" => Some(SLICE_OCR_TOOL),
        SLICE_FIND_TEXT_TOOL
        | SLICE_FIND_TEXT_TOOL_ALIAS
        | "arroba_slice_find_text"
        | "mcp__arroba__slice_find_text"
        | "mcp__arroba__arroba_slice_find_text" => Some(SLICE_FIND_TEXT_TOOL),
        SLICE_MOUSE_TOOL
        | SLICE_MOUSE_TOOL_ALIAS
        | "arroba_slice_mouse"
        | "mcp__arroba__slice_mouse"
        | "mcp__arroba__arroba_slice_mouse" => Some(SLICE_MOUSE_TOOL),
        SLICE_KEYBOARD_TOOL
        | SLICE_KEYBOARD_TOOL_ALIAS
        | "arroba_slice_keyboard"
        | "mcp__arroba__slice_keyboard"
        | "mcp__arroba__arroba_slice_keyboard" => Some(SLICE_KEYBOARD_TOOL),
        SLICE_OPEN_URL_TOOL
        | SLICE_OPEN_URL_TOOL_ALIAS
        | "arroba_slice_open_url"
        | "mcp__arroba__slice_open_url"
        | "mcp__arroba__arroba_slice_open_url" => Some(SLICE_OPEN_URL_TOOL),
        _ => None,
    }
}
