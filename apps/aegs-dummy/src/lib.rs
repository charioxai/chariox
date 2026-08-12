use std::collections::HashMap;

use arroba_aegs_sdk::{
    AegsProvider, AuthorizationCallback, NormalizedEvent, WebhookInput, WebhookRoute,
};
use arroba_event_protocol::{
    AegsAuthorizationFlow, AegsProviderResource, AegsProviderResourcePage,
    AegsProviderResourceQuery,
};

pub const GENERATOR_ID: &str = "dev.arroba.dummy";

#[derive(Debug, Clone, Copy, Default)]
pub struct DummyProvider;

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
        let page = DummyProvider
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
}
