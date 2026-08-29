use super::*;

pub fn slice_runtime_tool_specs() -> Vec<RuntimeToolSpec> {
    let canonical = vec![
        RuntimeToolSpec {
            name: SLICE_SCREEN_STATUS_TOOL.to_string(),
            description: "Return the Chariox slice display status, including screen size and the local noVNC viewer URL when available.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SLICE_SCREENSHOT_TOOL.to_string(),
            description: "Capture the current Chariox slice screen to a PNG file. Use return_image_base64 only when the image bytes are needed in the tool result.".to_string(),
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
            description: "Extract visible text from a slice screenshot with the slice OCR engine. If image_path is omitted, Chariox captures a fresh screenshot first.".to_string(),
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
            description: "Locate text on the slice screen and return its bounding box and center point. If image_path is omitted, Chariox captures a fresh screenshot first.".to_string(),
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
            description: "Control the Chariox slice virtual mouse in the active desktop application. Actions: move, click, double_click, scroll, drag. A click focuses the application under the pointer.".to_string(),
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
            description: "Control the Chariox slice virtual keyboard in the active desktop application. Click the intended application or field first, then use action=type with text or action=key with an xdotool-compatible key name.".to_string(),
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
            description: "Open a URL in the Chariox slice browser.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["url"],
                "properties": {
                    "url": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SLICE_BROWSER_STATUS_TOOL.to_string(),
            description: "Return DOM-level browser status for the Chariox slice browser, including URL, title, focused element, and visible fields/buttons/links.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SLICE_BROWSER_FIND_TOOL.to_string(),
            description: "Find visible browser fields, buttons, or links by label, placeholder, name, text, role, or selector.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": {"type": "string"},
                    "kind": {"type": "string", "enum": ["field", "button", "link", "any"]}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SLICE_BROWSER_FILL_TOOL.to_string(),
            description: "Fill a slice browser input, textarea, select, or contenteditable element by selector or field_id returned by slice_browser_find.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["text"],
                "properties": {
                    "selector": {"type": "string"},
                    "field_id": {"type": "string"},
                    "text": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SLICE_BROWSER_CLICK_TOOL.to_string(),
            description: "Click a slice browser element by selector or field_id returned by slice_browser_find.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": {"type": "string"},
                    "field_id": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SLICE_BROWSER_SUBMIT_TOOL.to_string(),
            description: "Submit the nearest form for a slice browser element by selector/field_id, or the currently focused form when omitted.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": {"type": "string"},
                    "field_id": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SLICE_BROWSER_DIALOG_TOOL.to_string(),
            description: "Accept or dismiss the currently open native JavaScript alert, confirm, or prompt in the Chariox slice browser.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {"type": "string", "enum": ["accept", "dismiss"]},
                    "prompt_text": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SLICE_BROWSER_TEXT_TOOL.to_string(),
            description: "Return the current slice browser document body text.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SLICE_BROWSER_WAIT_FOR_TEXT_TOOL.to_string(),
            description: "Wait until the slice browser document body contains text, keeping browser automation inside the runtime MCP instead of shell sleeps.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["text"],
                "properties": {
                    "text": {"type": "string"},
                    "timeout_ms": {"type": "integer", "minimum": 100, "maximum": 60000}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SLICE_BROWSER_WAIT_FOR_SELECTOR_TOOL.to_string(),
            description: "Wait until a selector exists and is visible in the slice browser.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["selector"],
                "properties": {
                    "selector": {"type": "string"},
                    "timeout_ms": {"type": "integer", "minimum": 100, "maximum": 60000}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SLICE_BROWSER_WAIT_FOR_IDLE_TOOL.to_string(),
            description: "Wait until the slice browser reports a complete document ready state.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "timeout_ms": {"type": "integer", "minimum": 100, "maximum": 60000}
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
        SLICE_BROWSER_STATUS_TOOL => SLICE_BROWSER_STATUS_TOOL_ALIAS,
        SLICE_BROWSER_FIND_TOOL => SLICE_BROWSER_FIND_TOOL_ALIAS,
        SLICE_BROWSER_FILL_TOOL => SLICE_BROWSER_FILL_TOOL_ALIAS,
        SLICE_BROWSER_CLICK_TOOL => SLICE_BROWSER_CLICK_TOOL_ALIAS,
        SLICE_BROWSER_SUBMIT_TOOL => SLICE_BROWSER_SUBMIT_TOOL_ALIAS,
        SLICE_BROWSER_DIALOG_TOOL => SLICE_BROWSER_DIALOG_TOOL_ALIAS,
        SLICE_BROWSER_TEXT_TOOL => SLICE_BROWSER_TEXT_TOOL_ALIAS,
        SLICE_BROWSER_WAIT_FOR_TEXT_TOOL => SLICE_BROWSER_WAIT_FOR_TEXT_TOOL_ALIAS,
        SLICE_BROWSER_WAIT_FOR_SELECTOR_TOOL => SLICE_BROWSER_WAIT_FOR_SELECTOR_TOOL_ALIAS,
        SLICE_BROWSER_WAIT_FOR_IDLE_TOOL => SLICE_BROWSER_WAIT_FOR_IDLE_TOOL_ALIAS,
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
        | "chariox_slice_screen_status"
        | "mcp__chariox__slice_screen_status"
        | "mcp__chariox__chariox_slice_screen_status" => Some(SLICE_SCREEN_STATUS_TOOL),
        SLICE_SCREENSHOT_TOOL
        | SLICE_SCREENSHOT_TOOL_ALIAS
        | "chariox_slice_screenshot"
        | "mcp__chariox__slice_screenshot"
        | "mcp__chariox__chariox_slice_screenshot" => Some(SLICE_SCREENSHOT_TOOL),
        SLICE_OCR_TOOL
        | SLICE_OCR_TOOL_ALIAS
        | "chariox_slice_ocr"
        | "mcp__chariox__slice_ocr"
        | "mcp__chariox__chariox_slice_ocr" => Some(SLICE_OCR_TOOL),
        SLICE_FIND_TEXT_TOOL
        | SLICE_FIND_TEXT_TOOL_ALIAS
        | "chariox_slice_find_text"
        | "mcp__chariox__slice_find_text"
        | "mcp__chariox__chariox_slice_find_text" => Some(SLICE_FIND_TEXT_TOOL),
        SLICE_MOUSE_TOOL
        | SLICE_MOUSE_TOOL_ALIAS
        | "chariox_slice_mouse"
        | "mcp__chariox__slice_mouse"
        | "mcp__chariox__chariox_slice_mouse" => Some(SLICE_MOUSE_TOOL),
        SLICE_KEYBOARD_TOOL
        | SLICE_KEYBOARD_TOOL_ALIAS
        | "chariox_slice_keyboard"
        | "mcp__chariox__slice_keyboard"
        | "mcp__chariox__chariox_slice_keyboard" => Some(SLICE_KEYBOARD_TOOL),
        SLICE_OPEN_URL_TOOL
        | SLICE_OPEN_URL_TOOL_ALIAS
        | "chariox_slice_open_url"
        | "mcp__chariox__slice_open_url"
        | "mcp__chariox__chariox_slice_open_url" => Some(SLICE_OPEN_URL_TOOL),
        SLICE_BROWSER_STATUS_TOOL
        | SLICE_BROWSER_STATUS_TOOL_ALIAS
        | "chariox_slice_browser_status"
        | "mcp__chariox__slice_browser_status"
        | "mcp__chariox__chariox_slice_browser_status" => Some(SLICE_BROWSER_STATUS_TOOL),
        SLICE_BROWSER_FIND_TOOL
        | SLICE_BROWSER_FIND_TOOL_ALIAS
        | "chariox_slice_browser_find"
        | "mcp__chariox__slice_browser_find"
        | "mcp__chariox__chariox_slice_browser_find" => Some(SLICE_BROWSER_FIND_TOOL),
        SLICE_BROWSER_FILL_TOOL
        | SLICE_BROWSER_FILL_TOOL_ALIAS
        | "chariox_slice_browser_fill"
        | "mcp__chariox__slice_browser_fill"
        | "mcp__chariox__chariox_slice_browser_fill" => Some(SLICE_BROWSER_FILL_TOOL),
        SLICE_BROWSER_CLICK_TOOL
        | SLICE_BROWSER_CLICK_TOOL_ALIAS
        | "chariox_slice_browser_click"
        | "mcp__chariox__slice_browser_click"
        | "mcp__chariox__chariox_slice_browser_click" => Some(SLICE_BROWSER_CLICK_TOOL),
        SLICE_BROWSER_SUBMIT_TOOL
        | SLICE_BROWSER_SUBMIT_TOOL_ALIAS
        | "chariox_slice_browser_submit"
        | "mcp__chariox__slice_browser_submit"
        | "mcp__chariox__chariox_slice_browser_submit" => Some(SLICE_BROWSER_SUBMIT_TOOL),
        SLICE_BROWSER_DIALOG_TOOL
        | SLICE_BROWSER_DIALOG_TOOL_ALIAS
        | "chariox_slice_browser_dialog"
        | "mcp__chariox__slice_browser_dialog"
        | "mcp__chariox__chariox_slice_browser_dialog" => Some(SLICE_BROWSER_DIALOG_TOOL),
        SLICE_BROWSER_TEXT_TOOL
        | SLICE_BROWSER_TEXT_TOOL_ALIAS
        | "chariox_slice_browser_text"
        | "mcp__chariox__slice_browser_text"
        | "mcp__chariox__chariox_slice_browser_text" => Some(SLICE_BROWSER_TEXT_TOOL),
        SLICE_BROWSER_WAIT_FOR_TEXT_TOOL
        | SLICE_BROWSER_WAIT_FOR_TEXT_TOOL_ALIAS
        | "chariox_slice_browser_wait_for_text"
        | "mcp__chariox__slice_browser_wait_for_text"
        | "mcp__chariox__chariox_slice_browser_wait_for_text" => {
            Some(SLICE_BROWSER_WAIT_FOR_TEXT_TOOL)
        }
        SLICE_BROWSER_WAIT_FOR_SELECTOR_TOOL
        | SLICE_BROWSER_WAIT_FOR_SELECTOR_TOOL_ALIAS
        | "chariox_slice_browser_wait_for_selector"
        | "mcp__chariox__slice_browser_wait_for_selector"
        | "mcp__chariox__chariox_slice_browser_wait_for_selector" => {
            Some(SLICE_BROWSER_WAIT_FOR_SELECTOR_TOOL)
        }
        SLICE_BROWSER_WAIT_FOR_IDLE_TOOL
        | SLICE_BROWSER_WAIT_FOR_IDLE_TOOL_ALIAS
        | "chariox_slice_browser_wait_for_idle"
        | "mcp__chariox__slice_browser_wait_for_idle"
        | "mcp__chariox__chariox_slice_browser_wait_for_idle" => {
            Some(SLICE_BROWSER_WAIT_FOR_IDLE_TOOL)
        }
        _ => None,
    }
}
