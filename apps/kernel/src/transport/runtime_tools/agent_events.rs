//! MP-08 / MP-09 / MP-10 / MP-11 A02: regular-agent event lifecycle tools.
use super::*;
pub(crate) fn canonical_agent_event_tool_name(name: &str) -> Option<&'static str> {
    let name = name.strip_prefix("mcp__chariox__").unwrap_or(name);
    [
        "chariox.events.subscribe",
        "chariox.events.subscriptions",
        "chariox.events.unsubscribe",
        "chariox.events.inbox",
        "chariox.events.ack",
        "chariox.events.yield",
        "chariox.events.blocked",
    ]
    .into_iter()
    .find(|n| name == *n || name == n.replace('.', "_"))
}
pub(crate) fn agent_event_tool_specs() -> Vec<RuntimeToolSpec> {
    let common =
        serde_json::json!({"task_id":{"type":"string"},"origin_prompt_id":{"type":"string"}});
    let specs=[
        ("subscribe","Register an admitted live delegate/workflow source for this task; source identity never grants authority.",serde_json::json!({"source_id":{"type":"string"},"obligation_id":{"type":"string"},"source_cursor":{"type":"integer","minimum":0}}),vec!["source_id"]),
        ("unsubscribe","Invalidate this task registration. A covered wait is woken as source lost; unfinished obligations remain supervised.",serde_json::json!({"registration_id":{"type":"string"}}),vec!["registration_id"]),
        ("subscriptions","List the current task's admitted completion sources.",serde_json::json!({}),vec![]),
        ("inbox","Read attributed, untrusted durable events; keep provider acceptance separate from inbox acknowledgement.",serde_json::json!({"after":{"type":"integer","minimum":0}}),vec![]),
        ("ack","Acknowledge an inbox item. Set handled only after handling its real result/failure; uncertain delivery cannot be acknowledged away.",serde_json::json!({"sequence":{"type":"integer","minimum":1},"handled":{"type":"boolean","default":false}}),vec!["sequence"]),
        ("yield","Request waiting on named live registrations with a finite future deadline and coverage of every open obligation. End the provider turn after success; waiting commits only at native settlement.",serde_json::json!({"registration_ids":{"type":"array","items":{"type":"string"},"minItems":1,"uniqueItems":true},"inbox_cursor":{"type":"integer","minimum":0},"deadline_ms":{"type":"integer","minimum":1},"reason":{"type":"string"}}),vec!["registration_ids","inbox_cursor","deadline_ms","reason"]),
        ("blocked","Ask the owner for the exact missing action. One kernel-owned interaction appears on all terminals; obligations remain supervised.",serde_json::json!({"reason":{"type":"string","minLength":1}}),vec!["reason"]),
    ];
    specs.into_iter().map(|(name,description,extra,required)|{
        let mut properties=common.as_object().unwrap().clone();properties.extend(extra.as_object().unwrap().clone());
        let mut fields=vec!["task_id","origin_prompt_id"];fields.extend(required);
        RuntimeToolSpec{name:format!("chariox.events.{name}"),description:description.into(),input_schema:serde_json::json!({"type":"object","properties":properties,"required":fields,"additionalProperties":false})}
    }).collect()
}
