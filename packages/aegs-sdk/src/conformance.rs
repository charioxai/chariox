use std::collections::HashMap;

use crate::{AegsProvider, NormalizedEvent, WebhookInput};
use arroba_event_protocol::validate_utc_timestamp;

#[derive(Debug, Clone)]
pub struct WebhookConformanceCase {
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    pub now_ms: u64,
    pub expected_event_type: String,
    pub expected_connection_scope: String,
}

pub fn verify_provider_contract(provider: &dyn AegsProvider) -> Result<(), String> {
    let generator_id = provider.generator_id();
    if generator_id.len() > 128
        || !generator_id.contains('.')
        || generator_id.bytes().any(|byte| {
            !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'.' || byte == b'-')
        })
    {
        return Err("generator_id must be a bounded lowercase publisher-scoped identifier".into());
    }
    let slug = provider.provider_slug();
    if slug.is_empty()
        || slug.len() > 64
        || slug
            .bytes()
            .any(|byte| !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'))
    {
        return Err("provider_slug must be a bounded lowercase URL segment".into());
    }
    provider
        .parse_webhook_route(&provider.conformance_webhook_path())
        .ok_or_else(|| "provider must expose its declared conformance webhook route".to_string())?;
    if provider
        .parse_webhook_route("/webhooks/not-this-provider")
        .is_some()
    {
        return Err("provider accepted another provider's webhook route".into());
    }
    Ok(())
}

pub fn verify_webhook_conformance(
    provider: &dyn AegsProvider,
    case: &WebhookConformanceCase,
) -> Result<NormalizedEvent, String> {
    verify_provider_contract(provider)?;
    let route = provider
        .parse_webhook_route(&case.path)
        .ok_or_else(|| "fixture path is not accepted by the provider".to_string())?;
    let input = WebhookInput {
        headers: &case.headers,
        body: &case.body,
        now_ms: case.now_ms,
    };
    let first = provider.normalize_webhook(input, &route)?;
    let second = provider.normalize_webhook(input, &route)?;
    if first != second {
        return Err("normalization is not deterministic for the same provider occurrence".into());
    }
    if first.occurrence_id.trim().is_empty() || first.occurrence_id.len() > 256 {
        return Err("occurrence_id must be non-empty and bounded".into());
    }
    if first.event_type != case.expected_event_type {
        return Err(format!(
            "normalized event type {} does not match {}",
            first.event_type, case.expected_event_type
        ));
    }
    if first.connection_scope != case.expected_connection_scope {
        return Err(format!(
            "normalized connection scope {} does not match {}",
            first.connection_scope, case.expected_connection_scope
        ));
    }
    if first.prompt.trim().is_empty() {
        return Err("normalized prompt must not be empty".into());
    }
    if !first.metadata.is_object() {
        return Err("normalized metadata must be an object".into());
    }
    validate_utc_timestamp("occurred_at", &first.occurred_at)?;
    Ok(first)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{WebhookInput, WebhookRoute};

    struct InvalidProvider;

    impl AegsProvider for InvalidProvider {
        fn generator_id(&self) -> &'static str {
            "GitHub"
        }

        fn provider_slug(&self) -> &'static str {
            "github"
        }

        fn normalize_webhook(
            &self,
            _input: WebhookInput<'_>,
            _route: &WebhookRoute,
        ) -> Result<NormalizedEvent, String> {
            unreachable!()
        }
    }

    #[test]
    fn rejects_unscoped_generator_identity() {
        assert!(verify_provider_contract(&InvalidProvider).is_err());
    }
}
