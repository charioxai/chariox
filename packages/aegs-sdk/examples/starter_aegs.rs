//! Minimal provider skeleton. Copy this file into a new AEGS repository and
//! replace the normalization and authorization methods with provider logic.
//!
//! The SDK intentionally keeps credentials and provider API calls inside the
//! AEGS process. The kernel only sees canonical events and scoped actions.

use std::collections::HashMap;

use chariox_aegs_sdk::{AegsProvider, NormalizedEvent, WebhookInput, WebhookRoute};
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
    let _ = provider;
    let _headers = HashMap::<String, String>::new();
    println!(
        "Implement authorization, signature verification, and webhook routing before deployment."
    );
}
