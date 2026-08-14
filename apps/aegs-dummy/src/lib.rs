use std::collections::HashMap;

use chariox_aegs_sdk::{
    apply_test_filter_constraints, baseline_provider_connection_inspection, now_ms,
    select_test_subscription, sha256_occurrence_id, AegsProvider, AegsStore, AuthorizationCallback,
    NormalizedEvent, WebhookInput, WebhookRoute,
};
use chariox_event_protocol::{
    AegsAuthorizationFlow, AegsConnectedResource, AegsConnectionInspection, AegsConnectionScope,
    AegsProviderResource, AegsProviderResourcePage, AegsProviderResourceQuery,
};
use chrono::{SecondsFormat, Utc};

pub const GENERATOR_ID: &str = "dev.chariox.dummy";

#[derive(Clone)]
pub struct DummyProvider {
    store: AegsStore,
}

impl DummyProvider {
    pub fn new(store: AegsStore) -> Self {
        Self { store }
    }
}

impl AegsProvider for DummyProvider {
    fn generator_id(&self) -> &'static str {
        GENERATOR_ID
    }

    fn provider_slug(&self) -> &'static str {
        "dummy"
    }

    fn authorization_configured(&self) -> bool {
        true
    }

    fn start_authorization(
        &self,
        owner_id: &str,
        _return_url: Option<&str>,
    ) -> Result<AegsAuthorizationFlow, String> {
        Ok(AegsAuthorizationFlow {
            generator_id: GENERATOR_ID.to_string(),
            status: "ready".to_string(),
            connection_id: Some(format!("local-dummy-{owner_id}")),
            authorization_url: None,
            user_code: None,
            expires_at_ms: None,
        })
    }

    fn complete_authorization(
        &self,
        _query: &HashMap<String, String>,
    ) -> Result<AuthorizationCallback, String> {
        Err("the dummy generator does not use an authorization callback".to_string())
    }

    fn reconnect_authorization(
        &self,
        owner_id: &str,
        connection_id: &str,
        _return_url: Option<&str>,
    ) -> Result<AegsAuthorizationFlow, String> {
        if connection_id != format!("local-dummy-{owner_id}") {
            return Err("the authorized connection was not found".to_string());
        }
        Ok(AegsAuthorizationFlow {
            generator_id: GENERATOR_ID.to_string(),
            status: "ready".to_string(),
            connection_id: Some(connection_id.to_string()),
            authorization_url: None,
            user_code: None,
            expires_at_ms: None,
        })
    }

    fn query_resources(
        &self,
        query: &AegsProviderResourceQuery,
    ) -> Result<AegsProviderResourcePage, String> {
        if query.cursor.is_some()
            || query.connection_id != format!("local-dummy-{}", query.owner_id)
        {
            return Err("the authorized connection was not found".to_string());
        }
        let matches = query.query.as_deref().is_none_or(|value| {
            let value = value.trim().to_ascii_lowercase();
            value.is_empty() || "default test environment".contains(&value)
        });
        Ok(AegsProviderResourcePage {
            resources: matches
                .then(|| AegsProviderResource {
                    id: "default".to_string(),
                    name: "Default test environment".to_string(),
                    kind: "test_scope".to_string(),
                    connection_scope: "default".to_string(),
                })
                .into_iter()
                .collect(),
            next_cursor: None,
        })
    }

    fn inspect_connection(
        &self,
        connection_id: &str,
    ) -> Result<Option<AegsConnectionInspection>, String> {
        let connection = self
            .store
            .connection(connection_id)?
            .ok_or_else(|| "the dummy connection was not found".to_string())?;
        let mut inspection =
            baseline_provider_connection_inspection(GENERATOR_ID, &connection, true);
        inspection.scopes = vec![AegsConnectionScope {
            id: "local:test".to_string(),
            label: "Local test events".to_string(),
            granted: connection.status == "ready",
            required: true,
        }];
        inspection.resources = vec![AegsConnectedResource {
            id: "default".to_string(),
            name: "Default test environment".to_string(),
            kind: "test_scope".to_string(),
        }];
        if connection.status == "ready" {
            let checked_at_ms = now_ms();
            self.store
                .mark_connection_health(connection_id, checked_at_ms)?;
            inspection.last_successful_health_check_at_ms = Some(checked_at_ms);
        }
        Ok(Some(inspection))
    }

    fn test_event(
        &self,
        connection_id: &str,
        event_type: Option<&str>,
    ) -> Result<Option<NormalizedEvent>, String> {
        const SUPPORTED: &[&str] = &["dummy.triggered"];
        let subscription =
            select_test_subscription(&self.store, connection_id, event_type, SUPPORTED)?;
        let connection_scope = subscription
            .as_ref()
            .map(|value| value.connection_scope.clone())
            .unwrap_or_else(|| "default".to_string());
        let mut metadata = serde_json::json!({
            "source": "dummy",
            "scope": connection_scope,
            "chariox": {"test_event": true}
        });
        if let Some(subscription) = &subscription {
            apply_test_filter_constraints(&mut metadata, &subscription.filter);
        }
        Ok(Some(NormalizedEvent {
            occurrence_id: sha256_occurrence_id(
                format!("{connection_id}:dummy.triggered:{}", now_ms()).as_bytes(),
            ),
            event_type: "dummy.triggered".to_string(),
            occurred_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            connection_scope,
            prompt: "Handle a Chariox dummy test notification.".to_string(),
            metadata,
            reply_context: None,
        }))
    }

    fn normalize_webhook(
        &self,
        _input: WebhookInput<'_>,
        _route: &WebhookRoute,
    ) -> Result<NormalizedEvent, String> {
        Err("dummy events use /v1/emit".to_string())
    }

    fn allows_direct_emit(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dummy_provider_exposes_only_the_local_test_scope() {
        let page = DummyProvider::new(AegsStore::open(":memory:").unwrap())
            .query_resources(&AegsProviderResourceQuery {
                generator_id: GENERATOR_ID.to_string(),
                owner_id: "owner-local-user".to_string(),
                connection_id: "local-dummy-owner-local-user".to_string(),
                query: None,
                cursor: None,
                limit: 20,
            })
            .unwrap();
        assert_eq!(page.resources.len(), 1);
        assert_eq!(page.resources[0].connection_scope, "default");
    }

    #[test]
    fn dummy_test_event_uses_active_trigger_filter() {
        let store = AegsStore::open(":memory:").unwrap();
        store
            .upsert_ready_connection(
                "local-dummy-owner-local-user",
                "owner-local-user",
                "dummy",
                &serde_json::json!({}),
                1,
            )
            .unwrap();
        store
            .reconcile(
                "owner-local-user",
                GENERATOR_ID,
                &[chariox_aegs_sdk::SubscriptionClaim {
                    binding_id: "binding-1".to_string(),
                    generator_id: GENERATOR_ID.to_string(),
                    connection_id: "local-dummy-owner-local-user".to_string(),
                    connection_scope: "default".to_string(),
                    event_interest_key: "sha256:test".to_string(),
                    event_type: "dummy.triggered".to_string(),
                    event_type_version: 1,
                    filter: serde_json::json!({"scenario.name": "smoke"}),
                    revision: 1,
                    active: true,
                }],
            )
            .unwrap();
        let provider = DummyProvider::new(store);
        let event = provider
            .test_event("local-dummy-owner-local-user", None)
            .unwrap()
            .unwrap();
        assert_eq!(event.connection_scope, "default");
        assert!(chariox_aegs_sdk::metadata_matches_filter(
            &event.metadata,
            &serde_json::json!({"scenario.name": "smoke"})
        ));
    }
}
