use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Weak};
use std::time::Duration;

use bytes::Bytes;
use chariox_event_protocol::{
    AegsAuthorizationStartRequest, AegsConnectionInspection, AegsConnectionInspectionRequest,
    AegsConnectionPage, AegsConnectionQuery, AegsConnectionReconnectRequest,
    AegsConnectionRefreshRequest, AegsConnectionRevokeRequest, AegsConnectionRevokeResponse,
    AegsConnectionStatus, AegsConnectionSummary, AegsConnectionTestEventRequest,
    AegsConnectionTestEventResponse, AegsProviderActionRequest, AegsProviderResourceQuery,
    PublishEventRequest,
};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

use crate::management_capability::{parse_public_key, verify_management_capability_scoped};
use crate::store::ActionReceiptLookupError;
use crate::{
    baseline_provider_connection_inspection, metadata_matches_filter, now_ms, AedsPublisher,
    AegsProvider, AegsStore, ControlWebhookResponse, WebhookInput, AEGS_PROTOCOL_VERSION,
    MAX_WEBHOOK_BYTES,
};

fn action_receipt_is_durable(action_id: &str) -> bool {
    // Context lookups are bounded, read-only queries. Persisting their full provider response
    // would retain conversation history and user profiles indefinitely in the AEGS database.
    !matches!(action_id, "event.context" | "notification.context")
}

#[derive(Clone)]
struct AegsServer {
    producer_id: String,
    management_token: Option<String>,
    management_public_key: Option<ed25519_dalek::VerifyingKey>,
    management_issuer: String,
    management_url: Option<String>,
    manifest_digest: Option<String>,
    publisher: AedsPublisher,
    store: AegsStore,
    provider: Arc<dyn AegsProvider>,
    action_locks: ActionLockMap,
}

enum ManagementAuthorization {
    Static,
    Signed(crate::management_capability::ManagementCapabilityClaims),
}

type ActionLock = Arc<tokio::sync::Mutex<()>>;
type ActionLockMap = Arc<std::sync::Mutex<HashMap<String, ActionLock>>>;

struct ActionLockLease {
    key: String,
    action_lock: Weak<tokio::sync::Mutex<()>>,
    locks: ActionLockMap,
    guard: Option<tokio::sync::OwnedMutexGuard<()>>,
}

impl Drop for ActionLockLease {
    fn drop(&mut self) {
        // Release the owned guard before checking the reference count. If a
        // retry arrived while this action was running, its Arc keeps the map
        // entry alive and the waiter will remove it when it finishes.
        let _ = self.guard.take();
        let Ok(mut locks) = self.locks.lock() else {
            return;
        };
        let remove = locks.get(&self.key).is_some_and(|entry| {
            Weak::ptr_eq(&Arc::downgrade(entry), &self.action_lock) && Arc::strong_count(entry) == 2
        });
        if remove {
            locks.remove(&self.key);
        }
    }
}

pub async fn run_from_environment<F>(provider_factory: F) -> Result<(), Box<dyn std::error::Error>>
where
    F: FnOnce(AegsStore) -> Result<Arc<dyn AegsProvider>, String>,
{
    let address: SocketAddr = std::env::var("CHARIOX_AEGS_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:43132".to_string())
        .parse()?;
    let store = AegsStore::open(
        std::env::var("CHARIOX_AEGS_DATABASE_PATH").unwrap_or_else(|_| "aegs.db".to_string()),
    )?;
    let provider = provider_factory(store.clone())?;
    let producer_id = std::env::var("CHARIOX_AEGS_PRODUCER_ID")
        .unwrap_or_else(|_| provider.generator_id().to_string());
    if producer_id != provider.generator_id() {
        return Err(format!(
            "producer ID `{producer_id}` does not match provider generator `{}`",
            provider.generator_id()
        )
        .into());
    }
    let management_url = std::env::var("CHARIOX_AEGS_MANAGEMENT_URL")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty());
    let manifest_digest = std::env::var("CHARIOX_AEGS_MANIFEST_DIGEST")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let management_public_key = std::env::var("CHARIOX_AEGS_MANAGEMENT_PUBLIC_KEY")
        .ok()
        .map(|value| parse_public_key(&value))
        .transpose()
        .map_err(|error| format!("invalid CHARIOX_AEGS_MANAGEMENT_PUBLIC_KEY: {error}"))?;
    if management_public_key.is_some() && (management_url.is_none() || manifest_digest.is_none()) {
        return Err(
            "signed management capabilities require CHARIOX_AEGS_MANAGEMENT_URL and CHARIOX_AEGS_MANIFEST_DIGEST"
                .into(),
        );
    }
    let server = AegsServer {
        producer_id: producer_id.clone(),
        management_token: read_secret(
            "CHARIOX_AEGS_MANAGEMENT_TOKEN",
            "CHARIOX_AEGS_MANAGEMENT_TOKEN_FILE",
        )?,
        management_public_key,
        management_issuer: std::env::var("CHARIOX_AEGS_MANAGEMENT_ISSUER")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "chariox-cloud".to_string()),
        management_url,
        manifest_digest,
        publisher: AedsPublisher::new(
            producer_id.clone(),
            read_secret(
                "CHARIOX_AEGS_PRODUCER_TOKEN",
                "CHARIOX_AEGS_PRODUCER_TOKEN_FILE",
            )?,
            std::env::var("CHARIOX_AEDS_EVENTS_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:43131/v1/events".to_string()),
        ),
        store,
        provider,
        action_locks: Arc::new(std::sync::Mutex::new(HashMap::new())),
    };
    if server.provider.authorization_configured() {
        spawn_subscription_maintenance(Arc::clone(&server.provider));
    }
    let listener = TcpListener::bind(address).await?;
    eprintln!(
        "{}",
        serde_json::json!({
            "component": "chariox-aegs",
            "event": "starting",
            "address": address,
            "producer_id": server.producer_id,
            "provider": server.provider.provider_slug(),
            "protocol_version": AEGS_PROTOCOL_VERSION,
        })
    );
    loop {
        let (stream, _) = listener.accept().await?;
        let server = server.clone();
        tokio::spawn(async move {
            let service = service_fn(move |request| {
                let server = server.clone();
                async move { Ok::<_, Infallible>(server.handle(request).await) }
            });
            let _ = http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
    }
}

fn spawn_subscription_maintenance(provider: Arc<dyn AegsProvider>) {
    let maintenance_interval = std::env::var("CHARIOX_AEGS_MAINTENANCE_INTERVAL_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .unwrap_or(6 * 60 * 60);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(maintenance_interval));
        loop {
            interval.tick().await;
            let provider = Arc::clone(&provider);
            match tokio::task::spawn_blocking(move || provider.maintain_subscriptions()).await {
                Ok(Ok(())) => {}
                Ok(Err(message)) => eprintln!(
                    "{}",
                    serde_json::json!({
                        "component": "chariox-aegs",
                        "event": "subscription_maintenance_failed",
                        "error": message,
                    })
                ),
                Err(error) => eprintln!(
                    "{}",
                    serde_json::json!({
                        "component": "chariox-aegs",
                        "event": "subscription_maintenance_task_failed",
                        "error": error.to_string(),
                    })
                ),
            }
        }
    });
}

