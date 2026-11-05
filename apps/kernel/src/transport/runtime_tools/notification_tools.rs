use super::*;
pub const NOTIFICATION_SOURCES: &str = "chariox.notification_sources";
pub const NOTIFICATION_SUBSCRIBE: &str = "chariox.notification_subscribe";
pub const NOTIFICATION_UNSUBSCRIBE: &str = "chariox.notification_unsubscribe";

pub fn notification_tool_specs() -> Vec<RuntimeToolSpec> {
    vec![
        RuntimeToolSpec { name: NOTIFICATION_SOURCES.into(), description: "List this Room's workflow exit notifications and installed Apps' outgoing events. Source identities grant no App capability or access to other Rooms.".into(), input_schema: serde_json::json!({"type":"object","properties":{},"additionalProperties":false}) },
        RuntimeToolSpec { name: NOTIFICATION_SUBSCRIBE.into(), description: "Subscribe yourself to one source returned by notification_sources. Every occurrence queues a prompt; there is no delivery policy. The queued row's Steer sends it into the running turn. No other agent can be named.".into(), input_schema: serde_json::json!({"type":"object","required":["source"],"properties":{"source":{"oneOf":[{"type":"object","required":["workflow"],"properties":{"workflow":{"type":"object","required":["source_id"],"properties":{"source_id":{"type":"string"}},"additionalProperties":false}},"additionalProperties":false},{"type":"object","required":["app"],"properties":{"app":{"type":"object","required":["installation_id","event_name"],"properties":{"installation_id":{"type":"string"},"event_name":{"type":"string"}},"additionalProperties":false}},"additionalProperties":false}]},"prompt":{"type":"string"}},"additionalProperties":false}) },
        RuntimeToolSpec { name: NOTIFICATION_UNSUBSCRIBE.into(), description: "Delete one of your own notification triggers and detach its subscription. Accepts only the trigger_id returned by notification_subscribe.".into(), input_schema: serde_json::json!({"type":"object","required":["trigger_id"],"properties":{"trigger_id":{"type":"string"}},"additionalProperties":false}) },
    ]
}
