use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use arroba_event_protocol::{
    AegsAuthorizationStartRequest, AegsConnectionPage, AegsConnectionQuery,
    AegsConnectionReconnectRequest, AegsConnectionRevokeRequest, AegsConnectionRevokeResponse,
    AegsConnectionStatus, AegsConnectionSummary, AegsProviderResourceQuery, PublishEventRequest,
};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

use crate::{
    metadata_matches_filter, now_ms, AedsPublisher, AegsProvider, AegsStore,
    ControlWebhookResponse, WebhookInput, AEGS_PROTOCOL_VERSION, MAX_WEBHOOK_BYTES,
};

#[derive(Clone)]
struct AegsServer {
    producer_id: String,
    management_token: Option<String>,
    publisher: AedsPublisher,
    store: AegsStore,
    provider: Arc<dyn AegsProvider>,
}

pub async fn run_from_environment<F>(provider_factory: F) -> Result<(), Box<dyn std::error::Error>>
where
    F: FnOnce(AegsStore) -> Result<Arc<dyn AegsProvider>, String>,
{
    let address: SocketAddr = std::env::var("ARROBA_AEGS_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:43132".to_string())
        .parse()?;
    let store = AegsStore::open(
        std::env::var("ARROBA_AEGS_DATABASE_PATH").unwrap_or_else(|_| "aegs.db".to_string()),
    )?;
    let provider = provider_factory(store.clone())?;
    let producer_id = std::env::var("ARROBA_AEGS_PRODUCER_ID")
        .unwrap_or_else(|_| provider.generator_id().to_string());
    if producer_id != provider.generator_id() {
        return Err(format!(
            "producer ID `{producer_id}` does not match provider generator `{}`",
            provider.generator_id()
        )
        .into());
    }
    let server = AegsServer {
        producer_id: producer_id.clone(),
        management_token: read_secret(
            "ARROBA_AEGS_MANAGEMENT_TOKEN",
            "ARROBA_AEGS_MANAGEMENT_TOKEN_FILE",
        )?,
        publisher: AedsPublisher::new(
            producer_id.clone(),
            read_secret(
                "ARROBA_AEGS_PRODUCER_TOKEN",
                "ARROBA_AEGS_PRODUCER_TOKEN_FILE",
            )?,
            std::env::var("ARROBA_AEDS_EVENTS_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:43131/v1/events".to_string()),
        ),
        store,
        provider,
    };
    if server.provider.authorization_configured() {
        spawn_subscription_maintenance(Arc::clone(&server.provider));
    }
    let listener = TcpListener::bind(address).await?;
    eprintln!(
        "{}",
        serde_json::json!({
            "component": "arroba-aegs",
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
    let maintenance_interval = std::env::var("ARROBA_AEGS_MAINTENANCE_INTERVAL_SECS")
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
                        "component": "arroba-aegs",
                        "event": "subscription_maintenance_failed",
                        "error": message,
                    })
                ),
                Err(error) => eprintln!(
                    "{}",
                    serde_json::json!({
                        "component": "arroba-aegs",
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
                    "component": "arroba-aegs",
                    "provider": self.producer_id,
                    "protocol_version": AEGS_PROTOCOL_VERSION,
                    "build_revision": option_env!("ARROBA_BUILD_REVISION").unwrap_or("development"),
                    "authorization_configured": self.provider.authorization_configured(),
                }),
            ),
            (Method::GET, "/metrics") => match self.store.metrics() {
                Ok(metrics) => text(
                    StatusCode::OK,
                    format!(
                        "arroba_aegs_active_subscriptions {}\n\
                         arroba_aegs_subscriptions {}\n\
                         arroba_aegs_connections {}\n\
                         arroba_aegs_provider_hooks {}\n",
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
                if let Err(response) = self.authorize_management(&request) {
                    return *response;
                }
                match self.store.all(&self.producer_id) {
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
                if let Err(response) = self.authorize_management(&request) {
                    return *response;
                }
                let body = match read_body(request).await {
                    Ok(body) => body,
                    Err(response) => return response,
                };
                let reconcile: arroba_event_protocol::AegsSubscriptionReconcileRequest =
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
                match self.store.reconcile(
                    &reconcile.owner_id,
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
            (Method::POST, "/v1/authorizations") => {
                if let Err(response) = self.authorize_management(&request) {
                    return *response;
                }
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
                if let Err(response) = self.authorize_management(&request) {
                    return *response;
                }
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
            (Method::POST, "/v1/connections/revoke") => {
                if let Err(response) = self.authorize_management(&request) {
                    return *response;
                }
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
                if let Err(response) = self.authorize_management(&request) {
                    return *response;
                }
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
                if let Err(response) = self.authorize_management(&request) {
                    return *response;
                }
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
                        ttl_seconds: arroba_event_protocol::DEFAULT_EVENT_DELIVERY_TTL_SECONDS,
                    };
                    match self.publisher.publish(event).await {
                        Ok(response) => responses.push(response),
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

    fn authorize_management(
        &self,
        request: &Request<Incoming>,
    ) -> Result<(), Box<Response<Full<Bytes>>>> {
        let Some(expected) = self.management_token.as_deref() else {
            return Err(Box::new(error(
                StatusCode::SERVICE_UNAVAILABLE,
                "management_disabled",
                "AEGS management token is not configured",
            )));
        };
        let presented = request
            .headers()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        if presented != Some(expected) {
            return Err(Box::new(error(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "valid AEGS management capability required",
            )));
        }
        Ok(())
    }
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
                format!("<a href=\"{}\">Return to Arroba</a>", html_escape(value)),
            ))
        })
        .unwrap_or_else(|| ("\"*\"".to_string(), String::new()));
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Connection authorized</title></head>\
         <body><main><h1>Connection authorized</h1><p>You can close this window.</p>{return_link}</main>\
         <script>window.opener?.postMessage({{type:'arroba-event-authorization',generatorId:{generator_json},connectionId:{connection_json}}},{origin_json});window.close();</script>\
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

    #[test]
    fn query_parser_rejects_duplicate_callback_parameters() {
        assert!(parse_query("state=one&state=two").is_err());
    }

    #[test]
    fn authorization_page_escapes_return_link() {
        let response = authorization_complete_page(
            "dev.arroba.test",
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
}
