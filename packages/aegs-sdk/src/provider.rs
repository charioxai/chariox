use std::collections::HashMap;

use chariox_event_protocol::{
    AegsAuthorizationFlow, AegsConnectionInspection, AegsConnectionLifecycleState,
    AegsProviderActionRequest, AegsProviderActionResponse, AegsProviderResourcePage,
    AegsProviderResourceQuery,
};
use serde_json::Value;

use crate::{AegsStore, ConnectionRecord, SubscriptionClaim};

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
    pub reply_context: Option<Value>,
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

    /// Executes a provider-owned outbound action using the credentials held by this AEGS.
    /// Providers should validate the action name and input, and return a stable provider result.
    fn perform_action(
        &self,
        _request: &AegsProviderActionRequest,
    ) -> Result<AegsProviderActionResponse, String> {
        Err("provider outbound actions are not supported".to_string())
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

/// Projects durable connection state without claiming provider health that was not observed.
/// Production providers can enrich this baseline with scopes, resources, recovery guidance,
/// and a fresh health timestamp.
pub fn baseline_provider_connection_inspection(
    generator_id: &str,
    connection: &ConnectionRecord,
    test_event_supported: bool,
) -> AegsConnectionInspection {
    let lifecycle_state = match connection.status.as_str() {
        "pending" => AegsConnectionLifecycleState::AuthorizationRequired,
        "ready" => AegsConnectionLifecycleState::Connected,
        "expired" => AegsConnectionLifecycleState::ReauthorizationRequired,
        "revoked" => AegsConnectionLifecycleState::Disconnected,
        "unavailable" => AegsConnectionLifecycleState::ProviderUnreachable,
        _ => AegsConnectionLifecycleState::Degraded,
    };
    AegsConnectionInspection {
        generator_id: generator_id.to_string(),
        connection_id: connection.connection_id.clone(),
        lifecycle_state,
        scopes: Vec::new(),
        resources: Vec::new(),
        last_successful_health_check_at_ms: connection.last_successful_health_check_at_ms,
        last_accepted_event_at_ms: connection.last_accepted_event_at_ms,
        problem_code: None,
        problem_message: None,
        recovery_action: None,
        test_event_supported: test_event_supported && connection.status == "ready",
    }
}

/// Chooses the active workflow trigger context for a provider-authentic test event.
pub fn select_test_subscription(
    store: &AegsStore,
    connection_id: &str,
    requested_event_type: Option<&str>,
    supported_event_types: &[&str],
) -> Result<Option<SubscriptionClaim>, String> {
    if requested_event_type.is_some_and(|value| !supported_event_types.contains(&value)) {
        return Err("the requested event type is not supported".to_string());
    }
    Ok(store
        .active_subscriptions_for_connection(connection_id)?
        .into_iter()
        .find(|subscription| {
            requested_event_type
                .map(|requested| subscription.event_type == requested)
                .unwrap_or(true)
        }))
}

/// Applies a trigger's declared filter values to synthetic provider metadata, including dotted
/// object paths, so the authentic test event traverses ordinary filter matching.
pub fn apply_test_filter_constraints(metadata: &mut Value, filter: &Value) {
    let Some(filter) = filter.as_object() else {
        return;
    };
    for (path, expected) in filter {
        // A real provider event carries one scalar resource identifier. For an
        // any-of binding, use a deterministic representative candidate so the
        // synthetic event has the same shape as production while still
        // traversing the ordinary filter matcher. An empty any-of filter must
        // remain non-matching rather than fabricating a resource.
        let materialized = expected
            .as_array()
            .map(|values| values.first().cloned().unwrap_or(Value::Null))
            .unwrap_or_else(|| expected.clone());
        set_dotted_value(metadata, path, materialized);
    }
}

fn set_dotted_value(target: &mut Value, path: &str, value: Value) {
    let parts = path
        .split('.')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return;
    }
    let mut current = target;
    for part in &parts[..parts.len() - 1] {
        if !current.is_object() {
            *current = Value::Object(Default::default());
        }
        current = current
            .as_object_mut()
            .expect("object was created above")
            .entry((*part).to_string())
            .or_insert_with(|| Value::Object(Default::default()));
    }
    if !current.is_object() {
        *current = Value::Object(Default::default());
    }
    current
        .as_object_mut()
        .expect("object was created above")
        .insert(parts[parts.len() - 1].to_string(), value);
}

pub fn metadata_matches_filter(metadata: &Value, filter: &Value) -> bool {
    let Some(filter) = filter.as_object() else {
        return filter.is_null();
    };
    filter.iter().all(|(key, expected)| {
        dotted_value(metadata, key).is_some_and(|actual| filter_value_matches(actual, expected))
    })
}

/// Match either a scalar filter value or an explicit any-of array. Provider
/// filters use arrays for one binding that intentionally selects several
/// resources (for example, several Slack channels), while preserving the
/// existing scalar and metadata-array behavior.
fn filter_value_matches(actual: &Value, expected: &Value) -> bool {
    if let Some(expected_values) = expected.as_array() {
        return expected_values
            .iter()
            .any(|candidate| filter_value_matches(actual, candidate));
    }
    actual == expected || array_contains(actual, expected)
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
        assert!(metadata_matches_filter(
            &serde_json::json!({"event": {"channel": "C123"}}),
            &serde_json::json!({"event.channel": ["C999", "C123"]})
        ));
        assert!(!metadata_matches_filter(
            &serde_json::json!({"event": {"channel": "C123"}}),
            &serde_json::json!({"event.channel": ["C999", "C456"]})
        ));
    }

    #[test]
    fn test_filter_constraints_materialize_dotted_paths() {
        let mut metadata = serde_json::json!({"repository": {"name": "chariox"}});
        apply_test_filter_constraints(
            &mut metadata,
            &serde_json::json!({"repository.owner.login": "charioxai"}),
        );
        assert_eq!(metadata["repository"]["name"], "chariox");
        assert_eq!(metadata["repository"]["owner"]["login"], "charioxai");
    }

    #[test]
    fn test_filter_constraints_materialize_one_scalar_any_of_candidate() {
        let mut metadata = serde_json::json!({});
        apply_test_filter_constraints(
            &mut metadata,
            &serde_json::json!({"event.channel": ["C123", "C456"]}),
        );
        assert_eq!(metadata["event"]["channel"], "C123");
        assert!(metadata_matches_filter(
            &metadata,
            &serde_json::json!({"event.channel": ["C123", "C456"]})
        ));

        let mut empty = serde_json::json!({});
        apply_test_filter_constraints(&mut empty, &serde_json::json!({"event.channel": []}));
        assert_eq!(empty["event"]["channel"], Value::Null);
        assert!(!metadata_matches_filter(
            &empty,
            &serde_json::json!({"event.channel": []})
        ));
    }
}
