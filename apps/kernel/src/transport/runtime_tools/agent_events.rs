//! MP-08 / MP-09 / MP-10 / MP-11 A02: regular-agent event lifecycle tools.
use super::*;

const TOOLS: [&str; 7] = [
    "chariox.events.inbox",
    "chariox.events.ack",
    "chariox.events.yield",
    "chariox.events.blocked",
    "chariox.events.timer",
    "chariox.events.cancel_wake",
    "chariox.events.wakes",
];

pub(crate) fn canonical_agent_event_tool_name(name: &str) -> Option<&'static str> {
    let name = name.strip_prefix("mcp__chariox__").unwrap_or(name);
    TOOLS
        .into_iter()
        .find(|n| name == *n || name == n.replace('.', "_"))
}

pub(crate) fn agent_event_tool_specs() -> Vec<RuntimeToolSpec> {
    let common =
        serde_json::json!({"task_id":{"type":"string"},"origin_prompt_id":{"type":"string"}});
    let specs = [
        (
            "inbox",
            "Read attributed, untrusted durable events; keep provider acceptance separate from inbox acknowledgement. A completion event can omit the source answer. Do not rerun completed work just because the event payload omits its answer.",
            serde_json::json!({"after":{"type":"integer","minimum":0}}),
            vec![],
        ),
        (
            "ack",
            "Acknowledge an inbox item. Set handled only after handling its real result/failure; uncertain delivery cannot be acknowledged away.",
            serde_json::json!({"sequence":{"type":"integer","minimum":1},"handled":{"type":"boolean","default":false}}),
            vec!["sequence"],
        ),
        (
            "yield",
            "Request waiting on named live registrations with a finite future deadline and coverage of every open obligation. After success, immediately end this provider turn WITHOUT ANY MORE TOOLS. Do not read inbox or wakes to confirm waiting: native settlement commits it and the kernel delivers a new turn for the source/deadline. A repeated yield cannot extend the accepted deadline.",
            serde_json::json!({"registration_ids":{"type":"array","items":{"type":"string"},"minItems":1,"uniqueItems":true},"inbox_cursor":{"type":"integer","minimum":0},"deadline_ms":{"type":"integer","minimum":1},"reason":{"type":"string"}}),
            vec!["registration_ids", "inbox_cursor", "deadline_ms", "reason"],
        ),
        (
            "timer",
            "Arm a kernel-owned durable timer for this task: one-shot after delay_ms, or recurring every interval_ms. It survives kernel restart, client exit and host sleep; missed fires coalesce into one occurrence with a count. Returns the registration_id to yield on and whether the scheduler confirmed it. An armed timer is an open obligation until it fires or you cancel it. Timer acknowledgements alone are not useful progress; three consecutive wakes without useful progress require owner resume or cancellation.",
            serde_json::json!({"label":{"type":"string","minLength":1,"maxLength":200},"delay_ms":{"type":"integer","minimum":1000},"interval_ms":{"type":"integer","minimum":60000}}),
            vec!["label", "delay_ms"],
        ),
        (
            "cancel_wake",
            "Request cancellation of one of your armed wakes, including one retained by an earlier task. Pass this running turn's task_id and origin_prompt_id. The original task keeps ownership and receives the cancellation outcome; a source that is still stopping stays unresolved until it has stopped.",
            serde_json::json!({"wake_id":{"type":"string"}}),
            vec!["wake_id"],
        ),
        (
            "wakes",
            "List your armed and recent wakes with next fire time, last fired, delivered and acknowledged times, plus per-fire receipts and scheduler health.",
            serde_json::json!({}),
            vec![],
        ),
        (
            "blocked",
            "Ask the owner for the exact missing action. One kernel-owned interaction appears on all terminals; obligations remain supervised.",
            serde_json::json!({"reason":{"type":"string","minLength":1}}),
            vec!["reason"],
        ),
    ];
    specs
        .into_iter()
        .map(|(name, description, extra, required)| {
            let mut properties = common.as_object().unwrap().clone();
            properties.extend(extra.as_object().unwrap().clone());
            let mut fields = vec!["task_id", "origin_prompt_id"];
            fields.extend(required);
            RuntimeToolSpec {
                name: format!("chariox.events.{name}"),
                description: description.into(),
                input_schema: serde_json::json!({"type":"object","properties":properties,"required":fields,"additionalProperties":false}),
            }
        })
        .collect()
}