impl AegsServer {
    async fn handle(&self, request: Request<Incoming>) -> Response<Full<Bytes>> {
        let method = request.method().clone();
        let path = request.uri().path().to_string();
        match (method, path.as_str()) {
            (Method::GET, "/healthz") => json(StatusCode::OK, serde_json::json!({"status": "ok"})),
            (Method::GET, "/readyz") => match self.store.all(&self.producer_id) {
                Ok(_) => json(StatusCode::OK, serde_json::json!({"status": "ready"})),
                Err(message) => error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "store_unavailable",
                    message,
                ),
            },
            (Method::GET, "/version") => json(
                StatusCode::OK,
                serde_json::json!({
                    "component": "chariox-aegs",
                    "provider": self.producer_id,
                    "protocol_version": AEGS_PROTOCOL_VERSION,
                    "build_revision": option_env!("CHARIOX_BUILD_REVISION").unwrap_or("development"),
                    "authorization_configured": self.provider.authorization_configured(),
                }),
            ),
            (Method::GET, "/metrics") => match self.store.metrics() {
                Ok(metrics) => text(
                    StatusCode::OK,
                    format!(
                        "chariox_aegs_active_subscriptions {}\n\
                         chariox_aegs_subscriptions {}\n\
                         chariox_aegs_connections {}\n\
                         chariox_aegs_provider_hooks {}\n",
                        metrics.active_subscriptions,
                        metrics.subscriptions,
                        metrics.connections,
                        metrics.provider_hooks,
                    ),
                    "text/plain; version=0.0.4; charset=utf-8",
                ),
                Err(message) => error(StatusCode::INTERNAL_SERVER_ERROR, "store_failed", message),
            },
            (Method::GET, "/v1/subscriptions") => {
                let authorization = match self.authorize_management(&request) {
                    Ok(authorization) => authorization,
                    Err(response) => return *response,
                };
                let subscriptions = match management_owner(&request, &authorization) {
                    Some(owner_id) => self.store.all_for_owner(&self.producer_id, &owner_id),
                    None => self.store.all(&self.producer_id),
                };
                match subscriptions {
                    Ok(subscriptions) => json(
                        StatusCode::OK,
                        serde_json::json!({"subscriptions": subscriptions}),
                    ),
                    Err(message) => {
                        error(StatusCode::INTERNAL_SERVER_ERROR, "store_failed", message)
                    }
                }
            }
            (Method::PUT, "/v1/subscriptions/reconcile") => {
                let authorization = match self.authorize_management(&request) {
                    Ok(authorization) => authorization,
                    Err(response) => return *response,
                };
                let body = match read_body(request).await {
                    Ok(body) => body,
                    Err(response) => return response,
                };
                let reconcile: chariox_event_protocol::AegsSubscriptionReconcileRequest =
                    match serde_json::from_slice(&body) {
                        Ok(value) => value,
                        Err(error_value) => {
                            return error(
                                StatusCode::BAD_REQUEST,
                                "invalid_json",
                                error_value.to_string(),
                            )
                        }
                    };
                if let Err(response) = enforce_management_owner(&authorization, &reconcile.owner_id)
                {
                    return response;
                }
                if reconcile.generator_id != self.producer_id {
                    return error(
                        StatusCode::CONFLICT,
                        "generator_mismatch",
                        format!(
                            "this AEGS owns {}, not {}",
                            self.producer_id, reconcile.generator_id
                        ),
                    );
                }
                let allowed_connection_owners = match &authorization {
                    ManagementAuthorization::Signed(claims) => Some(claims.owner_ids.as_slice()),
                    ManagementAuthorization::Static => None,
                };
                match self.store.reconcile_scoped(
                    &reconcile.owner_id,
                    allowed_connection_owners,
                    &reconcile.generator_id,
                    &reconcile.subscriptions,
                ) {
                    Ok(accepted_binding_ids) => {
                        let provider = Arc::clone(&self.provider);
                        match tokio::task::spawn_blocking(move || {
                            provider.maintain_subscriptions()
                        })
                        .await
                        {
                            Ok(Ok(())) => {}
                            Ok(Err(message)) => {
                                return error(
                                    StatusCode::BAD_GATEWAY,
                                    "provider_subscription_reconcile_failed",
                                    format!(
                                        "desired subscriptions were saved and will be retried: {message}"
                                    ),
                                )
                            }
                            Err(message) => {
                                return error(
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    "provider_subscription_reconcile_failed",
                                    format!(
                                        "desired subscriptions were saved and will be retried: {message}"
                                    ),
                                )
                            }
                        }
                        json(
                            StatusCode::OK,
                            serde_json::json!({
                                "accepted_binding_ids": accepted_binding_ids,
                                "authoritative": true,
                            }),
                        )
                    }
                    Err(message) => error(StatusCode::BAD_REQUEST, "invalid_subscription", message),
                }
            }
            (Method::POST, "/v1/actions") => {
                let authorization = match self.authorize_management(&request) {
                    Ok(authorization) => authorization,
                    Err(response) => return *response,
                };
                let body = match read_body(request).await {
                    Ok(body) => body,
                    Err(response) => return response,
                };
                let action: AegsProviderActionRequest = match serde_json::from_slice(&body) {
                    Ok(value) => value,
                    Err(error_value) => {
                        return error(
                            StatusCode::BAD_REQUEST,
                            "invalid_json",
                            error_value.to_string(),
                        )
                    }
                };
                if let Err(response) = enforce_management_owner(&authorization, &action.owner_id) {
                    return response;
                }
                if let Err(message) = action.validate() {
                    return error(StatusCode::BAD_REQUEST, "invalid_action", message);
                }
                if action.generator_id != self.producer_id {
                    return error(
                        StatusCode::CONFLICT,
                        "generator_mismatch",
                        format!(
                            "this AEGS owns {}, not {}",
                            self.producer_id, action.generator_id
                        ),
                    );
                }
                let connection = match self
                    .store
                    .claim_connection_owner(&action.connection_id, &action.owner_id)
                {
                    Ok(connection) => connection,
                    Err(message) => {
                        return error(StatusCode::NOT_FOUND, "connection_not_found", message)
                    }
                };
                if connection.status != "ready" {
                    return error(
                        StatusCode::CONFLICT,
                        "connection_not_ready",
                        "the provider connection is not ready for outbound actions",
                    );
                }
                // Serialize each idempotency key across concurrent HTTP retries. The durable
                // receipt makes retries safe after completion; this lock also prevents two
                // simultaneous retries from both reaching the provider before that receipt is
                // written.
                let action_lock_key = format!(
                    "{}\u{1f}{}\u{1f}{}",
                    action.owner_id, action.connection_id, action.idempotency_key
                );
                let action_lock = match self.action_locks.lock() {
                    Ok(mut locks) => locks
                        .entry(action_lock_key.clone())
                        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                        .clone(),
                    Err(_) => {
                        return error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "action_lock_failed",
                            "AEGS action lock was poisoned",
                        )
                    }
                };
                let _action_lock_lease = ActionLockLease {
                    key: action_lock_key,
                    action_lock: Arc::downgrade(&action_lock),
                    locks: Arc::clone(&self.action_locks),
                    guard: Some(Arc::clone(&action_lock).lock_owned().await),
                };
                let request_fingerprint = crate::action_request_fingerprint(
                    &action.action_id,
                    &action.input,
                    &action.context,
                );
                let durable_receipt = action_receipt_is_durable(&action.action_id);
                if durable_receipt {
                    match self.store.action_receipt(
                        &action.owner_id,
                        &action.connection_id,
                        &action.idempotency_key,
                        &request_fingerprint,
                    ) {
                        Ok(Some(response)) => return json(StatusCode::OK, response),
                        Ok(None) => {}
                        Err(ActionReceiptLookupError::Conflict(message)) => {
                            return error(StatusCode::CONFLICT, "idempotency_key_conflict", message)
                        }
                        Err(ActionReceiptLookupError::Storage(message)) => {
                            return error(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "action_receipt_lookup_failed",
                                message,
                            )
                        }
                    }
                }
                let provider = Arc::clone(&self.provider);
                let action_for_provider = action.clone();
                let response = match tokio::task::spawn_blocking(move || {
                    provider.perform_action(&action_for_provider)
                })
                .await
                {
                    Ok(Ok(response)) => response,
                    Ok(Err(message)) => {
                        return error(StatusCode::BAD_GATEWAY, "provider_action_failed", message)
                    }
                    Err(message) => {
                        return error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "provider_action_failed",
                            message.to_string(),
                        )
                    }
                };
                if response.action_id != action.action_id
                    || response.idempotency_key != action.idempotency_key
                {
                    return error(
                        StatusCode::BAD_GATEWAY,
                        "invalid_provider_action_response",
                        "provider action response identity did not match the request",
                    );
                }
                if durable_receipt {
                    if let Err(message) = self.store.record_action_receipt(
                        &action.owner_id,
                        &action.connection_id,
                        &response,
                        &request_fingerprint,
                        now_ms(),
                    ) {
                        return error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "action_receipt_failed",
                            message,
                        );
                    }
                }
                json(StatusCode::ACCEPTED, response)
            }
            (Method::POST, "/v1/authorizations") => {
                let authorization_guard = match self.authorize_management(&request) {
                    Ok(authorization) => authorization,
                    Err(response) => return *response,
                };
                let body = match read_body(request).await {
                    Ok(body) => body,
                    Err(response) => return response,
                };
                let authorization: AegsAuthorizationStartRequest =
                    match serde_json::from_slice(&body) {
                        Ok(value) => value,
                        Err(error_value) => {
                            return error(
                                StatusCode::BAD_REQUEST,
                                "invalid_json",
                                error_value.to_string(),
                            )
                        }
                    };
                if let Err(response) =
                    enforce_management_owner(&authorization_guard, &authorization.owner_id)
                {
                    return response;
                }
                if let Err(message) = authorization.validate() {
                    return error(StatusCode::BAD_REQUEST, "invalid_authorization", message);
                }
                if authorization.generator_id != self.producer_id {
                    return error(
                        StatusCode::CONFLICT,
                        "generator_mismatch",
                        format!(
                            "this AEGS owns {}, not {}",
                            self.producer_id, authorization.generator_id
                        ),
                    );
                }
                if !self.provider.authorization_configured() {
                    return error(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "authorization_unavailable",
                        "provider authorization credentials are not configured for this AEGS",
                    );
                }
                match self.provider.start_authorization(
                    &authorization.owner_id,
                    authorization.return_url.as_deref(),
                ) {
                    Ok(flow) => {
                        if flow.status == "ready" {
                            if let Some(connection_id) = flow.connection_id.as_deref() {
                                if let Err(message) = self.store.upsert_ready_connection(
                                    connection_id,
                                    &authorization.owner_id,
                                    self.provider.provider_slug(),
                                    &serde_json::json!({}),
                                    now_ms(),
                                ) {
                                    return error(
                                        StatusCode::INTERNAL_SERVER_ERROR,
                                        "connection_store_failed",
                                        message,
                                    );
                                }
                            }
                        }
                        json(StatusCode::OK, serde_json::json!(flow))
                    }
                    Err(message) => error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "authorization_start_failed",
                        message,
                    ),
                }
            }
            (Method::POST, "/v1/connections/query") => {
                let authorization_guard = match self.authorize_management(&request) {
                    Ok(authorization) => authorization,
                    Err(response) => return *response,
                };
                let body = match read_body(request).await {
                    Ok(body) => body,
                    Err(response) => return response,
                };
                let query: AegsConnectionQuery = match serde_json::from_slice(&body) {
                    Ok(value) => value,
                    Err(error_value) => {
                        return error(
                            StatusCode::BAD_REQUEST,
                            "invalid_json",
                            error_value.to_string(),
                        )
                    }
                };
                if let Err(response) =
                    enforce_management_owner(&authorization_guard, &query.owner_id)
                {
                    return response;
                }
                if let Err(message) = query.validate() {
                    return error(StatusCode::BAD_REQUEST, "invalid_connection_query", message);
                }
                if query.generator_id != self.producer_id {
                    return error(
                        StatusCode::CONFLICT,
                        "generator_mismatch",
                        format!(
                            "this AEGS owns {}, not {}",
                            self.producer_id, query.generator_id
                        ),
                    );
                }
                let page_number = match crate::decode_page(query.cursor.as_deref()) {
                    Ok(value) => value,
                    Err(message) => {
                        return error(
                            StatusCode::BAD_REQUEST,
                            "invalid_connection_cursor",
                            message,
                        )
                    }
                };
                let connections = if let Some(connection_id) = query.connection_id.as_deref() {
                    match self
                        .store
                        .claim_connection_owner(connection_id, &query.owner_id)
                    {
                        Ok(connection) => vec![connection],
                        Err(_) => Vec::new(),
                    }
                } else {
                    match self.store.connections_for_owner(&query.owner_id) {
                        Ok(values) => values,
                        Err(message) => {
                            return error(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "store_failed",
                                message,
                            )
                        }
                    }
                };
                let limit = query.limit as usize;
                let start = (page_number.saturating_sub(1) as usize).saturating_mul(limit);
                let has_more = connections.len() > start.saturating_add(limit);
                let summaries = connections
                    .into_iter()
                    .skip(start)
                    .take(limit)
                    .map(|connection| AegsConnectionSummary {
                        generator_id: self.producer_id.clone(),
                        connection_id: connection.connection_id,
                        status: connection_status(&connection.status, connection.expires_at_ms),
                        metadata: connection.metadata,
                        expires_at_ms: connection.expires_at_ms,
                        updated_at_ms: connection.updated_at_ms,
                    })
                    .collect();
                json(
                    StatusCode::OK,
                    serde_json::json!(AegsConnectionPage {
                        connections: summaries,
                        next_cursor: has_more.then(|| format!("page:{}", page_number + 1)),
                    }),
                )
            }
            (Method::POST, "/v1/connections/inspect") => {
                let authorization_guard = match self.authorize_management(&request) {
                    Ok(authorization) => authorization,
                    Err(response) => return *response,
                };
                let body = match read_body(request).await {
                    Ok(body) => body,
                    Err(response) => return response,
                };
                let inspection: AegsConnectionInspectionRequest =
                    match serde_json::from_slice(&body) {
                        Ok(value) => value,
                        Err(error_value) => {
                            return error(
                                StatusCode::BAD_REQUEST,
                                "invalid_json",
                                error_value.to_string(),
                            )
                        }
                    };
                if let Err(response) =
                    enforce_management_owner(&authorization_guard, &inspection.owner_id)
                {
                    return response;
                }
                if let Err(message) = inspection.validate() {
                    return error(
                        StatusCode::BAD_REQUEST,
                        "invalid_connection_inspection",
                        message,
                    );
                }
                match self
                    .inspect_owned_connection(
                        &inspection.generator_id,
                        &inspection.owner_id,
                        &inspection.connection_id,
                    )
                    .await
                {
                    Ok(inspection) => json(StatusCode::OK, inspection),
                    Err((status, code, message)) => error(status, code, message),
                }
            }
            (Method::POST, "/v1/connections/refresh") => {
                let authorization_guard = match self.authorize_management(&request) {
                    Ok(authorization) => authorization,
                    Err(response) => return *response,
                };
                let body = match read_body(request).await {
                    Ok(body) => body,
                    Err(response) => return response,
                };
                let refresh: AegsConnectionRefreshRequest = match serde_json::from_slice(&body) {
                    Ok(value) => value,
                    Err(error_value) => {
                        return error(
                            StatusCode::BAD_REQUEST,
                            "invalid_json",
                            error_value.to_string(),
                        )
                    }
                };
                if let Err(response) =
                    enforce_management_owner(&authorization_guard, &refresh.owner_id)
                {
                    return response;
                }
                if let Err(message) = refresh.validate() {
                    return error(
                        StatusCode::BAD_REQUEST,
                        "invalid_connection_refresh",
                        message,
                    );
                }
                if refresh.generator_id != self.producer_id {
                    return error(
                        StatusCode::CONFLICT,
                        "generator_mismatch",
                        format!(
                            "this AEGS owns {}, not {}",
                            self.producer_id, refresh.generator_id
                        ),
                    );
                }
                if self
                    .store
                    .claim_connection_owner(&refresh.connection_id, &refresh.owner_id)
                    .is_err()
                {
                    return error(
                        StatusCode::NOT_FOUND,
                        "connection_not_found",
                        "the owned connection was not found",
                    );
                }
                let provider = Arc::clone(&self.provider);
                let connection_id = refresh.connection_id.clone();
                match tokio::task::spawn_blocking(move || {
                    provider.refresh_connection(&connection_id)
                })
                .await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(message)) => {
                        return error(StatusCode::BAD_GATEWAY, "provider_refresh_failed", message)
                    }
                    Err(message) => {
                        return error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "provider_refresh_failed",
                            message.to_string(),
                        )
                    }
                }
                match self
                    .inspect_owned_connection(
                        &refresh.generator_id,
                        &refresh.owner_id,
                        &refresh.connection_id,
                    )
                    .await
                {
                    Ok(inspection) => json(StatusCode::OK, inspection),
                    Err((status, code, message)) => error(status, code, message),
                }
            }
            (Method::POST, "/v1/connections/test-event") => {
                let authorization_guard = match self.authorize_management(&request) {
                    Ok(authorization) => authorization,
                    Err(response) => return *response,
                };
                let body = match read_body(request).await {
                    Ok(body) => body,
                    Err(response) => return response,
                };
                let test: AegsConnectionTestEventRequest = match serde_json::from_slice(&body) {
                    Ok(value) => value,
                    Err(error_value) => {
                        return error(
                            StatusCode::BAD_REQUEST,
                            "invalid_json",
                            error_value.to_string(),
                        )
                    }
                };
                if let Err(response) =
                    enforce_management_owner(&authorization_guard, &test.owner_id)
                {
                    return response;
                }
                if let Err(message) = test.validate() {
                    return error(StatusCode::BAD_REQUEST, "invalid_test_event", message);
                }
                if test.generator_id != self.producer_id {
                    return error(
                        StatusCode::CONFLICT,
                        "generator_mismatch",
                        format!(
                            "this AEGS owns {}, not {}",
                            self.producer_id, test.generator_id
                        ),
                    );
                }
                if self
                    .store
                    .claim_connection_owner(&test.connection_id, &test.owner_id)
                    .is_err()
                {
                    return error(
                        StatusCode::NOT_FOUND,
                        "connection_not_found",
                        "the owned connection was not found",
                    );
                }
                let provider = Arc::clone(&self.provider);
                let connection_id = test.connection_id.clone();
                let event_type = test.event_type.clone();
                let normalized = match tokio::task::spawn_blocking(move || {
                    provider.test_event(&connection_id, event_type.as_deref())
                })
                .await
                {
                    Ok(Ok(Some(event))) => event,
                    Ok(Ok(None)) => {
                        return error(
                            StatusCode::NOT_IMPLEMENTED,
                            "test_event_unsupported",
                            "this event generator does not support test events",
                        )
                    }
                    Ok(Err(message)) => {
                        return error(StatusCode::BAD_GATEWAY, "test_event_failed", message)
                    }
                    Err(message) => {
                        return error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "test_event_failed",
                            message.to_string(),
                        )
                    }
                };
                match self
                    .publish_normalized_event(normalized, Some(test.connection_id.as_str()))
                    .await
                {
                    Ok(response) => json(StatusCode::ACCEPTED, response),
                    Err(message) => error(StatusCode::BAD_GATEWAY, "aeds_rejected", message),
                }
            }
            (Method::POST, "/v1/connections/revoke") => {
                let authorization_guard = match self.authorize_management(&request) {
                    Ok(authorization) => authorization,
                    Err(response) => return *response,
                };
                let body = match read_body(request).await {
                    Ok(body) => body,
                    Err(response) => return response,
                };
                let revoke: AegsConnectionRevokeRequest = match serde_json::from_slice(&body) {
                    Ok(value) => value,
                    Err(error_value) => {
                        return error(
                            StatusCode::BAD_REQUEST,
                            "invalid_json",
                            error_value.to_string(),
                        )
                    }
                };
                if let Err(response) =
                    enforce_management_owner(&authorization_guard, &revoke.owner_id)
                {
                    return response;
                }
                if let Err(message) = revoke.validate() {
                    return error(
                        StatusCode::BAD_REQUEST,
                        "invalid_connection_revoke",
                        message,
                    );
                }
                if revoke.generator_id != self.producer_id {
                    return error(
                        StatusCode::CONFLICT,
                        "generator_mismatch",
                        format!(
                            "this AEGS owns {}, not {}",
                            self.producer_id, revoke.generator_id
                        ),
                    );
                }
                let connection = match self
                    .store
                    .claim_connection_owner(&revoke.connection_id, &revoke.owner_id)
                {
                    Ok(connection) => connection,
                    Err(_) => {
                        return error(
                            StatusCode::NOT_FOUND,
                            "connection_not_found",
                            "the owned connection was not found",
                        )
                    }
                };
                if connection.status == "revoked" {
                    return json(
                        StatusCode::OK,
                        AegsConnectionRevokeResponse { revoked: true },
                    );
                }
                if let Err(message) = self.provider.revoke_connection(&revoke.connection_id) {
                    return error(StatusCode::BAD_GATEWAY, "provider_revoke_failed", message);
                }
                match self.store.revoke_connection(
                    &revoke.connection_id,
                    &revoke.owner_id,
                    now_ms(),
                ) {
                    Ok(true) => json(
                        StatusCode::OK,
                        AegsConnectionRevokeResponse { revoked: true },
                    ),
                    Ok(false) => json(
                        StatusCode::OK,
                        AegsConnectionRevokeResponse { revoked: true },
                    ),
                    Err(message) => {
                        error(StatusCode::INTERNAL_SERVER_ERROR, "store_failed", message)
                    }
                }
            }
            (Method::POST, "/v1/connections/reconnect") => {
                let authorization_guard = match self.authorize_management(&request) {
                    Ok(authorization) => authorization,
                    Err(response) => return *response,
                };
                let body = match read_body(request).await {
                    Ok(body) => body,
                    Err(response) => return response,
                };
                let reconnect: AegsConnectionReconnectRequest = match serde_json::from_slice(&body)
                {
                    Ok(value) => value,
                    Err(error_value) => {
                        return error(
                            StatusCode::BAD_REQUEST,
                            "invalid_json",
                            error_value.to_string(),
                        )
                    }
                };
                if let Err(response) =
                    enforce_management_owner(&authorization_guard, &reconnect.owner_id)
                {
                    return response;
                }
                if let Err(message) = reconnect.validate() {
                    return error(
                        StatusCode::BAD_REQUEST,
                        "invalid_connection_reconnect",
                        message,
                    );
                }
                if reconnect.generator_id != self.producer_id {
                    return error(
                        StatusCode::CONFLICT,
                        "generator_mismatch",
                        format!(
                            "this AEGS owns {}, not {}",
                            self.producer_id, reconnect.generator_id
                        ),
                    );
                }
                let existing = match self
                    .store
                    .claim_connection_owner(&reconnect.connection_id, &reconnect.owner_id)
                {
                    Ok(connection) => connection,
                    Err(_) => {
                        return error(
                            StatusCode::NOT_FOUND,
                            "connection_not_found",
                            "the owned connection was not found",
                        )
                    }
                };
                match self.provider.reconnect_authorization(
                    &reconnect.owner_id,
                    &reconnect.connection_id,
                    reconnect.return_url.as_deref(),
                ) {
                    Ok(flow) if flow.connection_id.as_deref() == Some(&reconnect.connection_id) => {
                        if flow.status == "ready" {
                            if let Err(message) = self.store.upsert_ready_connection(
                                &reconnect.connection_id,
                                &reconnect.owner_id,
                                self.provider.provider_slug(),
                                &existing.metadata,
                                now_ms(),
                            ) {
                                return error(
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    "connection_store_failed",
                                    message,
                                );
                            }
                        }
                        json(StatusCode::OK, serde_json::json!(flow))
                    }
                    Ok(_) => error(
                        StatusCode::BAD_GATEWAY,
                        "connection_identity_changed",
                        "provider reconnection must retain the existing connection ID",
                    ),
                    Err(message) => error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "authorization_reconnect_failed",
                        message,
                    ),
                }
            }
            (Method::POST, "/v1/resources/query") => {
                let authorization_guard = match self.authorize_management(&request) {
                    Ok(authorization) => authorization,
                    Err(response) => return *response,
                };
                let body = match read_body(request).await {
                    Ok(body) => body,
                    Err(response) => return response,
                };
                let query: AegsProviderResourceQuery = match serde_json::from_slice(&body) {
                    Ok(value) => value,
                    Err(error_value) => {
                        return error(
                            StatusCode::BAD_REQUEST,
                            "invalid_json",
                            error_value.to_string(),
                        )
                    }
                };
                if let Err(response) =
                    enforce_management_owner(&authorization_guard, &query.owner_id)
                {
                    return response;
                }
                if let Err(message) = query.validate() {
                    return error(StatusCode::BAD_REQUEST, "invalid_resource_query", message);
                }
                if query.generator_id != self.producer_id {
                    return error(
                        StatusCode::CONFLICT,
                        "generator_mismatch",
                        format!(
                            "this AEGS owns {}, not {}",
                            self.producer_id, query.generator_id
                        ),
                    );
                }
                if !self.provider.authorization_configured() {
                    return error(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "authorization_unavailable",
                        "provider authorization credentials are not configured for this AEGS",
                    );
                }
                if self
                    .store
                    .claim_connection_owner(&query.connection_id, &query.owner_id)
                    .is_err()
                {
                    return error(
                        StatusCode::NOT_FOUND,
                        "connection_not_found",
                        "the owned connection was not found",
                    );
                }
                let provider = Arc::clone(&self.provider);
                match tokio::task::spawn_blocking(move || provider.query_resources(&query)).await {
                    Ok(Ok(page)) => json(StatusCode::OK, serde_json::json!(page)),
                    Ok(Err(message)) if message == "authorization_pending" => error(
                        StatusCode::CONFLICT,
                        "authorization_pending",
                        "provider authorization is not complete",
                    ),
                    Ok(Err(message)) => {
                        error(StatusCode::BAD_GATEWAY, "resource_query_failed", message)
                    }
                    Err(message) => error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "resource_query_failed",
                        message.to_string(),
                    ),
                }
            }
            (Method::GET, "/oauth/callback") => {
                if !self.provider.authorization_configured() {
                    return error(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "authorization_unavailable",
                        "provider authorization credentials are not configured for this AEGS",
                    );
                }
                let query = match request
                    .uri()
                    .query()
                    .map(parse_query)
                    .unwrap_or_else(|| Ok(HashMap::new()))
                {
                    Ok(query) => query,
                    Err(message) => {
                        return error(StatusCode::BAD_REQUEST, "invalid_callback", message)
                    }
                };
                // GitHub sends `setup_action=update` when an administrator changes an
                // installation's repository or permission scope from GitHub settings. That
                // administrative callback has no OAuth state and must not be treated as a
                // connection authorization completion (nor should its one-time code be
                // exchanged without an owner-bound pending authorization).
                if query.get("setup_action").map(String::as_str) == Some("update")
                    && !query.contains_key("state")
                {
                    return authorization_setup_updated_page(
                        &self.producer_id,
                        query.get("installation_id").map(String::as_str),
                    );
                }
                let provider = Arc::clone(&self.provider);
                match tokio::task::spawn_blocking(move || provider.complete_authorization(&query))
                    .await
                {
                    Ok(Ok(callback)) => authorization_complete_page(
                        &self.producer_id,
                        &callback.connection_id,
                        callback.return_url.as_deref(),
                    ),
                    Ok(Err(message)) => {
                        error(StatusCode::BAD_REQUEST, "authorization_failed", message)
                    }
                    Err(message) => error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "authorization_failed",
                        message.to_string(),
                    ),
                }
            }
            (Method::POST, "/v1/emit") if self.provider.allows_direct_emit() => {
                let body = match read_body(request).await {
                    Ok(body) => body,
                    Err(response) => return response,
                };
                let event: PublishEventRequest = match serde_json::from_slice(&body) {
                    Ok(event) => event,
                    Err(error_value) => {
                        return error(
                            StatusCode::BAD_REQUEST,
                            "invalid_json",
                            error_value.to_string(),
                        )
                    }
                };
                match self.publisher.publish(event).await {
                    Ok(response) => json(StatusCode::ACCEPTED, serde_json::json!(response)),
                    Err(message) => error(StatusCode::BAD_GATEWAY, "aeds_rejected", message),
                }
            }
            (Method::POST, path) if self.provider.parse_webhook_route(path).is_some() => {
                let route = self
                    .provider
                    .parse_webhook_route(path)
                    .expect("route was matched above");
                let headers = normalized_headers(request.headers());
                let body = match read_body(request).await {
                    Ok(body) => body,
                    Err(response) => return response,
                };
                let input = WebhookInput {
                    headers: &headers,
                    body: &body,
                    now_ms: now_ms(),
                };
                if let Some(control) = self.provider.control_webhook(input) {
                    return match control {
                        Ok(ControlWebhookResponse { content_type, body }) => {
                            text(StatusCode::OK, body, &content_type)
                        }
                        Err(message) => {
                            error(StatusCode::UNAUTHORIZED, "invalid_signature", message)
                        }
                    };
                }
                let normalized = match self.provider.normalize_webhook(input, &route) {
                    Ok(value) => value,
                    Err(message) => {
                        return error(
                            if message.contains("signature")
                                || message.contains("authorization")
                                || message.contains("authentication")
                            {
                                StatusCode::UNAUTHORIZED
                            } else {
                                StatusCode::UNPROCESSABLE_ENTITY
                            },
                            "webhook_rejected",
                            message,
                        )
                    }
                };
                let subscriptions = match self.store.matching(
                    &self.producer_id,
                    &normalized.event_type,
                    &normalized.connection_scope,
                ) {
                    Ok(values) => values,
                    Err(message) => {
                        return error(StatusCode::INTERNAL_SERVER_ERROR, "store_failed", message)
                    }
                };
                let mut interest_keys = HashSet::new();
                let mut responses = Vec::new();
                for subscription in subscriptions {
                    if route
                        .connection_id
                        .as_deref()
                        .is_some_and(|connection_id| subscription.connection_id != connection_id)
                    {
                        continue;
                    }
                    if !metadata_matches_filter(&normalized.metadata, &subscription.filter) {
                        continue;
                    }
                    if !interest_keys.insert(subscription.event_interest_key.clone()) {
                        continue;
                    }
                    let connection_id = subscription.connection_id.clone();
                    let event = PublishEventRequest {
                        producer_id: self.producer_id.clone(),
                        event_interest_key: subscription.event_interest_key,
                        occurrence_id: normalized.occurrence_id.clone(),
                        event_type: normalized.event_type.clone(),
                        event_type_version: subscription.event_type_version,
                        occurred_at: normalized.occurred_at.clone(),
                        prompt: normalized.prompt.clone(),
                        artifacts: Vec::new(),
                        metadata: normalized.metadata.clone(),
                        reply_context: normalized.reply_context.clone(),
                        ttl_seconds: chariox_event_protocol::DEFAULT_EVENT_DELIVERY_TTL_SECONDS,
                    };
                    match self.publisher.publish(event).await {
                        Ok(response) => {
                            if let Err(message) = self
                                .store
                                .mark_connection_event_accepted(&connection_id, now_ms())
                            {
                                return error(
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    "store_failed",
                                    message,
                                );
                            }
                            responses.push(response)
                        }
                        Err(message) => {
                            return error(StatusCode::BAD_GATEWAY, "aeds_rejected", message)
                        }
                    }
                }
                json(
                    StatusCode::ACCEPTED,
                    serde_json::json!({
                        "occurrence_id": normalized.occurrence_id,
                        "matched_interest_count": interest_keys.len(),
                        "publications": responses,
                    }),
                )
            }
            _ => error(StatusCode::NOT_FOUND, "not_found", "route was not found"),
        }
    }

    async fn inspect_owned_connection(
        &self,
        generator_id: &str,
        owner_id: &str,
        connection_id: &str,
    ) -> Result<AegsConnectionInspection, (StatusCode, &'static str, String)> {
        if generator_id != self.producer_id {
            return Err((
                StatusCode::CONFLICT,
                "generator_mismatch",
                format!("this AEGS owns {}, not {generator_id}", self.producer_id),
            ));
        }
        let connection = self
            .store
            .claim_connection_owner(connection_id, owner_id)
            .map_err(|_| {
                (
                    StatusCode::NOT_FOUND,
                    "connection_not_found",
                    "the owned connection was not found".to_string(),
                )
            })?;
        let provider = Arc::clone(&self.provider);
        let requested_connection_id = connection_id.to_string();
        let provider_inspection = tokio::task::spawn_blocking(move || {
            provider.inspect_connection(&requested_connection_id)
        })
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "provider_inspection_failed",
                error.to_string(),
            )
        })?
        .map_err(|message| {
            (
                StatusCode::BAD_GATEWAY,
                "provider_inspection_failed",
                message,
            )
        })?;
        let inspection = provider_inspection.unwrap_or_else(|| {
            baseline_provider_connection_inspection(&self.producer_id, &connection, false)
        });
        if inspection.generator_id != self.producer_id
            || inspection.connection_id != connection.connection_id
        {
            return Err((
                StatusCode::BAD_GATEWAY,
                "invalid_provider_inspection",
                "provider inspection returned a different connection identity".to_string(),
            ));
        }
        Ok(inspection)
    }

    async fn publish_normalized_event(
        &self,
        normalized: crate::NormalizedEvent,
        connection_id: Option<&str>,
    ) -> Result<AegsConnectionTestEventResponse, String> {
        let subscriptions = self.store.matching(
            &self.producer_id,
            &normalized.event_type,
            &normalized.connection_scope,
        )?;
        let mut interest_keys = HashSet::new();
        for subscription in subscriptions {
            if connection_id.is_some_and(|value| subscription.connection_id != value) {
                continue;
            }
            if !metadata_matches_filter(&normalized.metadata, &subscription.filter) {
                continue;
            }
            if !interest_keys.insert(subscription.event_interest_key.clone()) {
                continue;
            }
            let accepted_connection_id = subscription.connection_id.clone();
            self.publisher
                .publish(PublishEventRequest {
                    producer_id: self.producer_id.clone(),
                    event_interest_key: subscription.event_interest_key,
                    occurrence_id: normalized.occurrence_id.clone(),
                    event_type: normalized.event_type.clone(),
                    event_type_version: subscription.event_type_version,
                    occurred_at: normalized.occurred_at.clone(),
                    prompt: normalized.prompt.clone(),
                    artifacts: Vec::new(),
                    metadata: normalized.metadata.clone(),
                    reply_context: normalized.reply_context.clone(),
                    ttl_seconds: chariox_event_protocol::DEFAULT_EVENT_DELIVERY_TTL_SECONDS,
                })
                .await?;
            self.store
                .mark_connection_event_accepted(&accepted_connection_id, now_ms())?;
        }
        let accepted = !interest_keys.is_empty();
        Ok(AegsConnectionTestEventResponse {
            occurrence_id: normalized.occurrence_id,
            accepted,
            message: (!accepted)
                .then(|| "no active workflow trigger matched the requested test event".to_string()),
        })
    }

    fn authorize_management(
        &self,
        request: &Request<Incoming>,
    ) -> Result<ManagementAuthorization, Box<Response<Full<Bytes>>>> {
        let presented = request
            .headers()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        if self
            .management_token
            .as_deref()
            .is_some_and(|expected| Some(expected) == presented)
        {
            return Ok(ManagementAuthorization::Static);
        }
        if let Some(public_key) = self.management_public_key.as_ref() {
            let Some(presented) = presented else {
                return Err(Box::new(error(
                    StatusCode::UNAUTHORIZED,
                    "unauthorized",
                    "valid AEGS management capability required",
                )));
            };
            let claims = verify_management_capability_scoped(
                presented,
                public_key,
                &self.management_issuer,
                &self.producer_id,
                now_ms() / 1_000,
                self.management_url.as_deref(),
                self.manifest_digest.as_deref(),
            )
            .map_err(|message| {
                Box::new(error(StatusCode::UNAUTHORIZED, "unauthorized", message))
            })?;
            let presented_owner = request
                .headers()
                .get("x-chariox-owner-id")
                .and_then(|value| value.to_str().ok());
            if presented_owner
                .is_none_or(|owner| !claims.owner_ids.iter().any(|allowed| allowed == owner))
            {
                return Err(Box::new(error(
                    StatusCode::FORBIDDEN,
                    "management_owner_mismatch",
                    "the management capability is not bound to the requested owner",
                )));
            }
            return Ok(ManagementAuthorization::Signed(claims));
        }
        if self.management_token.is_none() && self.management_public_key.is_none() {
            return Err(Box::new(error(
                StatusCode::SERVICE_UNAVAILABLE,
                "management_disabled",
                "AEGS management token or signed capability key is not configured",
            )));
        }
        Err(Box::new(error(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "valid AEGS management capability required",
        )))
    }
}

