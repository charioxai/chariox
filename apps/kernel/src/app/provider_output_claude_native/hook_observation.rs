//! MP-08 / MP-10 / MP-11: body-free native hook admission diagnostics.
use super::*;

pub(super) fn record(stage: &'static str, event: &Value, provider_run_id: Option<&str>) {
    if !std::env::var("CHARIOX_PERF_DIAGNOSTICS").is_ok_and(|v| v == "1") {
        return;
    }
    let prompt = event.get("prompt");
    let kind = match prompt {
        Some(Value::String(_)) => "string",
        Some(Value::Null) => "null",
        None => "missing",
        _ => "other",
    };
    let event_name = event
        .get("hook_event_name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let event_name = match event_name {
        "UserPromptSubmit"
        | "UserPromptSubmitReady"
        | "UserPromptSubmitInterrupted"
        | "SessionStart"
        | "SessionEnd"
        | "Stop"
        | "StopFailure"
        | "PermissionRequest" => event_name,
        _ => "other",
    };
    crate::logging::info_with_fields(
        "daemon.native_hook_timing",
        "native hook observed",
        serde_json::json!({
            "stage":stage, "provider_run_id":provider_run_id, "event":event_name, "prompt_kind":kind,
            "prompt_bytes":prompt.and_then(Value::as_str).map(str::len),
            "has_request":event.get("hook_context_request_id").and_then(Value::as_str).is_some(),
            "has_native_prompt":event.get("native_prompt_id").and_then(Value::as_str).is_some(),
        }),
    );
}
