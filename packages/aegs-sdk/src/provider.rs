use std::collections::HashMap;

use chariox_event_protocol::{
    AegsAuthorizationFlow, AegsConnectionInspection, AegsProviderResourcePage,
    AegsProviderResourceQuery,
};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationCallback {
    pub connection_id: String,
    pub return_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedEvent {
    pub occurrence_id: String,
    pub event_type: String,
    pub occurred_at: String,
    pub connection_scope: String,
    pub prompt: String,
    pub metadata: Value,
}

#[derive(Debug, Clone, Copy)]
pub struct WebhookInput<'a> {
    pub headers: &'a HashMap<String, String>,
    pub body: &'a [u8],
    pub now_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookRoute {
    pub connection_id: Option<String>,
    pub scope_prefix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlWebhookResponse {
    pub content_type: String,
    pub body: String,
}

pub trait AegsProvider: Send + Sync {
    fn generator_id(&self) -> &'static str;

    fn provider_slug(&self) -> &'static str;

    fn authorization_configured(&self) -> bool {
        false
    }

    fn start_authorization(
        &self,
        _owner_id: &str,
        _return_url: Option<&str>,
    ) -> Result<AegsAuthorizationFlow, String> {
        Err("provider authorization is not configured".to_string())
    }

    fn complete_authorization(
        &self,
        _query: &HashMap<String, String>,
    ) -> Result<AuthorizationCallback, String> {
        Err("provider authorization is not configured".to_string())
    }

    fn reconnect_authorization(
        &self,
        _owner_id: &str,
        _connection_id: &str,
        _return_url: Option<&str>,
    ) -> Result<AegsAuthorizationFlow, String> {
        Err("provider reconnection is not configured".to_string())
    }

    fn query_resources(
        &self,
        _query: &AegsProviderResourceQuery,
    ) -> Result<AegsProviderResourcePage, String> {
        Err("provider authorization is not configured".to_string())
    }

    fn revoke_connection(&self, _connection_id: &str) -> Result<(), String> {
        Ok(())
    }

    /// Returns provider-specific scopes, resources, health, and recovery guidance. Returning
    /// `None` asks the SDK server to project an honest baseline from its durable connection
    /// record; production providers should return `Some` so restricted scopes and provider
    /// failures remain distinguishable.
    fn inspect_connection(
        &self,
        _connection_id: &str,
    ) -> Result<Option<AegsConnectionInspection>, String> {
        Ok(None)
    }

    /// Refreshes credentials/provider metadata before the server performs a new inspection.
    fn refresh_connection(&self, connection_id: &str) -> Result<(), String> {
        let _ = connection_id;
        self.maintain_subscriptions()
    }

    /// Builds a provider-authentic test event. The SDK publishes it through the same AEDS path
    /// as a real webhook. `None` means the integration does not yet support test events.
    fn test_event(
        &self,
        _connection_id: &str,
        _event_type: Option<&str>,
    ) -> Result<Option<NormalizedEvent>, String> {
        Ok(None)
    }

    fn maintain_subscriptions(&self) -> Result<(), String> {
        Ok(())
    }

    fn parse_webhook_route(&self, path: &str) -> Option<WebhookRoute> {
        let prefix = format!("/webhooks/{}", self.provider_slug());
        if path == prefix {
            return Some(WebhookRoute {
                connection_id: None,
                scope_prefix: None,
            });
        }
        let connection_id = path.strip_prefix(&format!("{prefix}/"))?.trim();
        if connection_id.is_empty() || connection_id.contains('/') {
            return None;
        }
        Some(WebhookRoute {
            connection_id: Some(connection_id.to_string()),
            scope_prefix: None,
        })
    }

    fn conformance_webhook_path(&self) -> String {
        format!("/webhooks/{}", self.provider_slug())
    }

    fn control_webhook(
        &self,
        _input: WebhookInput<'_>,
    ) -> Option<Result<ControlWebhookResponse, String>> {
        None
    }

    fn normalize_webhook(
        &self,
        input: WebhookInput<'_>,
        route: &WebhookRoute,
    ) -> Result<NormalizedEvent, String>;

    fn allows_direct_emit(&self) -> bool {
        false
    }
}

pub fn metadata_matches_filter(metadata: &Value, filter: &Value) -> bool {
    let Some(filter) = filter.as_object() else {
        return filter.is_null();
    };
    filter.iter().all(|(key, expected)| {
        dotted_value(metadata, key)
            .is_some_and(|actual| actual == expected || array_contains(actual, expected))
    })
}

fn dotted_value<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .try_fold(value, |current, key| current.get(key))
}

fn array_contains(actual: &Value, expected: &Value) -> bool {
    actual
        .as_array()
        .is_some_and(|values| values.iter().any(|value| value == expected))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_filters_support_dotted_metadata_and_arrays() {
        let metadata = serde_json::json!({
            "repository": {"owner": "chariox"},
            "labels": ["runtime", "event"]
        });
        assert!(metadata_matches_filter(
            &metadata,
            &serde_json::json!({
                "repository.owner": "chariox",
                "labels": "event"
            })
        ));
        assert!(!metadata_matches_filter(
            &metadata,
            &serde_json::json!({"repository.owner": "other"})
        ));
    }
}
