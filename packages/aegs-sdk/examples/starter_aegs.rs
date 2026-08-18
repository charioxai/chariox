//! Minimal provider skeleton. Copy this file into a new AEGS repository and
//! replace the normalization and authorization methods with provider logic.
//!
//! The SDK intentionally keeps credentials and provider API calls inside the
//! AEGS process. The kernel only sees canonical events and scoped actions.

use std::collections::HashMap;

use chariox_aegs_sdk::{
    verify_webhook_conformance, AegsProvider, NormalizedEvent, WebhookConformanceCase,
    WebhookInput, WebhookRoute,
};
use serde_json::json;

struct StarterProvider;

impl AegsProvider for StarterProvider {
    fn generator_id(&self) -> &'static str {
        "com.example.starter"
    }

    fn provider_slug(&self) -> &'static str {
        "starter"
    }

    fn normalize_webhook(
        &self,
        input: WebhookInput<'_>,
        route: &WebhookRoute,
    ) -> Result<NormalizedEvent, String> {
        let payload: serde_json::Value = serde_json::from_slice(input.body)
            .map_err(|error| format!("provider payload is invalid JSON: {error}"))?;
        let connection_scope = route
            .connection_id
            .as_deref()
            .ok_or_else(|| "the webhook route must identify a connection".to_string())?;
        let occurrence_id = payload
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "provider payload is missing id".to_string())?;
        Ok(NormalizedEvent {
            occurrence_id: occurrence_id.to_string(),
            event_type: "example.created".to_string(),
            occurred_at: "2026-01-01T00:00:00.000Z".to_string(),
            connection_scope: connection_scope.to_string(),
            prompt: "Handle the provider event.".to_string(),
            metadata: json!({"provider_payload": payload}),
            reply_context: None,
        })
    }
}

fn main() {
    let provider = StarterProvider;
    let mut headers = HashMap::<String, String>::new();
    headers.insert("content-type".to_string(), "application/json".to_string());
    let case = WebhookConformanceCase {
        path: "/webhooks/starter/demo-connection".to_string(),
        headers,
        body: br#"{"id":"demo-occurrence"}"#.to_vec(),
        now_ms: 1_735_689_600_000,
        expected_event_type: "example.created".to_string(),
        expected_connection_scope: "demo-connection".to_string(),
    };

    let event = verify_webhook_conformance(&provider, &case)
        .expect("starter provider must pass the public webhook contract");
    println!(
        "starter AEGS conformance passed: {} ({})",
        event.event_type, event.occurrence_id
    );
    println!(
        "Next steps: replace the fixture normalization with provider signature verification,
authorization, and webhook routing before deployment."
    );
}
