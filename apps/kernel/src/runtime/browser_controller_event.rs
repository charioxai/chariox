use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

pub(crate) const MAX_BROWSER_EVENT_POLL_LIMIT: u16 = 200;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct BrowserControllerEventBatch {
    pub(crate) browser_generation: u64,
    pub(crate) events: Vec<BrowserControllerEvent>,
    pub(crate) next_cursor: u64,
    pub(crate) replay_gap: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct BrowserControllerEvent {
    pub(crate) event_id: u64,
    pub(crate) browser_generation: u64,
    pub(crate) kind: String,
    pub(crate) target_id: Option<String>,
    pub(crate) document_id: Option<String>,
    pub(crate) data: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RoomBrowserEventBatch {
    pub(crate) browser_generation: u64,
    pub(crate) events: Vec<RoomBrowserEvent>,
    pub(crate) next_cursor: u64,
    pub(crate) replay_gap: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RoomBrowserEvent {
    pub(crate) event_id: u64,
    pub(crate) kind: String,
    pub(crate) tab_id: Option<String>,
    pub(crate) document_id: Option<String>,
    pub(crate) data: BTreeMap<String, serde_json::Value>,
}

impl BrowserControllerEventBatch {
    pub(crate) fn validate(
        &self,
        expected_browser_generation: u64,
        cursor: u64,
        limit: u16,
    ) -> Result<(), String> {
        if expected_browser_generation == 0
            || self.browser_generation != expected_browser_generation
        {
            return Err(
                "browser controller event generation did not match the request".to_string(),
            );
        }
        if limit == 0 || limit > MAX_BROWSER_EVENT_POLL_LIMIT {
            return Err(format!(
                "browser event limit must be between 1 and {MAX_BROWSER_EVENT_POLL_LIMIT}"
            ));
        }
        if self.events.len() > usize::from(limit) {
            return Err("browser controller returned more events than requested".to_string());
        }
        if self.replay_gap && !self.events.is_empty() {
            return Err(
                "browser controller returned events together with a replay gap".to_string(),
            );
        }
        let mut previous = cursor;
        for event in &self.events {
            event.validate(expected_browser_generation)?;
            if event.event_id <= previous {
                return Err("browser controller events were not strictly ordered".to_string());
            }
            previous = event.event_id;
        }
        if !self.replay_gap && self.next_cursor < previous {
            return Err("browser controller event cursor moved backwards".to_string());
        }
        Ok(())
    }
}

impl BrowserControllerEvent {
    fn validate(&self, expected_browser_generation: u64) -> Result<(), String> {
        if self.event_id == 0 || self.browser_generation != expected_browser_generation {
            return Err("browser controller returned an invalid event identity".to_string());
        }
        validate_optional_identity(self.target_id.as_deref(), "target")?;
        validate_optional_identity(self.document_id.as_deref(), "document")?;
        let allowed = allowed_data_keys(&self.kind).ok_or_else(|| {
            format!(
                "browser controller returned unknown event kind `{}`",
                self.kind
            )
        })?;
        let actual: BTreeSet<&str> = self.data.keys().map(String::as_str).collect();
        if actual != allowed {
            return Err(format!(
                "browser controller event `{}` returned incomplete or unknown data",
                self.kind
            ));
        }
        validate_data_values(&self.data)?;
        if requires_target(&self.kind) && self.target_id.is_none() {
            return Err(format!(
                "browser controller event `{}` omitted its target",
                self.kind
            ));
        }
        Ok(())
    }
}

fn validate_optional_identity(value: Option<&str>, kind: &str) -> Result<(), String> {
    if value.is_some_and(|value| value.is_empty() || value.len() > 512) {
        return Err(format!(
            "browser controller returned an invalid {kind} identity"
        ));
    }
    Ok(())
}

fn validate_data_values(data: &BTreeMap<String, serde_json::Value>) -> Result<(), String> {
    for value in data.values() {
        match value {
            serde_json::Value::String(value) if value.len() <= 2_048 => {}
            serde_json::Value::Number(_) | serde_json::Value::Bool(_) | serde_json::Value::Null => {
            }
            _ => return Err("browser controller event data exceeded its scalar bounds".to_string()),
        }
    }
    Ok(())
}

fn requires_target(kind: &str) -> bool {
    !matches!(kind, "browser_connected" | "browser_disconnected")
}

fn allowed_data_keys(kind: &str) -> Option<BTreeSet<&'static str>> {
    let keys: &[&str] = match kind {
        "console" => &["console_type", "argument_count"],
        "network_request" => &["request_id", "method", "url", "resource_type"],
        "network_response" => &["request_id", "status", "url", "resource_type", "mime_type"],
        "network_failed" => &["request_id", "error_text", "canceled", "resource_type"],
        "page_navigated" | "target_created" | "target_changed" => &["url"],
        "dom_content_loaded"
        | "page_loaded"
        | "target_destroyed"
        | "browser_connected"
        | "browser_disconnected" => &[],
        "dialog_opened" => &["dialog_type", "has_message", "has_default_prompt"],
        "dialog_closed" => &["result", "user_input_present"],
        "target_crashed" => &["status", "error_code"],
        "download_started" => &["guid", "url", "suggested_filename"],
        "download_progress" => &["guid", "state", "received_bytes", "total_bytes"],
        _ => return None,
    };
    Some(keys.iter().copied().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_batch_rejects_payload_fields_outside_the_bounded_contract() {
        let batch: BrowserControllerEventBatch = serde_json::from_value(serde_json::json!({
            "browser_generation": 2,
            "events": [{
                "event_id": 1,
                "browser_generation": 2,
                "kind": "network_request",
                "target_id": "target-a",
                "document_id": "loader-a",
                "data": { "headers": { "authorization": "secret" } }
            }],
            "next_cursor": 1,
            "replay_gap": false
        }))
        .expect("event shape parses");

        assert!(batch.validate(2, 0, 10).is_err());
    }

    #[test]
    fn inspector_crash_payload_satisfies_the_target_crash_contract() {
        let batch: BrowserControllerEventBatch = serde_json::from_value(serde_json::json!({
            "browser_generation": 2,
            "events": [{
                "event_id": 1,
                "browser_generation": 2,
                "kind": "target_crashed",
                "target_id": "target-a",
                "document_id": "loader-a",
                "data": {"status": "crashed", "error_code": null}
            }],
            "next_cursor": 1,
            "replay_gap": false
        }))
        .expect("crash batch should deserialize");

        batch
            .validate(2, 0, 10)
            .expect("Inspector crash data should satisfy the kernel contract");
    }

    #[test]
    fn event_batch_requires_strict_order_and_target_attribution() {
        let batch: BrowserControllerEventBatch = serde_json::from_value(serde_json::json!({
            "browser_generation": 2,
            "events": [{
                "event_id": 2,
                "browser_generation": 2,
                "kind": "download_progress",
                "target_id": null,
                "document_id": null,
                "data": { "guid": "download-a", "state": "completed", "received_bytes": 1, "total_bytes": 1 }
            }],
            "next_cursor": 2,
            "replay_gap": false
        }))
        .expect("event shape parses");

        assert!(batch.validate(2, 1, 10).is_err());
    }
}
