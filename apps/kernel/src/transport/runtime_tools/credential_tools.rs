use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ManageCredentialVaultArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCredentialConfigInput {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<crate::config::UserCredentialSourceConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_hosts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_uses: Vec<crate::config::UserCredentialUse>,
    pub injection: crate::config::UserCredentialInjectionConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateGeneratedCredentialArgs {
    pub credential: RuntimeCredentialConfigInput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<GeneratedCredentialSecretGeneratorArgs>,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedCredentialSecretGeneratorArgs {
    #[serde(default = "default_generated_secret_kind")]
    pub kind: String,
    #[serde(default = "default_generated_secret_length")]
    pub length: usize,
    #[serde(default = "default_generated_secret_symbols")]
    pub symbols: bool,
    #[serde(default)]
    pub avoid_ambiguous: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestCredentialSecretArgs {
    pub credential: RuntimeCredentialConfigInput,
    pub prompt: RequestCredentialSecretPromptArgs,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestCredentialSecretPromptArgs {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_sec: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpRequestWithCredentialArgs {
    pub credential_id: String,
    pub url: String,
    #[serde(default = "default_http_method")]
    pub method: String,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub headers: std::collections::BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_json: Option<serde_json::Value>,
    #[serde(default = "default_http_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_http_max_response_bytes")]
    pub max_response_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendSecretToTerminalArgs {
    pub credential_id: String,
    #[serde(default = "default_append_newline")]
    pub append_newline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasteSecretToSliceArgs {
    pub credential_id: String,
    #[serde(default)]
    pub submit: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasteSecretToComputerArgs {
    pub credential_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestPopupChoiceArgs {
    pub id: String,
    pub label: String,
    pub reply: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<crate::session::RuntimeInteractionChoiceStyle>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestPopupCustomChoiceArgs {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestPopupArgs {
    pub message: String,
    pub choices: Vec<RequestPopupChoiceArgs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_choice: Option<RequestPopupCustomChoiceArgs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<crate::session::RuntimeInteractionLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_sec: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_on_timeout: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceScreenshotArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub return_image_base64: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceOcrArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceFindTextArgs {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceMouseArgs {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_x: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_y: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizontal_steps: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub button: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceKeyboardArgs {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<u16>,
}

#[derive(PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceClipboardWriteArgs {
    pub text: String,
}

impl SliceClipboardWriteArgs {
    pub(crate) fn into_zeroizing(mut self) -> zeroize::Zeroizing<String> {
        zeroize::Zeroizing::new(std::mem::take(&mut self.text))
    }
}

impl std::fmt::Debug for SliceClipboardWriteArgs {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SliceClipboardWriteArgs")
            .field("text", &"[redacted clipboard text]")
            .finish()
    }
}

impl zeroize::Zeroize for SliceClipboardWriteArgs {
    fn zeroize(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.text);
    }
}

impl Drop for SliceClipboardWriteArgs {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(self);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceOpenUrlArgs {
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceBrowserFindArgs {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceBrowserFillArgs {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceBrowserClickArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceBrowserSubmitArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceBrowserDialogArgs {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SliceBrowserEventsArgs {
    pub browser_generation: u64,
    #[serde(default)]
    pub cursor: u64,
    #[serde(default = "default_browser_event_limit")]
    pub limit: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SliceBrowserDownloadsArgs {
    #[serde(default)]
    pub cancel: Option<SliceBrowserDownloadCancelArgs>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SliceBrowserDownloadCancelArgs {
    pub browser_generation: u64,
    pub guid: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SliceBrowserUploadArgs {
    pub field_id: String,
    pub files: Vec<std::path::PathBuf>,
}

impl std::fmt::Debug for SliceBrowserUploadArgs {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SliceBrowserUploadArgs")
            .field("field_id", &self.field_id)
            .field("file_count", &self.files.len())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SliceBrowserPermissionArgs {
    pub permission: String,
    pub setting: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceBrowserWaitForTextArgs {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceBrowserWaitForSelectorArgs {
    pub selector: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceBrowserWaitForIdleArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

fn default_append_newline() -> bool {
    true
}

fn default_generated_secret_kind() -> String {
    "password".to_string()
}

fn default_generated_secret_length() -> usize {
    32
}

fn default_generated_secret_symbols() -> bool {
    true
}

fn default_browser_event_limit() -> u16 {
    100
}

fn default_http_method() -> String {
    "GET".to_string()
}

fn default_http_timeout_ms() -> u64 {
    30_000
}

fn default_http_max_response_bytes() -> u64 {
    1_048_576
}

pub fn credential_runtime_tool_specs() -> Vec<RuntimeToolSpec> {
    let canonical = vec![
        RuntimeToolSpec {
            name: LIST_CREDENTIAL_HANDLES_TOOL.to_string(),
            description: "List Chariox credential handles available to this runtime. Values are never returned; use a handle id with http_request_with_credential when a request needs a secret.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: CREATE_GENERATED_CREDENTIAL_TOOL.to_string(),
            description: "Create or update a vault-backed Chariox credential handle with a kernel-generated random password. The generated secret is stored in the vault and is never returned to the model.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["credential"],
                "properties": {
                    "credential": credential_creation_schema(),
                    "generator": {
                        "type": "object",
                        "properties": {
                            "kind": {"type": "string", "enum": ["password"]},
                            "length": {"type": "integer", "minimum": 12, "maximum": 256},
                            "symbols": {"type": "boolean"},
                            "avoid_ambiguous": {"type": "boolean"}
                        },
                        "additionalProperties": false
                    },
                    "overwrite": {"type": "boolean"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: REQUEST_CREDENTIAL_SECRET_TOOL.to_string(),
            description: "Ask the user for a credential secret through a redacted Chariox interaction, then store it as a vault-backed credential handle. The typed secret is never returned to the model.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["credential", "prompt"],
                "properties": {
                    "credential": credential_creation_schema(),
                    "prompt": {
                        "type": "object",
                        "required": ["message"],
                        "properties": {
                            "title": {"type": "string"},
                            "message": {"type": "string"},
                            "placeholder": {"type": "string"},
                            "min_length": {"type": "integer", "minimum": 1},
                            "max_length": {"type": "integer", "minimum": 1},
                            "timeout_sec": {"type": "integer", "minimum": 1}
                        },
                        "additionalProperties": false
                    },
                    "overwrite": {"type": "boolean"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: HTTP_REQUEST_WITH_CREDENTIAL_TOOL.to_string(),
            description: "Perform an HTTP request using a Chariox credential handle. Chariox resolves and injects/signs the secret outside the model context, enforces the handle policy, and returns only the HTTP status/body.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["credential_id", "url"],
                "properties": {
                    "credential_id": {"type": "string"},
                    "method": {
                        "type": "string",
                        "enum": ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"]
                    },
                    "url": {"type": "string"},
                    "headers": {
                        "type": "object",
                        "additionalProperties": {"type": "string"}
                    },
                    "body_text": {"type": "string"},
                    "body_json": {},
                    "timeout_ms": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Optional per-call timeout. Defaults to 30000."
                    },
                    "max_response_bytes": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Optional response body cap. Defaults to 1048576."
                    }
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SEND_SECRET_TO_TERMINAL_TOOL.to_string(),
            description: "Write a terminal credential directly to the current provider PTY stdin. The secret value is not returned, recorded as terminal input, or placed in model context.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["credential_id"],
                "properties": {
                    "credential_id": {"type": "string"},
                    "append_newline": {"type": "boolean"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: PASTE_SECRET_TO_SLICE_TOOL.to_string(),
            description: "Paste a browser credential into an editable password field after validating the current Chariox slice browser target. Unmasked fields are rejected before the secret is resolved, and the secret value is not returned to the model.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["credential_id"],
                "properties": {
                    "credential_id": {"type": "string"},
                    "submit": {"type": "boolean", "description": "Submit the containing browser form after filling. Defaults to false."},
                    "expected_host": {"type": "string", "description": "Optional expected current browser host. The paste fails before secret resolution if the browser is on a different host."},
                    "expected_url": {"type": "string", "description": "Optional expected current browser URL prefix. The paste fails before secret resolution if the browser URL does not start with this value."},
                    "selector": {"type": "string", "description": "Optional CSS selector for the intended fillable field."},
                    "field_id": {"type": "string", "description": "Optional opaque field id returned by slice_browser_find or slice_browser_status."}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: PASTE_SECRET_TO_COMPUTER_TOOL.to_string(),
            description: "After explicit user approval, type a computer credential into the already-focused desktop control without exposing the value to the model or clipboard. Use only when the user can verify that the focused control masks secret input.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["credential_id"],
                "properties": {
                    "credential_id": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: MANAGE_CREDENTIAL_VAULT_TOOL.to_string(),
            description: "Check, lock, or request the Chariox Vault unlock/extend popup for the current session. Passphrases and secrets are never returned to the model.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["status", "lock", "popup"],
                        "description": "Defaults to popup. status returns locked/unlocked metadata; lock clears in-memory vault keys; popup asks the user to unlock, extend, lock, or dismiss."
                    }
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: REQUEST_POPUP_TOOL.to_string(),
            description: "Request a synchronous Chariox popup in the current agent pane. The tool call blocks until the user answers or the timeout resolves. Set default_on_timeout to an existing choice id to return that reply on timeout; omit it to return status timed_out with no choice_id or reply.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["message", "choices"],
                "properties": {
                    "title": {"type": "string"},
                    "message": {"type": "string"},
                    "level": {
                        "type": "string",
                        "enum": ["info", "warning", "critical"]
                    },
                    "timeout_sec": {"type": "integer", "minimum": 1},
                    "default_on_timeout": {"type": "string"},
                    "custom_choice": {
                        "type": "object",
                        "required": ["id", "label"],
                        "properties": {
                            "id": {"type": "string"},
                            "label": {"type": "string"},
                            "placeholder": {"type": "string"},
                            "min_length": {"type": "integer", "minimum": 1},
                            "max_length": {"type": "integer", "minimum": 1}
                        },
                        "additionalProperties": false
                    },
                    "choices": {
                        "type": "array",
                        "minItems": 2,
                        "items": {
                            "type": "object",
                            "required": ["id", "label", "reply"],
                            "properties": {
                                "id": {"type": "string"},
                                "label": {"type": "string"},
                                "reply": {"type": "string"},
                                "style": {
                                    "type": "string",
                                    "enum": ["primary", "secondary", "danger"]
                                }
                            },
                            "additionalProperties": false
                        }
                    }
                },
                "additionalProperties": false
            }),
        },
    ];
    let aliases = canonical
        .iter()
        .filter_map(credential_alias_spec)
        .collect::<Vec<_>>();
    let mut specs = canonical;
    specs.extend(aliases);
    specs
}

fn credential_creation_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "required": ["id", "injection"],
        "properties": {
            "id": {"type": "string"},
            "description": {"type": "string"},
            "source": {
                "type": "object",
                "required": ["type", "key"],
                "properties": {
                    "type": {"type": "string", "enum": ["vault"]},
                    "key": {"type": "string"}
                },
                "additionalProperties": false
            },
            "allowed_hosts": {
                "type": "array",
                "items": {"type": "string"}
            },
            "allowed_uses": {
                "type": "array",
                "items": {
                    "type": "string",
                    "enum": ["http", "pty", "connector", "browser", "computer", "mcp"]
                }
            },
            "injection": {
                "type": "object",
                "required": ["kind"],
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["header", "query", "basic", "hmac", "pty", "browser", "computer"]
                    },
                    "name": {"type": "string"},
                    "value": {"type": "string"},
                    "username": {"type": "string"},
                    "timestamp_header": {"type": "string"},
                    "signature_header": {"type": "string"}
                },
                "additionalProperties": false
            }
        },
        "additionalProperties": false
    })
}

fn credential_alias_spec(spec: &RuntimeToolSpec) -> Option<RuntimeToolSpec> {
    let alias = match spec.name.as_str() {
        LIST_CREDENTIAL_HANDLES_TOOL => LIST_CREDENTIAL_HANDLES_TOOL_ALIAS,
        CREATE_GENERATED_CREDENTIAL_TOOL => CREATE_GENERATED_CREDENTIAL_TOOL_ALIAS,
        REQUEST_CREDENTIAL_SECRET_TOOL => REQUEST_CREDENTIAL_SECRET_TOOL_ALIAS,
        HTTP_REQUEST_WITH_CREDENTIAL_TOOL => HTTP_REQUEST_WITH_CREDENTIAL_TOOL_ALIAS,
        SEND_SECRET_TO_TERMINAL_TOOL => SEND_SECRET_TO_TERMINAL_TOOL_ALIAS,
        PASTE_SECRET_TO_SLICE_TOOL => PASTE_SECRET_TO_SLICE_TOOL_ALIAS,
        PASTE_SECRET_TO_COMPUTER_TOOL => PASTE_SECRET_TO_COMPUTER_TOOL_ALIAS,
        MANAGE_CREDENTIAL_VAULT_TOOL => MANAGE_CREDENTIAL_VAULT_TOOL_ALIAS,
        REQUEST_POPUP_TOOL => REQUEST_POPUP_TOOL_ALIAS,
        _ => return None,
    };
    let mut spec = spec.clone();
    spec.name = alias.to_string();
    spec.description = format!("{} Alias for `{}`.", spec.description, alias);
    Some(spec)
}

pub fn canonical_credential_tool_name(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        LIST_CREDENTIAL_HANDLES_TOOL
        | LIST_CREDENTIAL_HANDLES_TOOL_ALIAS
        | "chariox_list_credential_handles"
        | "mcp__chariox__list_credential_handles"
        | "mcp__chariox__chariox_list_credential_handles" => Some(LIST_CREDENTIAL_HANDLES_TOOL),
        CREATE_GENERATED_CREDENTIAL_TOOL
        | CREATE_GENERATED_CREDENTIAL_TOOL_ALIAS
        | "chariox_create_generated_credential"
        | "mcp__chariox__create_generated_credential"
        | "mcp__chariox__chariox_create_generated_credential" => {
            Some(CREATE_GENERATED_CREDENTIAL_TOOL)
        }
        REQUEST_CREDENTIAL_SECRET_TOOL
        | REQUEST_CREDENTIAL_SECRET_TOOL_ALIAS
        | "chariox_request_credential_secret"
        | "mcp__chariox__request_credential_secret"
        | "mcp__chariox__chariox_request_credential_secret" => Some(REQUEST_CREDENTIAL_SECRET_TOOL),
        HTTP_REQUEST_WITH_CREDENTIAL_TOOL
        | HTTP_REQUEST_WITH_CREDENTIAL_TOOL_ALIAS
        | "chariox_http_request_with_credential"
        | "mcp__chariox__http_request_with_credential"
        | "mcp__chariox__chariox_http_request_with_credential" => {
            Some(HTTP_REQUEST_WITH_CREDENTIAL_TOOL)
        }
        SEND_SECRET_TO_TERMINAL_TOOL
        | SEND_SECRET_TO_TERMINAL_TOOL_ALIAS
        | "chariox_send_secret_to_terminal"
        | "mcp__chariox__send_secret_to_terminal"
        | "mcp__chariox__chariox_send_secret_to_terminal" => Some(SEND_SECRET_TO_TERMINAL_TOOL),
        PASTE_SECRET_TO_SLICE_TOOL
        | PASTE_SECRET_TO_SLICE_TOOL_ALIAS
        | "chariox_paste_secret_to_slice"
        | "mcp__chariox__paste_secret_to_slice"
        | "mcp__chariox__chariox_paste_secret_to_slice" => Some(PASTE_SECRET_TO_SLICE_TOOL),
        PASTE_SECRET_TO_COMPUTER_TOOL
        | PASTE_SECRET_TO_COMPUTER_TOOL_ALIAS
        | "chariox_paste_secret_to_computer"
        | "mcp__chariox__paste_secret_to_computer"
        | "mcp__chariox__chariox_paste_secret_to_computer" => Some(PASTE_SECRET_TO_COMPUTER_TOOL),
        MANAGE_CREDENTIAL_VAULT_TOOL
        | MANAGE_CREDENTIAL_VAULT_TOOL_ALIAS
        | "chariox_manage_credential_vault"
        | "mcp__chariox__manage_credential_vault"
        | "mcp__chariox__chariox_manage_credential_vault" => Some(MANAGE_CREDENTIAL_VAULT_TOOL),
        REQUEST_POPUP_TOOL
        | REQUEST_POPUP_TOOL_ALIAS
        | "chariox_request_popup"
        | "mcp__chariox__request_popup"
        | "mcp__chariox__chariox_request_popup" => Some(REQUEST_POPUP_TOOL),
        _ => None,
    }
}