fn management_owner(
    request: &Request<Incoming>,
    authorization: &ManagementAuthorization,
) -> Option<String> {
    match authorization {
        ManagementAuthorization::Signed(_) => request
            .headers()
            .get("x-chariox-owner-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string),
        ManagementAuthorization::Static => request
            .headers()
            .get("x-chariox-owner-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string),
    }
}

fn enforce_management_owner(
    authorization: &ManagementAuthorization,
    requested_owner: &str,
) -> Result<(), Response<Full<Bytes>>> {
    if let ManagementAuthorization::Signed(claims) = authorization {
        if !claims
            .owner_ids
            .iter()
            .any(|owner| owner == requested_owner)
        {
            return Err(error(
                StatusCode::FORBIDDEN,
                "management_owner_mismatch",
                "the management capability is not bound to the requested owner",
            ));
        }
    }
    Ok(())
}

fn connection_status(status: &str, expires_at_ms: Option<u64>) -> AegsConnectionStatus {
    if status == "ready" && expires_at_ms.is_some_and(|expires_at_ms| expires_at_ms <= now_ms()) {
        return AegsConnectionStatus::Expired;
    }
    match status {
        "pending" => AegsConnectionStatus::Pending,
        "ready" => AegsConnectionStatus::Ready,
        "expired" => AegsConnectionStatus::Expired,
        "revoked" => AegsConnectionStatus::Revoked,
        "unavailable" => AegsConnectionStatus::Unavailable,
        _ => AegsConnectionStatus::Error,
    }
}

pub fn read_secret(name: &str, file_name: &str) -> Result<Option<String>, String> {
    if let Some(value) = std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return Ok(Some(value));
    }
    let Some(path) = std::env::var_os(file_name).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let value = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(format!("{file_name} must not be empty"));
    }
    Ok(Some(value))
}

