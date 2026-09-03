use super::*;

pub fn slice_runtime_tool_specs() -> Vec<RuntimeToolSpec> {
    let canonical = vec![
        RuntimeToolSpec {
            name: SLICE_SCREEN_STATUS_TOOL.to_string(),
            description: "Return availability and display dimensions for the shared Chariox Computer. A local slice may also return its private viewer URL; a Room agent receives canonical Room dimensions and a client-attachment marker instead of worker connection details.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SLICE_SCREENSHOT_TOOL.to_string(),
            description: "Capture the current shared Chariox Computer screen. Set return_image_base64 to receive a native MCP image block bounded to 16 MiB. A local slice may honor path; a Room-owned remote Computer returns opaque artifact metadata and never exposes its worker path.".to_string(),
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
            description: "Extract visible text from the shared Chariox Computer with the slice OCR engine. A Room agent may reuse an opaque artifact_id returned by slice_screenshot; local slices may use image_path. If both are omitted, Chariox captures a fresh screenshot first.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "image_path": {"type": "string"},
                    "artifact_id": {"type": "string", "minLength": 1}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SLICE_FIND_TEXT_TOOL.to_string(),
            description: "Locate every visible occurrence of text on the shared Chariox Computer in reading order and return native display-pixel bounding boxes and center points. The backward-compatible match field contains the first result, while matches and match_count describe the complete result. A Room agent may reuse an opaque artifact_id returned by slice_screenshot; local slices may use image_path. If both are omitted, Chariox captures a fresh screenshot first.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": {"type": "string", "minLength": 1, "maxLength": 4096},
                    "image_path": {"type": "string"},
                    "artifact_id": {"type": "string", "minLength": 1}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SLICE_MOUSE_TOOL.to_string(),
            description: "Control the shared Chariox Computer pointer through the Room action authority. Actions: move, click, double_click, scroll, drag. Room scroll requires x and y; amount is vertical steps and horizontal_steps is horizontal steps.".to_string(),
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
                    "amount": {"type": "integer"},
                    "horizontal_steps": {"type": "integer"},
                    "button": {"type": "string", "enum": ["left", "middle", "right"]}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SLICE_KEYBOARD_TOOL.to_string(),
            description: "Control the shared Chariox Computer keyboard through the Room action authority. Use action=type with text or action=key with an xdotool-compatible key name and optional repeat count.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["type", "key"]
                    },
                    "text": {"type": "string"},
                    "key": {"type": "string"},
                    "repeat": {"type": "integer", "minimum": 1, "maximum": 32}
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
            name: SLICE_BROWSER_EVENTS_TOOL.to_string(),
            description: "Poll the bounded, sanitized Room browser event stream. Use browser_generation from slice_browser_status and resume with next_cursor.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["browser_generation"],
                "properties": {
                    "browser_generation": {"type": "integer", "minimum": 1},
                    "cursor": {"type": "integer", "minimum": 0, "default": 0},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 200, "default": 100}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SLICE_BROWSER_DOWNLOADS_TOOL.to_string(),
            description: "Enable downloads for the focused Room browser tab. Download paths remain private to the slice.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SLICE_BROWSER_UPLOAD_TOOL.to_string(),
            description: "Upload bounded files from configured slice roots through an opaque field_id returned by slice_browser_status or slice_browser_find.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["field_id", "files"],
                "properties": {
                    "field_id": {"type": "string", "minLength": 1},
                    "files": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 20,
                        "items": {"type": "string", "minLength": 1, "maxLength": 4096}
                    }
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SLICE_BROWSER_PERMISSION_TOOL.to_string(),
            description: "Set a closed-list browser permission decision for the focused Room browser origin.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["permission", "setting"],
                "properties": {
                    "permission": {
                        "type": "string",
                        "enum": ["camera", "clipboard-read-write", "clipboard-sanitized-write", "display-capture", "geolocation", "local-fonts", "microphone", "midi", "midi-sysex", "notifications"]
                    },
                    "setting": {"type": "string", "enum": ["granted", "denied", "prompt"]}
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
        SLICE_BROWSER_EVENTS_TOOL => SLICE_BROWSER_EVENTS_TOOL_ALIAS,
        SLICE_BROWSER_DOWNLOADS_TOOL => SLICE_BROWSER_DOWNLOADS_TOOL_ALIAS,
        SLICE_BROWSER_UPLOAD_TOOL => SLICE_BROWSER_UPLOAD_TOOL_ALIAS,
        SLICE_BROWSER_PERMISSION_TOOL => SLICE_BROWSER_PERMISSION_TOOL_ALIAS,
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
        SLICE_BROWSER_EVENTS_TOOL
        | SLICE_BROWSER_EVENTS_TOOL_ALIAS
        | "chariox_slice_browser_events"
        | "mcp__chariox__slice_browser_events"
        | "mcp__chariox__chariox_slice_browser_events" => Some(SLICE_BROWSER_EVENTS_TOOL),
        SLICE_BROWSER_DOWNLOADS_TOOL
        | SLICE_BROWSER_DOWNLOADS_TOOL_ALIAS
        | "chariox_slice_browser_downloads"
        | "mcp__chariox__slice_browser_downloads"
        | "mcp__chariox__chariox_slice_browser_downloads" => Some(SLICE_BROWSER_DOWNLOADS_TOOL),
        SLICE_BROWSER_UPLOAD_TOOL
        | SLICE_BROWSER_UPLOAD_TOOL_ALIAS
        | "chariox_slice_browser_upload"
        | "mcp__chariox__slice_browser_upload"
        | "mcp__chariox__chariox_slice_browser_upload" => Some(SLICE_BROWSER_UPLOAD_TOOL),
        SLICE_BROWSER_PERMISSION_TOOL
        | SLICE_BROWSER_PERMISSION_TOOL_ALIAS
        | "chariox_slice_browser_permission"
        | "mcp__chariox__slice_browser_permission"
        | "mcp__chariox__chariox_slice_browser_permission" => Some(SLICE_BROWSER_PERMISSION_TOOL),
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
