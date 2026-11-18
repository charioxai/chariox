use super::*;
pub const NOTIFICATION_SOURCES: &str = "chariox.notification_sources";
pub const NOTIFICATION_SUBSCRIBE: &str = "chariox.notification_subscribe";
pub const NOTIFICATION_UNSUBSCRIBE: &str = "chariox.notification_unsubscribe";
pub const NOTIFY_PERSON: &str = "chariox.notify_person";

/// The Room's notification tools: they keep their own descriptions and come with the room tools.
pub fn is_notification_tool(name: &str) -> bool {
    name.starts_with("chariox.notification_") || name == NOTIFY_PERSON
}

pub fn notification_tool_specs() -> Vec<RuntimeToolSpec> {
    vec![
        RuntimeToolSpec { name: NOTIFICATION_SOURCES.into(), description: "List this Room's workflow exit notifications and installed Apps' outgoing events, and for the Room owner's own agents the owner's workflow sources on its other kernels. Source identities grant no App capability or access to other Rooms.".into(), input_schema: serde_json::json!({"type":"object","properties":{},"additionalProperties":false}) },
        RuntimeToolSpec { name: NOTIFICATION_SUBSCRIBE.into(), description: "Subscribe yourself to one source returned by notification_sources. Every occurrence queues prompt with the quoted occurrence; write in prompt what you were asked and what to do, since you may wake with no memory of it. options picks values for the options the source lists (for example PR number 976); occurrences that differ wake no one. once ends the subscription after its first delivery. The queued row's Steer sends it into the running turn. No other agent can be named.".into(), input_schema: serde_json::json!({"type":"object","required":["source","prompt"],"properties":{"source":{"oneOf":[{"type":"object","required":["workflow"],"properties":{"workflow":{"type":"object","required":["source_id"],"properties":{"source_id":{"type":"string"}},"additionalProperties":false}},"additionalProperties":false},{"type":"object","required":["app"],"properties":{"app":{"type":"object","required":["installation_id","event_name"],"properties":{"installation_id":{"type":"string"},"event_name":{"type":"string"}},"additionalProperties":false}},"additionalProperties":false}]},"prompt":{"type":"string","minLength":1},"options":{"type":"array","items":{"type":"object","required":["name","value"],"properties":{"name":{"type":"string"},"value":{"type":["integer","string","boolean"]}},"additionalProperties":false}},"once":{"type":"boolean"}},"additionalProperties":false}) },
        RuntimeToolSpec { name: NOTIFICATION_UNSUBSCRIBE.into(), description: "Delete one of your own notification triggers and detach its subscription. Accepts only the trigger_id returned by notification_subscribe.".into(), input_schema: serde_json::json!({"type":"object","required":["trigger_id"],"properties":{"trigger_id":{"type":"string"}},"additionalProperties":false}) },
        RuntimeToolSpec { name: NOTIFY_PERSON.into(), description: "Send the Room's owner a for-your-information card in their Inbox, for example when the work they asked to hear about is done. You do not wait and are not woken by it; to ask for an answer use chariox.request_popup. title is one line; body is optional detail; link is a URL or a Chariox target; priority high sorts first. Returns card_id.".into(), input_schema: serde_json::json!({"type":"object","required":["title"],"properties":{"title":{"type":"string","minLength":1},"body":{"type":"string"},"link":{"type":"string"},"priority":{"type":"string","enum":["normal","high"]}},"additionalProperties":false}) },
    ]
}