async fn read_body(request: Request<Incoming>) -> Result<Bytes, Response<Full<Bytes>>> {
    let body = request.into_body().collect().await.map_err(|error_value| {
        error(
            StatusCode::BAD_REQUEST,
            "invalid_body",
            error_value.to_string(),
        )
    })?;
    let body = body.to_bytes();
    if body.len() > MAX_WEBHOOK_BYTES {
        return Err(error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload_too_large",
            format!("webhook exceeds {MAX_WEBHOOK_BYTES} bytes"),
        ));
    }
    Ok(body)
}

fn normalized_headers(headers: &hyper::HeaderMap) -> HashMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_ascii_lowercase(), value.to_string()))
        })
        .collect()
}

fn parse_query(value: &str) -> Result<HashMap<String, String>, String> {
    let mut values = HashMap::new();
    for (key, value) in url::form_urlencoded::parse(value.as_bytes()) {
        if values
            .insert(key.into_owned(), value.into_owned())
            .is_some()
        {
            return Err("authorization callback contains a duplicate parameter".to_string());
        }
    }
    Ok(values)
}

fn authorization_complete_page(
    generator_id: &str,
    connection_id: &str,
    return_url: Option<&str>,
) -> Response<Full<Bytes>> {
    let generator_json =
        serde_json::to_string(generator_id).unwrap_or_else(|_| "\"unknown\"".to_string());
    let connection_json =
        serde_json::to_string(connection_id).unwrap_or_else(|_| "\"unknown\"".to_string());
    let (origin_json, return_link) = return_url
        .and_then(|value| {
            let url = url::Url::parse(value).ok()?;
            let origin = url.origin().ascii_serialization();
            Some((
                serde_json::to_string(&origin).ok()?,
                format!("<a href=\"{}\">Return to Chariox</a>", html_escape(value)),
            ))
        })
        .unwrap_or_else(|| ("\"*\"".to_string(), String::new()));
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Connection authorized</title></head>\
         <body><main><h1>Connection authorized</h1><p>You can close this window.</p>{return_link}</main>\
         <script>window.opener?.postMessage({{type:'chariox-event-authorization',generatorId:{generator_json},connectionId:{connection_json}}},{origin_json});window.close();</script>\
         </body></html>"
    );
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/html; charset=utf-8")
        .header("cache-control", "no-store")
        .header(
            "content-security-policy",
            "default-src 'none'; script-src 'unsafe-inline'; style-src 'none'; base-uri 'none'; frame-ancestors 'none'",
        )
        .body(Full::new(Bytes::from(html)))
        .expect("valid authorization response")
}

