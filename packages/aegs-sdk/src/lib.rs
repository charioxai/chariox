mod auth;
mod conformance;
mod hooks;
mod manifest;
mod oauth;
mod provider;
mod publisher;
mod server;
mod store;

pub use conformance::{
    verify_provider_contract, verify_webhook_conformance, WebhookConformanceCase,
};
pub use manifest::{unsigned_manifest_digest, validate_manifest_envelope, PublishEventBuilder};
pub use oauth::{
    OAuthAuthorization, OAuthConfig, OAuthCredential, OAuthDefaults, OAuthTokenProtocol,
};
pub use provider::{
    metadata_matches_filter, AegsProvider, AuthorizationCallback, ControlWebhookResponse,
    NormalizedEvent, WebhookInput, WebhookRoute,
};
pub use publisher::AedsPublisher;
pub use server::{read_secret, run_from_environment};
pub use store::{
    AegsStore, AegsStoreMetrics, AuthorizationRecord, ConnectionRecord, ProviderHookRecord,
    SubscriptionClaim,
};

pub const AEGS_PROTOCOL_VERSION: u32 = 1;
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
