mod auth;
mod conformance;
mod hooks;
mod manifest;
mod oauth;
mod provider;
mod publisher;
mod server;
mod store;

pub use chariox_event_protocol::{AegsProviderActionRequest, AegsProviderActionResponse};
pub use conformance::{
    verify_provider_contract, verify_webhook_conformance, WebhookConformanceCase,
};
pub use manifest::{unsigned_manifest_digest, validate_manifest_envelope, PublishEventBuilder};
pub use oauth::{
    OAuthAuthorization, OAuthConfig, OAuthCredential, OAuthDefaults, OAuthTokenProtocol,
};
pub use provider::{
    apply_test_filter_constraints, baseline_provider_connection_inspection,
    metadata_matches_filter, select_test_subscription, AegsProvider, AuthorizationCallback,
    ControlWebhookResponse, NormalizedEvent, WebhookInput, WebhookRoute,
};
pub use publisher::AedsPublisher;
pub use server::{read_secret, run_from_environment};
pub use store::{
    AegsStore, AegsStoreMetrics, AuthorizationRecord, ConnectionRecord, CreateAuthorizationRequest,
    ProviderHookRecord, SubscriptionClaim,
};

pub const AEGS_PROTOCOL_VERSION: u32 = chariox_event_protocol::AEGS_MANAGEMENT_PROTOCOL_VERSION;
pub const MAX_WEBHOOK_BYTES: usize = 2 * 1024 * 1024;

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub use auth::{
    bearer_json, decode_page, digest, environment, filter_resources, http_empty, http_json,
    parse_base_url, parse_public_base_url, parse_url, random_opaque, sha256_occurrence_id,
    slice_resources, verify_hmac_sha256_hex, CredentialCipher, AUTHORIZATION_TTL_MS,
    PROVIDER_HTTP_TIMEOUT,
};
pub use hooks::{event_configuration_digest, reconcile_provider_hooks};
pub(crate) use store::action_request_fingerprint;