fn authorization_setup_updated_page(
    generator_id: &str,
    installation_id: Option<&str>,
) -> Response<Full<Bytes>> {
    let generator = html_escape(generator_id);
    let installation = installation_id
        .map(html_escape)
        .map(|value| format!("<p>Installation: <code>{value}</code></p>"))
        .unwrap_or_default();
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Installation updated</title></head>\
         <body><main><h1>Installation updated</h1><p>The {generator} installation was updated.\
         Return to Chariox and reconnect or refresh this service so the kernel can verify the\
         new permissions.</p>{installation}<p>You can close this window.</p></main></body></html>"
    );
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/html; charset=utf-8")
        .header("cache-control", "no-store")
        .header(
            "content-security-policy",
            "default-src 'none'; style-src 'none'; base-uri 'none'; frame-ancestors 'none'",
        )
        .body(Full::new(Bytes::from(html)))
        .expect("valid installation update response")
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn json(status: StatusCode, body: impl serde::Serialize) -> Response<Full<Bytes>> {
    let body = serde_json::to_string(&body).expect("AEGS JSON response should serialize");
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body)))
        .expect("valid response")
}

fn text(status: StatusCode, body: String, content_type: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("content-type", content_type)
        .body(Full::new(Bytes::from(body)))
        .expect("valid response")
}

fn error(
    status: StatusCode,
    code: impl Into<String>,
    message: impl Into<String>,
) -> Response<Full<Bytes>> {
    json(
        status,
        serde_json::json!({"error": {"code": code.into(), "message": message.into()}}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chariox_event_protocol::AegsConnectionLifecycleState;

    #[test]
    fn query_parser_rejects_duplicate_callback_parameters() {
        assert!(parse_query("state=one&state=two").is_err());
    }

    #[test]
    fn authorization_page_escapes_return_link() {
        let response = authorization_complete_page(
            "dev.chariox.test",
            "connection-1",
            Some("https://example.test/return?value=%22"),
        );
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("cache-control")
                .and_then(|value| value.to_str().ok()),
            Some("no-store")
        );
    }

    #[test]
    fn setup_update_without_state_is_an_informational_success() {
        let response = authorization_setup_updated_page("dev.arroba.github", Some("153623497"));
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("cache-control")
                .and_then(|value| value.to_str().ok()),
            Some("no-store")
        );
    }

    #[test]
    fn baseline_inspection_never_claims_provider_health_or_test_support() {
        let inspection = baseline_provider_connection_inspection(
            "dev.chariox.github",
            &crate::ConnectionRecord {
                connection_id: "connection-1".to_string(),
                owner_id: "owner-1".to_string(),
                provider: "github".to_string(),
                status: "ready".to_string(),
                encrypted_credential: None,
                metadata: serde_json::Value::Null,
                expires_at_ms: None,
                updated_at_ms: 1,
                last_successful_health_check_at_ms: None,
                last_accepted_event_at_ms: None,
            },
            false,
        );
        assert_eq!(
            inspection.lifecycle_state,
            AegsConnectionLifecycleState::Connected
        );
        assert_eq!(inspection.last_successful_health_check_at_ms, None);
        assert!(!inspection.test_event_supported);
    }

    #[test]
    fn context_actions_do_not_persist_provider_responses() {
        assert!(!action_receipt_is_durable("event.context"));
        assert!(!action_receipt_is_durable("notification.context"));
        assert!(action_receipt_is_durable("notification.reply"));
        assert!(action_receipt_is_durable("custom.action"));
    }

    #[test]
    fn signed_management_capability_cannot_cross_owner_boundary() {
        let authorization = ManagementAuthorization::Signed(
            crate::management_capability::ManagementCapabilityClaims {
                issuer: "chariox-cloud".to_string(),
                audience: "aegs-management".to_string(),
                subject: "kernel-1".to_string(),
                generator_id: "dev.chariox.slack".to_string(),
                manifest_digest: "sha256:test".to_string(),
                management_url: "https://aegs.example.test".to_string(),
                user_id: "user-1".to_string(),
                owner_ids: vec!["owner-1".to_string(), "kernel-1".to_string()],
                issued_at: 100,
                expires_at: 200,
                token_id: "cap-1".to_string(),
            },
        );
        assert!(enforce_management_owner(&authorization, "owner-1").is_ok());
        let response = enforce_management_owner(&authorization, "owner-2").unwrap_err();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
