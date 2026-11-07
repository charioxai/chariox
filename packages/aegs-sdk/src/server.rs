use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use chariox_event_protocol::{
    AegsAuthorizationStartRequest, AegsConnectionInspection, AegsConnectionInspectionRequest,
    AegsConnectionPage, AegsConnectionQuery, AegsConnectionReconnectRequest,
    AegsConnectionRefreshRequest, AegsConnectionRevokeRequest, AegsConnectionRevokeResponse,
    AegsConnectionStatus, AegsConnectionSummary, AegsConnectionTestEventRequest,
    AegsConnectionTestEventResponse, AegsProviderResourceQuery, PublishEventRequest,
};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::net::TcpListener;

use crate::kernel_auth::{Admission, AuthenticatedKernel, ManagementAuth, Refusal};
use crate::store::OwnerMembership;
use crate::{
    baseline_provider_connection_inspection, now_ms, AedsPublisher, AegsProvider, AegsStore,
    ControlWebhookResponse, WebhookInput, AEGS_PROTOCOL_VERSION, MAX_WEBHOOK_BYTES,
};

mod inbox;
mod scopes;
use inbox::InboxAttempt;

#[derive(Clone)]
struct AegsServer {
    producer_id: String,
    management_auth: ManagementAuth,
    publisher: AedsPublisher,
    store: AegsStore,
    provider: Arc<dyn AegsProvider>,
    scope_cache: scopes::ScopeCache,
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
    let server = AegsServer {
        producer_id: producer_id.clone(),
        management_auth: ManagementAuth::new(Admission::from_env(address.ip().is_loopback())?),
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
        scope_cache: Default::default(),
    };
    if server.provider.authorization_configured() {
        spawn_subscription_maintenance(Arc::clone(&server.provider));
    }
    inbox::spawn_inbox_delivery(server.clone());
    inbox::spawn_delivery_sweep(server.clone());
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
    let connections = Arc::new(tokio::sync::Semaphore::new(64));
    loop {
        let permit = Arc::clone(&connections).acquire_owned().await?;
        let (stream, _) = listener.accept().await?;
        let server = server.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let service = service_fn(move |request| {
                let server = server.clone();
                async move { Ok::<_, Infallible>(server.handle(request).await) }
            });
            let _ = http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(Duration::from_secs(15))
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
                Ok(Err(_message)) => eprintln!(
                    "{}",
                    serde_json::json!({
                        "component": "chariox-aegs",
                        "event": "subscription_maintenance_failed",
                        "error": "provider_maintenance_failed",
                    })
                ),
                Err(_error) => eprintln!(
                    "{}",
                    serde_json::json!({
                        "component": "chariox-aegs",
                        "event": "subscription_maintenance_task_failed",
                        "error": "provider_maintenance_task_failed",
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
                let (authorization, _) = match self.authenticate_management(request).await {
                    Ok(authenticated) => authenticated,
                    Err(response) => return *response,
                };
                match self
                    .store
                    .all_for_owner(&self.producer_id, &authorization.owner_id)
                {
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
                let (authorization, body) = match self.authenticate_management(request).await {
                    Ok(authenticated) => authenticated,
                    Err(response) => return *response,
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
                    return *response;
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
                let (subscriptions, refused) = match self
                    .vet_subscription_claims(&reconcile.owner_id, &reconcile.subscriptions)
                    .await
                {
                        Ok(vetted) => vetted,
                        Err(message) => {
                            return error(
                                StatusCode::BAD_GATEWAY,
                                "scope_check_unavailable",
                                format!(
                                    "the connection's resources could not be listed; stored subscriptions are unchanged: {message}"
                                ),
                            )
                        }
                    };
                match self.store.reconcile_for_owner(
                    &authorization.kernel_id,
                    &reconcile.owner_id,
                    &reconcile.generator_id,
                    &subscriptions,
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
                                "refused": refused,
                            }),
                        )
                    }
                    Err(message) => error(StatusCode::BAD_REQUEST, "invalid_subscription", message),
                }
            }
            (Method::GET, "/v1/owners/kernels") => {
                let (authorization, _) = match self.authenticate_management(request).await {
                    Ok(authenticated) => authenticated,
                    Err(response) => return *response,
                };
                match self.store.owner_kernels(&authorization.owner_id) {
                    Ok(kernels) => json(
                        StatusCode::OK,
                        serde_json::json!({"kernels": kernels
                            .into_iter()
                            .map(|(kernel_id, state)| serde_json::json!({"kernel_id": kernel_id, "state": state}))
                            .collect::<Vec<_>>()}),
                    ),
                    Err(message) => {
                        error(StatusCode::INTERNAL_SERVER_ERROR, "store_failed", message)
                    }
                }
            }
            (Method::POST, "/v1/owners/kernels/revoke") => {
                let (authorization, body) = match self.authenticate_management(request).await {
                    Ok(authenticated) => authenticated,
                    Err(response) => return *response,
                };
                let revoke: chariox_event_protocol::AegsOwnerKernelRevokeRequest =
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
                if let Err(response) = enforce_management_owner(&authorization, &revoke.owner_id) {
                    return *response;
                }
                match self
                    .store
                    .revoke_owner_kernel(&revoke.owner_id, &revoke.kernel_id, now_ms())
                {
                    Ok(()) => json(StatusCode::OK, serde_json::json!({"revoked": true})),
                    Err(message) => {
                        error(StatusCode::INTERNAL_SERVER_ERROR, "store_failed", message)
                    }
                }
            }
            (Method::POST, "/v1/authorizations") => {
                let (authorization_guard, body) = match self.authenticate_management(request).await
                {
                    Ok(authenticated) => authenticated,
                    Err(response) => return *response,
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
                    return *response;
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
            (Method::POST, "/v1/events/query") => self.query_received_events(request).await,
            (Method::POST, "/v1/events/replay") => self.replay_received_event(request).await,
            (Method::POST, "/v1/connections/query") => {
                let (authorization_guard, body) = match self.authenticate_management(request).await
                {
                    Ok(authenticated) => authenticated,
                    Err(response) => return *response,
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
                    return *response;
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
                let (authorization_guard, body) = match self.authenticate_management(request).await
                {
                    Ok(authenticated) => authenticated,
                    Err(response) => return *response,
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
                    return *response;
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
                let (authorization_guard, body) = match self.authenticate_management(request).await
                {
                    Ok(authenticated) => authenticated,
                    Err(response) => return *response,
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
                    return *response;
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
                let (authorization_guard, body) = match self.authenticate_management(request).await
                {
                    Ok(authenticated) => authenticated,
                    Err(response) => return *response,
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
                    return *response;
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
                let (authorization_guard, body) = match self.authenticate_management(request).await
                {
                    Ok(authenticated) => authenticated,
                    Err(response) => return *response,
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
                    return *response;
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
                match self
                    .store
                    .revoke_connection_wait(&revoke.connection_id, &revoke.owner_id, now_ms())
                    .await
                {
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
                let (authorization_guard, body) = match self.authenticate_management(request).await
                {
                    Ok(authenticated) => authenticated,
                    Err(response) => return *response,
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
                    return *response;
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
                let (authorization_guard, body) = match self.authenticate_management(request).await
                {
                    Ok(authenticated) => authenticated,
                    Err(response) => return *response,
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
                    return *response;
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
                let mut event: PublishEventRequest = match serde_json::from_slice(&body) {
                    Ok(event) => event,
                    Err(error_value) => {
                        return error(
                            StatusCode::BAD_REQUEST,
                            "invalid_json",
                            error_value.to_string(),
                        )
                    }
                };
                // A direct emission reaches the kernels subscribed to its interest, never a
                // route the caller names.
                event.audience = match self.store.audience_for_interest(
                    &self.producer_id,
                    &event.event_interest_key,
                    None,
                ) {
                    Ok(audience) => audience,
                    Err(message) => {
                        return error(StatusCode::INTERNAL_SERVER_ERROR, "store_error", message)
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
                let delivery_id = self
                    .provider
                    .webhook_delivery_id(input)
                    .unwrap_or_else(|| crate::random_opaque("delivery"));
                // The event is durable before GitHub-style senders, which never retry on their
                // own, are told it arrived.
                if let Err(message) = self.store.record_inbox_event(
                    &delivery_id,
                    route.connection_id.as_deref(),
                    &normalized,
                    now_ms(),
                ) {
                    return error(StatusCode::INTERNAL_SERVER_ERROR, "store_failed", message);
                }
                match self
                    .attempt_inbox_delivery(&delivery_id, None, now_ms())
                    .await
                {
                    Ok(InboxAttempt::Published(responses)) => json(
                        StatusCode::ACCEPTED,
                        serde_json::json!({
                            "delivery_id": delivery_id,
                            "occurrence_id": normalized.occurrence_id,
                            "matched_interest_count": responses.len(),
                            "publications": responses,
                        }),
                    ),
                    Ok(InboxAttempt::Retrying | InboxAttempt::NotRun) => {
                        match self.store.inbox_event(&delivery_id) {
                            Ok(Some(entry)) => json(
                                StatusCode::ACCEPTED,
                                serde_json::json!({
                                    "delivery_id": delivery_id,
                                    "occurrence_id": normalized.occurrence_id,
                                    "status": entry.status,
                                }),
                            ),
                            Ok(None) => error(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "store_failed",
                                "the stored webhook event disappeared",
                            ),
                            Err(message) => {
                                error(StatusCode::INTERNAL_SERVER_ERROR, "store_failed", message)
                            }
                        }
                    }
                    Err(message) => {
                        error(StatusCode::INTERNAL_SERVER_ERROR, "store_failed", message)
                    }
                }
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
        let accepted = !self
            .publish_to_matching(&normalized, connection_id)
            .await?
            .is_empty();
        Ok(AegsConnectionTestEventResponse {
            occurrence_id: normalized.occurrence_id,
            accepted,
            message: (!accepted)
                .then(|| "no active workflow trigger matched the requested test event".to_string()),
        })
    }

    /// Reads the body, verifies the kernel's signature over it, applies the operator's
    /// admission policy and adds the kernel to the owner's set on its first request with the
    /// owner id. Returns who is asking and the verified body.
    async fn authenticate_management(
        &self,
        request: Request<Incoming>,
    ) -> Result<(AuthenticatedKernel, Bytes), Box<Response<Full<Bytes>>>> {
        let (parts, body) = request.into_parts();
        let method = parts.method.to_string();
        let path = parts.uri.path().to_string();
        let body = read_body(Request::from_parts(parts.clone(), body))
            .await
            .map_err(Box::new)?;
        let now = now_ms();
        let kernel = self
            .management_auth
            .authenticate(
                &parts.headers,
                &method,
                &path,
                &self.producer_id,
                &body,
                now,
            )
            .await
            .map_err(refusal_response)?;
        let membership = self
            .store
            .admit_kernel_to_owner(
                &kernel.owner_id,
                &kernel.kernel_id,
                now,
                self.management_auth.max_owners_per_kernel(),
                self.management_auth.max_memberships_total(),
            )
            .map_err(|message| {
                Box::new(error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "store_failed",
                    message,
                ))
            })?;
        match membership {
            OwnerMembership::Member => Ok((kernel, body)),
            OwnerMembership::Revoked => Err(Box::new(error(
                StatusCode::FORBIDDEN,
                "kernel_revoked",
                "this kernel was removed from the owner's kernels",
            ))),
            OwnerMembership::GatewayLimit => Err(Box::new(error(
                StatusCode::SERVICE_UNAVAILABLE,
                "gateway_capacity",
                "the gateway holds the most kernels and owners the operator allows",
            ))),
            OwnerMembership::OwnerLimit => Err(Box::new(error(
                StatusCode::FORBIDDEN,
                "owner_limit_reached",
                "this kernel already holds the most owners the operator allows",
            ))),
        }
    }
}

fn refusal_response(refusal: Refusal) -> Box<Response<Full<Bytes>>> {
    let (status, code) = refusal.status_and_code();
    Box::new(error(status, code, refusal.message()))
}

fn enforce_management_owner(
    authenticated: &AuthenticatedKernel,
    requested_owner: &str,
) -> Result<(), Box<Response<Full<Bytes>>>> {
    if authenticated.owner_id != requested_owner {
        return Err(Box::new(error(
            StatusCode::FORBIDDEN,
            "management_owner_mismatch",
            "the request body names a different owner than the one the kernel signed",
        )));
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

async fn read_body<B>(request: Request<B>) -> Result<Bytes, Response<Full<Bytes>>>
where
    B: hyper::body::Body<Data = Bytes> + Unpin,
{
    if request
        .headers()
        .get(hyper::header::CONTENT_LENGTH)
        .is_some_and(|value| {
            value
                .to_str()
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .is_some_and(|length| length > MAX_WEBHOOK_BYTES as u64)
        })
    {
        return Err(error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload_too_large",
            "request exceeds byte limit",
        ));
    }
    bounded_body(request.into_body(), Duration::from_secs(15)).await
}

async fn bounded_body<B>(mut body: B, timeout: Duration) -> Result<Bytes, Response<Full<Bytes>>>
where
    B: hyper::body::Body<Data = Bytes> + Unpin,
{
    tokio::time::timeout(timeout, async move {
        let mut bytes = bytes::BytesMut::new();
        while let Some(frame) = body.frame().await {
            let frame = frame.map_err(|_| {
                error(
                    StatusCode::BAD_REQUEST,
                    "invalid_body",
                    "request body read failed",
                )
            })?;
            if let Ok(data) = frame.into_data() {
                if data.len() > MAX_WEBHOOK_BYTES.saturating_sub(bytes.len()) {
                    return Err(error(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "payload_too_large",
                        "request exceeds byte limit",
                    ));
                }
                bytes.extend_from_slice(&data);
            }
        }
        Ok(bytes.freeze())
    })
    .await
    .map_err(|_| {
        error(
            StatusCode::REQUEST_TIMEOUT,
            "body_timeout",
            "request body timed out",
        )
    })?
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
    let code = code.into();
    let message = if code.starts_with("provider_")
        || code.starts_with("authorization_")
        || code.starts_with("invalid_provider_")
        || matches!(
            code.as_str(),
            "aeds_rejected"
                | "test_event_failed"
                | "webhook_rejected"
                | "invalid_signature"
                | "resource_query_failed"
        ) {
        "provider operation failed".to_string()
    } else {
        message.into()
    };
    json(
        status,
        serde_json::json!({"error": {"code": code, "message": message}}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chariox_event_protocol::AegsConnectionLifecycleState;

    struct CountedBody {
        frames: std::collections::VecDeque<Bytes>,
        polled: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl hyper::body::Body for CountedBody {
        type Data = Bytes;
        type Error = Infallible;

        fn poll_frame(
            mut self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Result<hyper::body::Frame<Bytes>, Infallible>>> {
            self.polled
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::task::Poll::Ready(
                self.frames
                    .pop_front()
                    .map(|bytes| Ok(hyper::body::Frame::data(bytes))),
            )
        }
    }

    #[tokio::test]
    async fn body_limit_stops_chunked_input_at_the_first_overflow() {
        let polled = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let body = CountedBody {
            frames: [
                Bytes::from(vec![b'x'; MAX_WEBHOOK_BYTES]),
                Bytes::from_static(b"x"),
                Bytes::from_static(b"must not be polled"),
            ]
            .into(),
            polled: polled.clone(),
        };
        let response = read_body(Request::new(body)).await.unwrap_err();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(polled.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn body_limit_rejects_an_oversized_first_frame_without_polling_more() {
        let polled = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let body = CountedBody {
            frames: [
                Bytes::from(vec![b'x'; MAX_WEBHOOK_BYTES + 1]),
                Bytes::from_static(b"must not be polled"),
            ]
            .into(),
            polled: polled.clone(),
        };
        assert_eq!(
            read_body(Request::new(body)).await.unwrap_err().status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(polled.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn body_limit_accepts_empty_and_exact_limit() {
        for length in [0, MAX_WEBHOOK_BYTES] {
            let bytes = Bytes::from(vec![b'x'; length]);
            assert_eq!(
                read_body(Request::new(Full::new(bytes.clone())))
                    .await
                    .unwrap(),
                bytes
            );
        }
    }

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
}

#[cfg(test)]
mod mp11_ingress_tests {
    use super::*;
    struct Frames {
        remaining: usize,
        polled: Arc<std::sync::atomic::AtomicUsize>,
        stall: bool,
    }
    impl hyper::body::Body for Frames {
        type Data = Bytes;
        type Error = Infallible;
        fn poll_frame(
            mut self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Result<hyper::body::Frame<Bytes>, Infallible>>> {
            if self.stall {
                return std::task::Poll::Pending;
            }
            if self.remaining == 0 {
                return std::task::Poll::Ready(None);
            }
            self.remaining -= 1;
            self.polled
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::task::Poll::Ready(Some(Ok(hyper::body::Frame::data(Bytes::from(
                vec![b'x'; 1024 * 1024],
            )))))
        }
    }
    #[tokio::test]
    async fn mp11_ingress_stops_polling_chunked_overflow() {
        let polled = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let result = bounded_body(
            Frames {
                remaining: 100,
                polled: polled.clone(),
                stall: false,
            },
            Duration::from_secs(1),
        )
        .await;
        assert_eq!(result.unwrap_err().status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(polled.load(std::sync::atomic::Ordering::SeqCst), 3);
    }
    #[tokio::test]
    async fn mp11_ingress_rejects_stalled_body() {
        let result = bounded_body(
            Frames {
                remaining: 1,
                polled: Default::default(),
                stall: true,
            },
            Duration::from_millis(20),
        )
        .await;
        assert_eq!(result.unwrap_err().status(), StatusCode::REQUEST_TIMEOUT);
    }
    #[tokio::test]
    async fn mp11_provider_http_error_projection_is_private() {
        let response = error(
            StatusCode::BAD_GATEWAY,
            "authorization_failed",
            "synthetic-client-secret",
        );
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert!(!String::from_utf8_lossy(&bytes).contains("synthetic-client-secret"));
    }
}

#[cfg(test)]
mod mp11_delivery_http_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    struct SignedProvider;
    impl AegsProvider for SignedProvider {
        fn generator_id(&self) -> &'static str {
            "dev.chariox.github"
        }
        fn provider_slug(&self) -> &'static str {
            "github"
        }
        fn normalize_webhook(
            &self,
            input: WebhookInput<'_>,
            _: &crate::WebhookRoute,
        ) -> Result<crate::NormalizedEvent, String> {
            let signature = input
                .headers
                .get("x-signature")
                .ok_or("signature missing")?;
            if !crate::verify_hmac_sha256_hex(input.body, signature, "synthetic-webhook-key") {
                return Err("signature invalid".into());
            }
            Ok(event())
        }
        fn webhook_delivery_id(&self, input: WebhookInput<'_>) -> Option<String> {
            input.headers.get("x-delivery").cloned()
        }
        fn event_subject(&self, _: &crate::NormalizedEvent) -> Option<String> {
            Some("#7".into())
        }
        /// The one scope every fixture connection has.
        fn query_resources(
            &self,
            _: &chariox_event_protocol::AegsProviderResourceQuery,
        ) -> Result<chariox_event_protocol::AegsProviderResourcePage, String> {
            Ok(chariox_event_protocol::AegsProviderResourcePage {
                resources: vec![chariox_event_protocol::AegsProviderResource {
                    id: "1".into(),
                    name: "scope".into(),
                    kind: "repository".into(),
                    connection_scope: "scope".into(),
                    filter: None,
                }],
                next_cursor: None,
            })
        }
    }
    fn event() -> crate::NormalizedEvent {
        crate::NormalizedEvent {
            occurrence_id: "occurrence".into(),
            event_type: "event".into(),
            occurred_at: "2026-10-05T00:00:00Z".into(),
            connection_scope: "scope".into(),
            prompt: "fixture".into(),
            metadata: serde_json::Value::Null,
            reply_context: None,
        }
    }
    // This fake AEDS accepts real HTTP and records calls. Stop/join even on assertion failure.
    struct PublisherFixture {
        url: String,
        calls: Arc<AtomicUsize>,
        accepted: Arc<AtomicUsize>,
        /// Interest keys of the accepted publications, in order.
        interests: Arc<std::sync::Mutex<Vec<String>>>,
        /// The `audience` of each accepted publication, in order (`null` when it had none).
        audiences: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
        refusals: Arc<AtomicUsize>,
        stop: Arc<AtomicBool>,
        hold: Arc<AtomicBool>,
        /// Milliseconds the fake AEDS takes to accept each publication.
        delay_ms: Arc<AtomicU64>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl PublisherFixture {
        fn new() -> Self {
            Self::with_blocked_publication(false)
        }
        fn with_blocked_publication(blocked: bool) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/events", listener.local_addr().unwrap());
            listener.set_nonblocking(true).unwrap();
            let calls = Arc::new(AtomicUsize::new(0));
            let accepted = Arc::new(AtomicUsize::new(0));
            let refusals = Arc::new(AtomicUsize::new(0));
            let interests = Arc::new(std::sync::Mutex::new(Vec::new()));
            let recorded = interests.clone();
            let audiences = Arc::new(std::sync::Mutex::new(Vec::new()));
            let recorded_audiences = audiences.clone();
            let (accepts, refuse) = (accepted.clone(), refusals.clone());
            let stop = Arc::new(AtomicBool::new(false));
            let count = calls.clone();
            let done = stop.clone();
            let hold = Arc::new(AtomicBool::new(blocked));
            let held = hold.clone();
            let delay_ms = Arc::new(AtomicU64::new(0));
            let delay = delay_ms.clone();
            let thread = std::thread::spawn(move || {
                while !done.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream
                                .set_read_timeout(Some(Duration::from_secs(2)))
                                .unwrap();
                            let mut header = Vec::new();
                            while !header.ends_with(b"\r\n\r\n") {
                                let mut byte = [0];
                                stream.read_exact(&mut byte).unwrap();
                                header.push(byte[0]);
                                assert!(header.len() < 8192);
                            }
                            let header = String::from_utf8(header).unwrap();
                            let length: usize = header
                                .lines()
                                .find_map(|line| {
                                    line.split_once(':')
                                        .filter(|(name, _)| {
                                            name.eq_ignore_ascii_case("content-length")
                                        })
                                        .map(|(_, value)| value.trim().parse().unwrap())
                                })
                                .unwrap();
                            assert!(length < 8192);
                            let mut request_body = vec![0; length];
                            stream.read_exact(&mut request_body).unwrap();
                            count.fetch_add(1, Ordering::SeqCst);
                            while held.load(Ordering::SeqCst) && !done.load(Ordering::SeqCst) {
                                std::thread::sleep(Duration::from_millis(2));
                            }
                            if done.load(Ordering::SeqCst) {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(delay.load(Ordering::SeqCst)));
                            // fetch_update: the measured kernel build pins Rust 1.88, where try_update is unstable.
                            #[allow(deprecated)]
                            if refuse
                                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                                    left.checked_sub(1)
                                })
                                .is_ok()
                            {
                                write!(stream, "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                                continue;
                            }
                            accepts.fetch_add(1, Ordering::SeqCst);
                            let request: serde_json::Value =
                                serde_json::from_slice(&request_body).unwrap();
                            recorded
                                .lock()
                                .unwrap()
                                .push(request["event_interest_key"].as_str().unwrap().into());
                            recorded_audiences
                                .lock()
                                .unwrap()
                                .push(request["audience"].clone());
                            let body = r#"{"occurrence_id":"occurrence","accepted_route_count":1,"delivery_ids":[],"duplicate":false}"#;
                            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(2))
                        }
                        Err(error) => panic!("fixture accept: {error}"),
                    }
                }
            });
            Self {
                url,
                calls,
                accepted,
                interests,
                audiences,
                refusals,
                stop,
                hold,
                delay_ms,
                thread: Some(thread),
            }
        }
    }
    impl Drop for PublisherFixture {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            let _ = self.thread.take().unwrap().join();
        }
    }
    async fn signed_webhook(server: Arc<AegsServer>) -> serde_json::Value {
        post_signed_webhook(server, None).await.1
    }
    async fn post_signed_webhook(
        server: Arc<AegsServer>,
        delivery_id: Option<&'static str>,
    ) -> (u16, serde_json::Value) {
        post_signed_webhook_at(server, "/webhooks/github/connection", delivery_id).await
    }
    async fn post_signed_webhook_at(
        server: Arc<AegsServer>,
        path: &'static str,
        delivery_id: Option<&'static str>,
    ) -> (u16, serde_json::Value) {
        use hmac::Mac;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}{path}", listener.local_addr().unwrap());
        let serving = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            http1::Builder::new()
                .keep_alive(false)
                .serve_connection(
                    TokioIo::new(stream),
                    service_fn(move |request| {
                        let server = server.clone();
                        async move { Ok::<_, Infallible>(server.handle(request).await) }
                    }),
                )
                .await
                .unwrap();
        });
        let body = "synthetic-webhook";
        let mut mac =
            <hmac::Hmac<sha2::Sha256> as Mac>::new_from_slice(b"synthetic-webhook-key").unwrap();
        mac.update(body.as_bytes());
        let signature: String = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let response = tokio::task::spawn_blocking(move || {
            let mut request = ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(3))
                .build()
                .post(&url)
                .set("x-signature", &signature)
                .set("connection", "close");
            if let Some(delivery_id) = delivery_id {
                request = request.set("x-delivery", delivery_id);
            }
            match request.send_string(body) {
                Ok(response) => (response.status(), response.into_json().unwrap()),
                Err(ureq::Error::Status(status, response)) => {
                    (status, response.into_json().unwrap_or_default())
                }
                Err(error) => panic!("webhook transport failed: {error}"),
            }
        })
        .await
        .unwrap();
        serving.await.unwrap();
        response
    }
    fn subscription_claim() -> crate::SubscriptionClaim {
        crate::SubscriptionClaim {
            binding_id: "binding".into(),
            generator_id: "dev.chariox.github".into(),
            connection_id: "connection".into(),
            connection_scope: "scope".into(),
            event_interest_key: "interest".into(),
            event_type: "event".into(),
            event_type_version: 1,
            filter: serde_json::Value::Null,
            revision: 1,
            active: true,
        }
    }
    fn subscribed_server(publisher: &PublisherFixture) -> (Arc<AegsServer>, AegsStore) {
        subscribed_server_reconciled_by(publisher, "owner")
    }
    /// The kernel reconciles subscriptions under its own id, not the connection owner's.
    fn subscribed_server_reconciled_by(
        publisher: &PublisherFixture,
        reconciler: &str,
    ) -> (Arc<AegsServer>, AegsStore) {
        let store = AegsStore::open(":memory:").unwrap();
        store
            .upsert_ready_connection("connection", "owner", "github", &serde_json::Value::Null, 1)
            .unwrap();
        let claim = subscription_claim();
        store
            .reconcile_for_owner(
                reconciler,
                "owner",
                &claim.generator_id,
                std::slice::from_ref(&claim),
            )
            .unwrap();
        let server = Arc::new(AegsServer {
            producer_id: claim.generator_id.clone(),
            management_auth: ManagementAuth::open_for_tests(),
            publisher: AedsPublisher::new(&claim.generator_id, None, &publisher.url),
            store: store.clone(),
            provider: Arc::new(SignedProvider),
            scope_cache: Default::default(),
        });
        (server, store)
    }
    fn claim_for(binding_id: &str, active: bool) -> crate::SubscriptionClaim {
        crate::SubscriptionClaim {
            binding_id: binding_id.into(),
            active,
            ..subscription_claim()
        }
    }

    /// AEDS delivers only to the routes the producer names, because anyone can claim an
    /// interest key. The audience is every kernel with an active subscription for the key.
    #[tokio::test]
    async fn publication_names_the_kernels_holding_a_subscription_for_the_interest() {
        let publisher = PublisherFixture::new();
        let (server, store) = subscribed_server_reconciled_by(&publisher, "kernel-1");
        for (kernel, binding, active) in [
            ("kernel-2", "binding-2", true),
            ("kernel-3", "binding-3", false),
        ] {
            store
                .reconcile_for_owner(
                    kernel,
                    "owner",
                    "dev.chariox.github",
                    &[claim_for(binding, active)],
                )
                .unwrap();
        }
        signed_webhook(server).await;
        let audiences = publisher.audiences.lock().unwrap().clone();
        assert_eq!(
            audiences,
            vec![serde_json::json!([
                {"kernel_id": "kernel-1", "binding_id": "binding"},
                {"kernel_id": "kernel-2", "binding_id": "binding-2"},
            ])]
        );
    }

    /// Two users watching one scope share an interest key: each connection is published to on
    /// its own, naming only its own kernels.
    #[tokio::test]
    async fn connections_sharing_an_interest_are_published_to_separately() {
        let publisher = PublisherFixture::new();
        let (server, store) = subscribed_server_reconciled_by(&publisher, "kernel-1");
        store
            .upsert_ready_connection(
                "connection-b",
                "owner-b",
                "github",
                &serde_json::Value::Null,
                1,
            )
            .unwrap();
        let claim_b = crate::SubscriptionClaim {
            binding_id: "binding-b".into(),
            connection_id: "connection-b".into(),
            ..subscription_claim()
        };
        store
            .reconcile_for_owner("kernel-b", "owner-b", "dev.chariox.github", &[claim_b])
            .unwrap();
        let (status, _) = post_signed_webhook_at(server, "/webhooks/github", None).await;
        assert_eq!(status, 202);
        let mut audiences = publisher.audiences.lock().unwrap().clone();
        audiences.sort_by_key(|audience| audience.to_string());
        assert_eq!(
            audiences,
            vec![
                serde_json::json!([{"kernel_id": "kernel-1", "binding_id": "binding"}]),
                serde_json::json!([{"kernel_id": "kernel-b", "binding_id": "binding-b"}]),
            ]
        );
    }

    async fn reconcile_as(
        server: &Arc<AegsServer>,
        kernel: &'static str,
        claims: &[crate::SubscriptionClaim],
    ) -> (u16, serde_json::Value) {
        let body = serde_json::json!({
            "owner_id": kernel,
            "generator_id": "dev.chariox.github",
            "subscriptions": claims,
        });
        management_send("PUT", server, "/v1/subscriptions/reconcile", kernel, body).await
    }

    /// A connection can only watch scopes it has: the interest key of a stranger's repository is
    /// derivable, so a subscription for it would be a way to read that repository's events.
    #[tokio::test]
    async fn reconcile_refuses_a_scope_the_connection_does_not_have() {
        let publisher = PublisherFixture::new();
        let (server, store) = two_owner_server(&publisher);
        let server = Arc::new(AegsServer {
            management_auth: ManagementAuth::open_for_tests(),
            ..(*server).clone()
        });
        let foreign = crate::SubscriptionClaim {
            binding_id: "foreign".into(),
            connection_scope: "stranger/private-repo".into(),
            event_interest_key: "foreign-interest".into(),
            ..subscription_claim()
        };
        let own = claim_for("own", true);
        let (status, body) = reconcile_as(&server, "owner", &[own, foreign]).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["accepted_binding_ids"], serde_json::json!(["own"]));
        assert_eq!(
            body["refused"],
            serde_json::json!([{"binding_id": "foreign", "reason": "scope_not_in_connection"}])
        );
        assert!(store
            .matching("dev.chariox.github", "event", "stranger/private-repo")
            .unwrap()
            .is_empty());
    }

    /// The claim of a connection the AEGS does not know is refused as well.
    #[tokio::test]
    async fn reconcile_refuses_a_connection_the_aegs_does_not_know() {
        let publisher = PublisherFixture::new();
        let (server, _) = two_owner_server(&publisher);
        let unknown = crate::SubscriptionClaim {
            binding_id: "unknown".into(),
            connection_id: "no-such-connection".into(),
            ..subscription_claim()
        };
        let (status, body) = reconcile_as(&server, "owner", &[unknown]).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["accepted_binding_ids"], serde_json::json!([]));
        assert_eq!(
            body["refused"],
            serde_json::json!([{"binding_id": "unknown", "reason": "connection_not_found"}])
        );
    }

    #[tokio::test]
    async fn signed_webhook_is_acknowledged_when_aeds_refuses_the_first_publication() {
        let publisher = PublisherFixture::new();
        publisher.refusals.store(2, Ordering::SeqCst);
        let (server, _) = subscribed_server(&publisher);
        let (status, _) = post_signed_webhook(server, None).await;
        assert_eq!(status, 202);
        assert_eq!(publisher.accepted.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn refused_publication_is_retried_until_aeds_recovers_and_published_once() {
        let publisher = PublisherFixture::new();
        publisher.refusals.store(2, Ordering::SeqCst);
        let (server, store) = subscribed_server(&publisher);
        let (status, body) = post_signed_webhook(server.clone(), Some("delivery-1")).await;
        assert_eq!((status, body["status"].as_str()), (202, Some("pending")));
        assert_eq!(publisher.calls.load(Ordering::SeqCst), 1);
        let stored = store.inbox_event("delivery-1").unwrap().unwrap();
        assert_eq!(stored.status, crate::InboxStatus::Pending);
        assert!(stored.last_error.is_some());

        // Not due yet: the backoff protects a struggling AEDS.
        server
            .deliver_due_inbox(stored.received_at_ms)
            .await
            .unwrap();
        assert_eq!(publisher.calls.load(Ordering::SeqCst), 1);

        let mut at = stored.received_at_ms;
        for _ in 0..4 {
            at += 10 * 60_000;
            server.deliver_due_inbox(at).await.unwrap();
        }
        assert_eq!(publisher.accepted.load(Ordering::SeqCst), 1);
        assert_eq!(publisher.calls.load(Ordering::SeqCst), 3);
        let stored = store.inbox_event("delivery-1").unwrap().unwrap();
        assert_eq!(stored.status, crate::InboxStatus::Published);
        assert_eq!(stored.publication_count, 1);
    }

    #[tokio::test]
    async fn redelivered_webhook_with_a_stored_delivery_id_is_not_published_again() {
        let publisher = PublisherFixture::new();
        let (server, store) = subscribed_server(&publisher);
        let (_, first) = post_signed_webhook(server.clone(), Some("delivery-2")).await;
        assert_eq!(first["matched_interest_count"], 1);
        let (status, again) = post_signed_webhook(server.clone(), Some("delivery-2")).await;
        assert_eq!((status, again["status"].as_str()), (202, Some("published")));
        assert_eq!(publisher.accepted.load(Ordering::SeqCst), 1);
        assert!(store.has_inbox_event("delivery-2").unwrap());
    }

    #[tokio::test]
    async fn exhausted_publication_waits_for_a_replay_with_a_fresh_budget() {
        let publisher = PublisherFixture::new();
        publisher.refusals.store(usize::MAX, Ordering::SeqCst);
        let (server, store) = subscribed_server(&publisher);
        post_signed_webhook(server.clone(), Some("delivery-3")).await;
        let mut at = now_ms();
        for _ in 0..20 {
            at += 11 * 60_000;
            server.deliver_due_inbox(at).await.unwrap();
        }
        let stored = store.inbox_event("delivery-3").unwrap().unwrap();
        assert_eq!(stored.status, crate::InboxStatus::Failed);
        assert_eq!(stored.attempts, 8);
        let calls = publisher.calls.load(Ordering::SeqCst);
        server.deliver_due_inbox(at + 11 * 60_000).await.unwrap();
        assert_eq!(publisher.calls.load(Ordering::SeqCst), calls);

        publisher.refusals.store(0, Ordering::SeqCst);
        let selected = store
            .matching_for_delivery("dev.chariox.github", "event", "scope")
            .await
            .unwrap();
        assert_eq!(
            store
                .requeue_inbox_event("delivery-3", "owner", &selected, at)
                .unwrap(),
            crate::Requeue::Queued
        );
        server.deliver_due_inbox(at).await.unwrap();
        assert_eq!(publisher.accepted.load(Ordering::SeqCst), 1);
        assert_eq!(
            store.inbox_event("delivery-3").unwrap().unwrap().status,
            crate::InboxStatus::Published
        );
    }

    // Stands in for the provider's delivery log (GitHub's app hook deliveries API).
    struct DeliveryLogProvider {
        log: Vec<crate::ProviderDelivery>,
        redelivered: std::sync::Mutex<Vec<String>>,
    }
    impl AegsProvider for DeliveryLogProvider {
        fn generator_id(&self) -> &'static str {
            "dev.chariox.github"
        }
        fn provider_slug(&self) -> &'static str {
            "github"
        }
        fn normalize_webhook(
            &self,
            _: WebhookInput<'_>,
            _: &crate::WebhookRoute,
        ) -> Result<crate::NormalizedEvent, String> {
            Ok(event())
        }
        fn recent_deliveries(&self, since_ms: u64) -> Result<Vec<crate::ProviderDelivery>, String> {
            Ok(self
                .log
                .iter()
                .filter(|delivery| delivery.delivered_at_ms >= since_ms)
                .cloned()
                .collect())
        }
        fn redeliver(&self, delivery: &crate::ProviderDelivery) -> Result<(), String> {
            self.redelivered
                .lock()
                .unwrap()
                .push(delivery.redelivery_ref.clone());
            Ok(())
        }
    }

    #[tokio::test]
    async fn sweep_redelivers_only_recent_deliveries_that_were_never_stored() {
        let publisher = PublisherFixture::new();
        let (server, store) = subscribed_server(&publisher);
        let now = 100 * 60 * 60_000;
        let delivery = |id: &str, age_ms: u64| crate::ProviderDelivery {
            delivery_id: id.into(),
            redelivery_ref: format!("ref-{id}"),
            delivered_at_ms: now - age_ms,
        };
        let provider = Arc::new(DeliveryLogProvider {
            log: vec![
                delivery("stored", 10 * 60_000),
                delivery("missing", 20 * 60_000),
                delivery("in-flight", 30_000),
                delivery("too-old", 7 * 60 * 60_000),
            ],
            redelivered: Default::default(),
        });
        store
            .record_inbox_event("stored", None, &event(), now - 10 * 60_000)
            .unwrap();
        let server = Arc::new(AegsServer {
            provider: provider.clone(),
            ..(*server).clone()
        });
        let mut requested = HashMap::new();
        assert_eq!(
            server
                .sweep_missed_deliveries(now, &mut requested)
                .await
                .unwrap(),
            1
        );
        assert_eq!(*provider.redelivered.lock().unwrap(), vec!["ref-missing"]);

        // Still missing on later sweeps: asked again, but only a few times.
        for _ in 0..5 {
            server
                .sweep_missed_deliveries(now, &mut requested)
                .await
                .unwrap();
        }
        assert_eq!(provider.redelivered.lock().unwrap().len(), 3);

        // Once the redelivery arrives it is stored and the sweep leaves it alone.
        store
            .record_inbox_event("missing", None, &event(), now)
            .unwrap();
        assert_eq!(
            server
                .sweep_missed_deliveries(now, &mut requested)
                .await
                .unwrap(),
            0
        );
    }

    /// The kernel behind a test request is derived from the owner name, so each name is its own
    /// kernel with its own key.
    fn test_kernel(name: &str) -> (String, String) {
        use base64::Engine as _;
        use sha2::Digest as _;
        let secret = p256::SecretKey::from_slice(&sha2::Sha256::digest(name.as_bytes())).unwrap();
        let public = p256::elliptic_curve::sec1::ToEncodedPoint::to_encoded_point(
            &secret.public_key(),
            false,
        );
        (
            base64::engine::general_purpose::STANDARD.encode(secret.to_bytes()),
            base64::engine::general_purpose::STANDARD.encode(public.as_bytes()),
        )
    }

    fn signed_headers(
        method: &str,
        path: &str,
        owner: &str,
        body: &str,
    ) -> Vec<(&'static str, String)> {
        use chariox_event_protocol::kernel_identity as identity;
        static NEXT_NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let (private, public) = test_kernel(owner);
        let nonce = format!("nonce-{}", NEXT_NONCE.fetch_add(1, Ordering::Relaxed));
        let timestamp_ms = now_ms();
        let signature = identity::sign(
            &private,
            &identity::ManagementRequest {
                method,
                path,
                generator_id: "dev.chariox.github",
                owner_id: owner,
                timestamp_ms,
                nonce: &nonce,
                body: body.as_bytes(),
            }
            .message(),
        )
        .unwrap();
        vec![
            (identity::KERNEL_KEY_HEADER, public),
            (identity::KERNEL_TIMESTAMP_HEADER, timestamp_ms.to_string()),
            (identity::KERNEL_NONCE_HEADER, nonce),
            (identity::KERNEL_SIGNATURE_HEADER, signature),
            (identity::OWNER_ID_HEADER, owner.to_string()),
        ]
    }

    async fn management_post(
        server: &Arc<AegsServer>,
        path: &'static str,
        owner: &'static str,
        body: serde_json::Value,
    ) -> (u16, serde_json::Value) {
        management_send("POST", server, path, owner, body).await
    }
    async fn management_send(
        method: &'static str,
        server: &Arc<AegsServer>,
        path: &'static str,
        owner: &'static str,
        body: serde_json::Value,
    ) -> (u16, serde_json::Value) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}{path}", listener.local_addr().unwrap());
        let server = server.clone();
        let serving = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            http1::Builder::new()
                .keep_alive(false)
                .serve_connection(
                    TokioIo::new(stream),
                    service_fn(move |request| {
                        let server = server.clone();
                        async move { Ok::<_, Infallible>(server.handle(request).await) }
                    }),
                )
                .await
                .unwrap();
        });
        let response = tokio::task::spawn_blocking(move || {
            let body = body.to_string();
            let mut request = ureq::request(method, &url).set("connection", "close");
            for (name, value) in signed_headers(method, path, owner, &body) {
                request = request.set(name, &value);
            }
            match request
                .set("content-type", "application/json")
                .send_string(&body)
            {
                Ok(response) => (response.status(), response.into_json().unwrap()),
                Err(ureq::Error::Status(status, response)) => {
                    (status, response.into_json().unwrap_or_default())
                }
                Err(error) => panic!("management transport failed: {error}"),
            }
        })
        .await
        .unwrap();
        serving.await.unwrap();
        response
    }

    #[tokio::test]
    async fn the_gateway_only_generates_events_and_has_no_actions_route() {
        let publisher = PublisherFixture::new();
        let (server, _) = subscribed_server(&publisher);
        let server = Arc::new(AegsServer {
            management_auth: ManagementAuth::open_for_tests(),
            ..(*server).clone()
        });
        let (status, body) = management_post(
            &server,
            "/v1/actions",
            "owner-a",
            serde_json::json!({
                "generator_id": "dev.chariox.github",
                "owner_id": "owner-a",
                "connection_id": "connection-a",
                "action_id": "notification.reply",
                "input": {},
                "context": {},
                "idempotency_key": "reply-1"
            }),
        )
        .await;
        assert_eq!(status, 404, "{body}");
        assert_eq!(body["error"]["code"], "not_found", "{body}");
    }

    #[tokio::test]
    async fn owner_sees_received_events_and_replays_one_that_never_reached_aeds() {
        let publisher = PublisherFixture::new();
        publisher.refusals.store(1, Ordering::SeqCst);
        let (server, _) = subscribed_server(&publisher);
        let server = Arc::new(AegsServer {
            management_auth: ManagementAuth::open_for_tests(),
            ..(*server).clone()
        });
        post_signed_webhook(server.clone(), Some("delivery-4")).await;

        let query = |owner| {
            serde_json::json!({
                "generator_id": "dev.chariox.github", "owner_id": owner, "limit": 10
            })
        };
        let (status, page) =
            management_post(&server, "/v1/events/query", "owner", query("owner")).await;
        assert_eq!(status, 200);
        let events = page["events"].as_array().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["delivery_id"], "delivery-4");
        assert_eq!(events[0]["status"], "pending");
        assert_eq!(events[0]["event_type"], "event");
        assert_eq!(
            events[0]["last_error"],
            "the event service returned HTTP 503"
        );
        assert_eq!(events[0]["connection_scope"], "scope");
        assert_eq!(events[0]["subject"], "#7");
        assert!(events[0].get("prompt").is_none());
        assert!(events[0].get("metadata").is_none());
        let (_, page) =
            management_post(&server, "/v1/events/query", "stranger", query("stranger")).await;
        assert!(page["events"].as_array().unwrap().is_empty());

        let replay = |owner| {
            serde_json::json!({
                "generator_id": "dev.chariox.github", "owner_id": owner, "delivery_id": "delivery-4"
            })
        };
        let (status, _) =
            management_post(&server, "/v1/events/replay", "stranger", replay("stranger")).await;
        assert_eq!(status, 404);
        assert_eq!(publisher.accepted.load(Ordering::SeqCst), 0);
        let (status, replayed) =
            management_post(&server, "/v1/events/replay", "owner", replay("owner")).await;
        assert_eq!(status, 200);
        assert_eq!(replayed["event"]["status"], "published");
        assert_eq!(publisher.accepted.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn replay_publishes_only_to_the_requesting_owners_subscriptions() {
        let publisher = PublisherFixture::new();
        publisher.refusals.store(1, Ordering::SeqCst);
        let (server, store) = subscribed_server(&publisher);
        let server = Arc::new(AegsServer {
            management_auth: ManagementAuth::open_for_tests(),
            ..(*server).clone()
        });
        post_signed_webhook_at(server.clone(), "/webhooks/github", Some("delivery-7")).await;
        assert_eq!(publisher.accepted.load(Ordering::SeqCst), 0);

        // Owner B reauthorizes and subscribes to the same scope after the webhook failed.
        store
            .upsert_ready_connection(
                "connection-b",
                "owner-b",
                "github",
                &serde_json::Value::Null,
                1,
            )
            .unwrap();
        let claim_a = subscription_claim();
        let claim_b = crate::SubscriptionClaim {
            binding_id: "binding-b".into(),
            connection_id: "connection-b".into(),
            event_interest_key: "interest-b".into(),
            ..subscription_claim()
        };
        store
            .reconcile("kernel", "dev.chariox.github", &[claim_a, claim_b])
            .unwrap();

        let replay = serde_json::json!({
            "generator_id": "dev.chariox.github", "owner_id": "owner", "delivery_id": "delivery-7"
        });
        let (status, replayed) =
            management_post(&server, "/v1/events/replay", "owner", replay).await;
        assert_eq!(status, 200, "{replayed}");
        assert_eq!(replayed["event"]["status"], "published");
        assert_eq!(
            *publisher.interests.lock().unwrap(),
            vec!["interest"],
            "owner's replay must not reach another owner's interest"
        );
    }

    #[tokio::test]
    async fn replay_waits_for_an_attempt_that_is_still_in_flight() {
        let publisher = PublisherFixture::new();
        let (server, store) = subscribed_server(&publisher);
        let server = Arc::new(AegsServer {
            management_auth: ManagementAuth::open_for_tests(),
            ..(*server).clone()
        });
        store
            .record_inbox_event("in-flight", None, &event(), now_ms())
            .unwrap();
        let selected = store
            .matching_for_delivery("dev.chariox.github", "event", "scope")
            .await
            .unwrap();
        store
            .select_inbox_interests("in-flight", &selected, now_ms())
            .unwrap();
        // An inline or worker attempt that has been claimed and is awaiting AEDS.
        assert_eq!(
            store
                .claim_all_inbox_deliveries("in-flight", None, now_ms())
                .unwrap()
                .len(),
            1
        );
        let replay = serde_json::json!({
            "generator_id": "dev.chariox.github", "owner_id": "owner", "delivery_id": "in-flight"
        });
        let (status, body) = management_post(&server, "/v1/events/replay", "owner", replay).await;
        assert_eq!(status, 409, "{body}");
        assert_eq!(body["error"]["code"], "received_event_publishing");
        assert_eq!(publisher.calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            store.inbox_event("in-flight").unwrap().unwrap().attempts,
            1,
            "the running attempt keeps its budget"
        );
    }

    fn claim_b() -> crate::SubscriptionClaim {
        crate::SubscriptionClaim {
            binding_id: "binding-b".into(),
            connection_id: "connection-b".into(),
            event_interest_key: "interest-b".into(),
            ..subscription_claim()
        }
    }

    /// Owners A (`owner`, interest `interest`) and B (`owner-b`, interest `interest-b`) both
    /// watch the same scope, reconciled by the kernel in one call.
    fn two_owner_server(publisher: &PublisherFixture) -> (Arc<AegsServer>, AegsStore) {
        let (server, store) = subscribed_server(publisher);
        store
            .upsert_ready_connection(
                "connection-b",
                "owner-b",
                "github",
                &serde_json::Value::Null,
                1,
            )
            .unwrap();
        reconcile_both(&store);
        let server = Arc::new(AegsServer {
            management_auth: ManagementAuth::open_for_tests(),
            ..(*server).clone()
        });
        (server, store)
    }

    fn reconcile_both(store: &AegsStore) {
        store
            .reconcile(
                "kernel",
                "dev.chariox.github",
                &[subscription_claim(), claim_b()],
            )
            .unwrap();
    }

    async fn replay_as(
        server: &Arc<AegsServer>,
        owner: &'static str,
        delivery_id: &str,
    ) -> (u16, serde_json::Value) {
        let replay = serde_json::json!({
            "generator_id": "dev.chariox.github", "owner_id": owner, "delivery_id": delivery_id
        });
        management_post(server, "/v1/events/replay", owner, replay).await
    }

    fn accepted_interests(publisher: &PublisherFixture) -> Vec<String> {
        let mut interests = publisher.interests.lock().unwrap().clone();
        interests.sort();
        interests
    }

    /// Both owners match, publication fails for both, and the webhook is stored pending.
    async fn failed_two_owner_webhook(
        publisher: &PublisherFixture,
        server: &Arc<AegsServer>,
        delivery_id: &'static str,
    ) {
        publisher.refusals.store(2, Ordering::SeqCst);
        let (status, body) =
            post_signed_webhook_at(server.clone(), "/webhooks/github", Some(delivery_id)).await;
        assert_eq!((status, body["status"].as_str()), (202, Some("pending")));
        publisher.refusals.store(0, Ordering::SeqCst);
        assert!(accepted_interests(publisher).is_empty());
    }

    #[tokio::test]
    async fn owner_replay_during_backoff_leaves_the_other_owners_retry_alone() {
        let publisher = PublisherFixture::new();
        let (server, store) = two_owner_server(&publisher);
        failed_two_owner_webhook(&publisher, &server, "matrix-1").await;
        let before = store.inbox_deliveries("matrix-1").unwrap();

        let (status, replayed) = replay_as(&server, "owner", "matrix-1").await;
        assert_eq!(status, 200, "{replayed}");
        assert_eq!(accepted_interests(&publisher), vec!["interest"]);
        assert_eq!(
            replayed["event"]["status"], "published",
            "A's response folds A's rows only; B's backoff is B's"
        );
        assert_eq!(
            store.inbox_event("matrix-1").unwrap().unwrap().status,
            crate::InboxStatus::Pending,
            "the retry worker still sees B owed"
        );
        let after = store.inbox_deliveries("matrix-1").unwrap();
        assert_eq!(after[1], before[1], "B keeps its attempts, timer and error");

        // B's own retry still delivers, once, and replaying A again adds nothing for B.
        server
            .deliver_due_inbox(now_ms() + 60 * 60_000)
            .await
            .unwrap();
        assert_eq!(
            accepted_interests(&publisher),
            vec!["interest", "interest-b"]
        );
        assert_eq!(
            store.inbox_event("matrix-1").unwrap().unwrap().status,
            crate::InboxStatus::Published
        );
        server
            .deliver_due_inbox(now_ms() + 2 * 60 * 60_000)
            .await
            .unwrap();
        assert_eq!(
            accepted_interests(&publisher),
            vec!["interest", "interest-b"]
        );
    }

    #[tokio::test]
    async fn revoked_owner_is_fenced_while_the_replaying_owner_is_delivered_once() {
        let publisher = PublisherFixture::new();
        let (server, store) = two_owner_server(&publisher);
        failed_two_owner_webhook(&publisher, &server, "matrix-2").await;

        assert!(store
            .revoke_connection("connection-b", "owner-b", now_ms())
            .unwrap());
        let (status, replayed) = replay_as(&server, "owner", "matrix-2").await;
        assert_eq!(status, 200, "{replayed}");
        server
            .deliver_due_inbox(now_ms() + 60 * 60_000)
            .await
            .unwrap();

        assert_eq!(accepted_interests(&publisher), vec!["interest"]);
        let rows = store.inbox_deliveries("matrix-2").unwrap();
        assert_eq!(rows[0].status, crate::DeliveryStatus::Published);
        assert_eq!(rows[1].status, crate::DeliveryStatus::Fenced);
        assert_eq!(
            store.inbox_event("matrix-2").unwrap().unwrap().status,
            crate::InboxStatus::Published
        );
        assert!(store
            .inbox_event_for_owner("owner-b", "matrix-2")
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn reauthorized_owner_is_fenced_and_the_replaying_owner_is_delivered_once() {
        let publisher = PublisherFixture::new();
        let (server, store) = two_owner_server(&publisher);
        failed_two_owner_webhook(&publisher, &server, "matrix-3").await;

        store
            .upsert_ready_connection(
                "connection-b",
                "owner-b",
                "github",
                &serde_json::Value::Null,
                2,
            )
            .unwrap();
        reconcile_both(&store);
        let (status, replayed) = replay_as(&server, "owner", "matrix-3").await;
        assert_eq!(status, 200, "{replayed}");
        server
            .deliver_due_inbox(now_ms() + 60 * 60_000)
            .await
            .unwrap();

        assert_eq!(accepted_interests(&publisher), vec!["interest"]);
        let rows = store.inbox_deliveries("matrix-3").unwrap();
        assert_eq!(rows[0].status, crate::DeliveryStatus::Published);
        assert_eq!(rows[1].status, crate::DeliveryStatus::Fenced);

        // B asking for its own replay now publishes under the new authorization.
        let (status, replayed) = replay_as(&server, "owner-b", "matrix-3").await;
        assert_eq!(status, 200, "{replayed}");
        assert_eq!(
            accepted_interests(&publisher),
            vec!["interest", "interest-b"]
        );
    }

    #[tokio::test]
    async fn owner_replay_does_not_interrupt_the_other_owners_attempt_in_flight() {
        let publisher = PublisherFixture::new();
        let (server, store) = two_owner_server(&publisher);
        failed_two_owner_webhook(&publisher, &server, "matrix-4").await;

        // B's retry has been claimed and is awaiting AEDS when A asks for a replay.
        let due = now_ms() + 60_000;
        let in_flight = store
            .claim_all_inbox_deliveries("matrix-4", Some("owner-b"), due)
            .unwrap();
        assert_eq!(in_flight.len(), 1);
        let (status, replayed) = replay_as(&server, "owner", "matrix-4").await;
        assert_eq!(status, 200, "{replayed}");
        assert_eq!(accepted_interests(&publisher), vec!["interest"]);

        // B's attempt was not superseded: it still finishes the row it claimed.
        assert!(store.finish_inbox_delivery(&in_flight[0], due).unwrap());
        let rows = store.inbox_deliveries("matrix-4").unwrap();
        assert_eq!(rows[0].status, crate::DeliveryStatus::Published);
        assert_eq!(rows[1].status, crate::DeliveryStatus::Published);
        // And B's own replay is refused only while its attempt is running.
        let (status, _) = replay_as(&server, "owner-b", "matrix-4").await;
        assert_eq!(status, 200);
        assert_eq!(
            accepted_interests(&publisher),
            vec!["interest", "interest-b"]
        );
    }

    #[tokio::test]
    async fn unrouted_webhook_belongs_to_the_connection_owner_not_the_reconciling_kernel() {
        let publisher = PublisherFixture::new();
        publisher.refusals.store(1, Ordering::SeqCst);
        let (server, _) = subscribed_server_reconciled_by(&publisher, "kernel-1");
        let server = Arc::new(AegsServer {
            management_auth: ManagementAuth::open_for_tests(),
            ..(*server).clone()
        });
        post_signed_webhook_at(server.clone(), "/webhooks/github", Some("delivery-5")).await;

        let query = |owner| {
            serde_json::json!({
                "generator_id": "dev.chariox.github", "owner_id": owner, "limit": 10
            })
        };
        let (_, page) = management_post(&server, "/v1/events/query", "owner", query("owner")).await;
        assert_eq!(page["events"].as_array().unwrap().len(), 1);
        let (_, page) =
            management_post(&server, "/v1/events/query", "kernel-1", query("kernel-1")).await;
        assert!(page["events"].as_array().unwrap().is_empty());

        let replay = |owner| {
            serde_json::json!({
                "generator_id": "dev.chariox.github", "owner_id": owner, "delivery_id": "delivery-5"
            })
        };
        let (status, _) =
            management_post(&server, "/v1/events/replay", "kernel-1", replay("kernel-1")).await;
        assert_eq!(status, 404);
        let (status, replayed) =
            management_post(&server, "/v1/events/replay", "owner", replay("owner")).await;
        assert_eq!(status, 200);
        assert_eq!(replayed["event"]["status"], "published");
    }

    #[tokio::test]
    async fn retry_keeps_the_authorization_selected_when_the_webhook_arrived() {
        let publisher = PublisherFixture::new();
        publisher.refusals.store(1, Ordering::SeqCst);
        let (server, store) = subscribed_server(&publisher);
        let (_, body) = post_signed_webhook(server.clone(), Some("delivery-6")).await;
        assert_eq!(body["status"], "pending");

        // The connection is reauthorized and reconciled before the retry runs.
        store
            .upsert_ready_connection("connection", "owner", "github", &serde_json::Value::Null, 2)
            .unwrap();
        let claim = subscription_claim();
        store
            .reconcile_for_owner(
                "owner",
                "owner",
                &claim.generator_id,
                std::slice::from_ref(&claim),
            )
            .unwrap();
        server
            .deliver_due_inbox(now_ms() + 60 * 60_000)
            .await
            .unwrap();

        assert_eq!(publisher.accepted.load(Ordering::SeqCst), 0);
        let entry = store.inbox_event("delivery-6").unwrap().unwrap();
        assert_eq!(entry.status, crate::InboxStatus::Unmatched);
    }

    #[tokio::test]
    async fn stalled_aeds_does_not_stall_the_retry_worker() {
        let publisher = PublisherFixture::with_blocked_publication(true);
        let (server, store) = subscribed_server(&publisher);
        let server = Arc::new(AegsServer {
            publisher: server
                .publisher
                .clone()
                .with_request_timeout(Duration::from_millis(200)),
            ..(*server).clone()
        });
        for delivery in ["first", "second"] {
            store
                .record_inbox_event(delivery, None, &event(), now_ms())
                .unwrap();
        }
        tokio::time::timeout(
            Duration::from_secs(5),
            server.deliver_due_inbox(now_ms() + 1_000),
        )
        .await
        .expect("the retry worker finished its pass")
        .unwrap();
        for delivery in ["first", "second"] {
            let entry = store.inbox_event(delivery).unwrap().unwrap();
            assert_eq!(entry.attempts, 1);
            assert!(entry.last_error.is_some(), "{delivery} records the failure");
        }
    }

    #[tokio::test]
    async fn mp11_review_oauth_completion_survives_blocked_webhook_publisher() {
        let publisher = PublisherFixture::with_blocked_publication(true);
        let store = AegsStore::open(":memory:").unwrap();
        store
            .upsert_ready_connection("connection", "owner", "github", &serde_json::Value::Null, 1)
            .unwrap();
        let claim = crate::SubscriptionClaim {
            binding_id: "binding".into(),
            generator_id: "dev.chariox.github".into(),
            connection_id: "connection".into(),
            connection_scope: "scope".into(),
            event_interest_key: "interest".into(),
            event_type: "event".into(),
            event_type_version: 1,
            filter: serde_json::Value::Null,
            revision: 1,
            active: true,
        };
        store
            .reconcile("owner", &claim.generator_id, std::slice::from_ref(&claim))
            .unwrap();
        store
            .create_authorization(crate::store::CreateAuthorizationRequest {
                state_digest: "pending-state",
                connection_id: "pending",
                owner_id: "owner",
                provider: "github",
                return_url: None,
                expires_at_ms: 1000,
                now_ms: 1,
            })
            .unwrap();
        let server = Arc::new(AegsServer {
            producer_id: claim.generator_id.clone(),
            management_auth: ManagementAuth::open_for_tests(),
            publisher: AedsPublisher::new(&claim.generator_id, None, &publisher.url),
            store: store.clone(),
            provider: Arc::new(SignedProvider),
            scope_cache: Default::default(),
        });

        let publishing = tokio::spawn(signed_webhook(server));
        tokio::time::timeout(Duration::from_secs(2), async {
            while publisher.calls.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
        assert!(!publishing.is_finished());
        let completion_store = store.clone();
        let completion = tokio::task::spawn_blocking(move || {
            completion_store.complete_authorization(
                "pending-state",
                b"synthetic-encrypted-exchange",
                &serde_json::Value::Null,
                None,
                2,
            )
        });
        let result = tokio::time::timeout(Duration::from_secs(2), completion).await;
        // Release the real publisher even when the assertion below fails.
        publisher.hold.store(false, Ordering::SeqCst);
        let ready = result.unwrap().unwrap().unwrap();
        assert_eq!(ready.status, "ready");
        assert_eq!(
            ready.encrypted_credential.as_deref(),
            Some(b"synthetic-encrypted-exchange".as_slice())
        );
        assert_eq!(publishing.await.unwrap()["matched_interest_count"], 1);
    }

    #[tokio::test]
    async fn mp11_signed_webhook_delivers_shared_interest_after_first_binding_retires() {
        let publisher = PublisherFixture::with_blocked_publication(true);
        let store = AegsStore::open(":memory:").unwrap();
        store
            .upsert_ready_connection("connection", "owner", "github", &serde_json::Value::Null, 1)
            .unwrap();
        let earlier = crate::SubscriptionClaim {
            binding_id: "00-earlier".into(),
            generator_id: "dev.chariox.github".into(),
            connection_id: "connection".into(),
            connection_scope: "scope".into(),
            event_interest_key: "earlier-interest".into(),
            event_type: "event".into(),
            event_type_version: 1,
            filter: serde_json::Value::Null,
            revision: 1,
            active: true,
        };
        let first = crate::SubscriptionClaim {
            binding_id: "10-first".into(),
            event_interest_key: "shared-interest".into(),
            ..earlier.clone()
        };
        let remaining = crate::SubscriptionClaim {
            binding_id: "20-remaining".into(),
            ..first.clone()
        };
        let allowed_owners = ["owner".to_string()];
        store
            .reconcile_for_owner(
                "kernel",
                &allowed_owners[0],
                &earlier.generator_id,
                &[earlier.clone(), first, remaining.clone()],
            )
            .unwrap();
        let server = Arc::new(AegsServer {
            producer_id: earlier.generator_id.clone(),
            management_auth: ManagementAuth::open_for_tests(),
            publisher: AedsPublisher::new(&earlier.generator_id, None, &publisher.url),
            store: store.clone(),
            provider: Arc::new(SignedProvider),
            scope_cache: Default::default(),
        });
        let publishing = tokio::spawn(signed_webhook(server));
        tokio::time::timeout(Duration::from_secs(2), async {
            while publisher.calls.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
        assert!(!publishing.is_finished());
        // All three bindings have been selected. Retire only the first shared
        // binding while the earlier interest is waiting for its HTTP response.
        let reconciliation = store.reconcile_for_owner(
            "kernel",
            &allowed_owners[0],
            &earlier.generator_id,
            &[earlier.clone(), remaining.clone()],
        );
        publisher.hold.store(false, Ordering::SeqCst);
        reconciliation.unwrap();
        let response = publishing.await.unwrap();
        assert_eq!(
            store
                .matching(
                    &earlier.generator_id,
                    &earlier.event_type,
                    &earlier.connection_scope
                )
                .unwrap(),
            vec![earlier, remaining]
        );
        assert_eq!(publisher.calls.load(Ordering::SeqCst), 2);
        assert_eq!(response["matched_interest_count"], 2);
        assert_eq!(response["publications"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn mp11_signed_late_webhook_and_selected_test_event_do_not_publish_after_revoke() {
        let publisher = PublisherFixture::new();
        let store = AegsStore::open(":memory:").unwrap();
        store
            .upsert_ready_connection("connection", "owner", "github", &serde_json::Value::Null, 1)
            .unwrap();
        let claim = crate::SubscriptionClaim {
            binding_id: "binding".into(),
            generator_id: "dev.chariox.github".into(),
            connection_id: "connection".into(),
            connection_scope: "scope".into(),
            event_interest_key: "interest".into(),
            event_type: "event".into(),
            event_type_version: 1,
            filter: serde_json::Value::Null,
            revision: 1,
            active: true,
        };
        store
            .reconcile("owner", &claim.generator_id, &[claim.clone()])
            .unwrap();
        let server = Arc::new(AegsServer {
            producer_id: claim.generator_id.clone(),
            management_auth: ManagementAuth::open_for_tests(),
            publisher: AedsPublisher::new(&claim.generator_id, None, &publisher.url),
            store: store.clone(),
            provider: Arc::new(SignedProvider),
            scope_cache: Default::default(),
        });
        assert_eq!(
            signed_webhook(server.clone()).await["matched_interest_count"],
            1
        );
        assert_eq!(publisher.calls.load(Ordering::SeqCst), 1);
        let selected_test_event = event();
        store.revoke_connection("connection", "owner", 2).unwrap();
        assert_eq!(
            signed_webhook(server.clone()).await["matched_interest_count"],
            0
        );
        assert!(
            !server
                .publish_normalized_event(selected_test_event, Some("connection"))
                .await
                .unwrap()
                .accepted
        );
        assert_eq!(publisher.calls.load(Ordering::SeqCst), 1);
        store
            .upsert_ready_connection("connection", "owner", "github", &serde_json::Value::Null, 3)
            .unwrap();
        assert_eq!(
            signed_webhook(server.clone()).await["matched_interest_count"],
            0
        );
        store
            .reconcile("owner", &claim.generator_id, std::slice::from_ref(&claim))
            .unwrap();
        assert_eq!(signed_webhook(server).await["matched_interest_count"], 1);
        assert_eq!(publisher.calls.load(Ordering::SeqCst), 2);
    }

    /// A delivery is finished: further retry sweeps make no AEDS call, change nothing and leave
    /// no row waiting.
    async fn assert_settled(
        server: &Arc<AegsServer>,
        store: &AegsStore,
        publisher: &PublisherFixture,
        delivery_id: &str,
    ) {
        let calls = publisher.calls.load(Ordering::SeqCst);
        let before = store.inbox_event(delivery_id).unwrap().unwrap();
        assert_eq!(before.next_attempt_at_ms, None);
        let mut at = now_ms();
        for _ in 0..12 {
            at += 11 * 60_000;
            server.deliver_due_inbox(at).await.unwrap();
        }
        assert_eq!(publisher.calls.load(Ordering::SeqCst), calls, "no retry");
        assert_eq!(store.inbox_event(delivery_id).unwrap().unwrap(), before);
        for row in store.inbox_deliveries(delivery_id).unwrap() {
            assert!(
                matches!(
                    row.status,
                    crate::DeliveryStatus::Published | crate::DeliveryStatus::Fenced
                ),
                "{} is still {:?}",
                row.connection_id,
                row.status
            );
        }
    }

    fn server_without_connection_rows(
        publisher: &PublisherFixture,
    ) -> (Arc<AegsServer>, AegsStore) {
        let (server, _) = subscribed_server(publisher);
        let store = AegsStore::open(":memory:").unwrap();
        store
            .reconcile("kernel", "dev.chariox.github", &[subscription_claim()])
            .unwrap();
        let server = Arc::new(AegsServer {
            store: store.clone(),
            management_auth: ManagementAuth::open_for_tests(),
            ..(*server).clone()
        });
        (server, store)
    }

    async fn owner_events(server: &Arc<AegsServer>, owner: &'static str) -> serde_json::Value {
        let query = serde_json::json!({
            "generator_id": "dev.chariox.github", "owner_id": owner, "limit": 10
        });
        management_post(server, "/v1/events/query", owner, query)
            .await
            .1
    }

    #[tokio::test]
    async fn connection_owned_webhook_is_published_once_and_never_retried() {
        let publisher = PublisherFixture::new();
        let (server, store) = subscribed_server(&publisher);
        let server = Arc::new(AegsServer {
            management_auth: ManagementAuth::open_for_tests(),
            ..(*server).clone()
        });
        let (status, body) = post_signed_webhook(server.clone(), Some("settled-1")).await;
        assert_eq!((status, &body["matched_interest_count"]), (202, &1.into()));
        assert_eq!(publisher.accepted.load(Ordering::SeqCst), 1);
        assert_eq!(
            owner_events(&server, "owner").await["events"][0]["status"],
            "published"
        );
        assert_settled(&server, &store, &publisher, "settled-1").await;
        let (_, again) = post_signed_webhook(server.clone(), Some("settled-1")).await;
        assert_eq!(again["status"], "published");
        assert_eq!(publisher.calls.load(Ordering::SeqCst), 1);
        let connection = store.connection("connection").unwrap().unwrap();
        assert!(connection.last_accepted_event_at_ms.is_some());
    }

    fn claim_c() -> crate::SubscriptionClaim {
        crate::SubscriptionClaim {
            binding_id: "binding-c".into(),
            connection_id: "connection-c".into(),
            event_interest_key: "interest-c".into(),
            ..subscription_claim()
        }
    }

    async fn wait_for_calls(publisher: &PublisherFixture, calls: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while publisher.calls.load(Ordering::SeqCst) < calls {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .expect("the fake AEDS received the publication");
    }

    #[tokio::test]
    async fn an_interest_is_leased_when_its_publication_starts_not_when_the_event_is_picked_up() {
        let publisher = PublisherFixture::new();
        let (server, store) = two_owner_server(&publisher);
        store
            .upsert_ready_connection(
                "connection-c",
                "owner-c",
                "github",
                &serde_json::Value::Null,
                1,
            )
            .unwrap();
        store
            .reconcile(
                "kernel",
                "dev.chariox.github",
                &[subscription_claim(), claim_b(), claim_c()],
            )
            .unwrap();
        // Three publications of 200 ms against a 300 ms lease: the third starts after a lease
        // taken at the start of the event would have ended.
        publisher.delay_ms.store(200, Ordering::SeqCst);
        let at = now_ms();
        store
            .record_inbox_event("slow-1", None, &event(), at)
            .unwrap();
        let inline = tokio::spawn({
            let server = server.clone();
            async move { server.attempt_inbox_delivery("slow-1", None, at).await }
        });
        wait_for_calls(&publisher, 3).await;
        // The retry worker passes while the third interest is awaiting AEDS.
        server.deliver_due_inbox(now_ms()).await.unwrap();
        inline.await.unwrap().unwrap();
        assert_eq!(
            accepted_interests(&publisher),
            vec!["interest", "interest-b", "interest-c"]
        );
        let rows = store.inbox_deliveries("slow-1").unwrap();
        assert!(rows
            .iter()
            .all(|row| row.status == crate::DeliveryStatus::Published && row.attempts == 1));
    }

    #[tokio::test]
    async fn the_retry_worker_leases_each_event_when_its_own_publication_starts() {
        let publisher = PublisherFixture::new();
        let (server, store) = subscribed_server(&publisher);
        publisher.delay_ms.store(200, Ordering::SeqCst);
        let at = now_ms();
        let deliveries = ["batch-1", "batch-2", "batch-3"];
        for delivery in deliveries {
            store
                .record_inbox_event(delivery, None, &event(), at)
                .unwrap();
        }
        let first_pass = tokio::spawn({
            let server = server.clone();
            async move { server.deliver_due_inbox(at).await }
        });
        wait_for_calls(&publisher, 3).await;
        // A second pass while the third event is awaiting AEDS: its lease began with its own
        // attempt, not with the first pass.
        server.deliver_due_inbox(now_ms()).await.unwrap();
        first_pass.await.unwrap().unwrap();
        assert_eq!(accepted_interests(&publisher).len(), 3);
        for delivery in deliveries {
            let entry = store.inbox_event(delivery).unwrap().unwrap();
            assert_eq!(
                (entry.status, entry.attempts, entry.publication_count),
                (crate::InboxStatus::Published, 1, 1),
                "{delivery}"
            );
        }
    }

    #[tokio::test]
    async fn attempts_that_keep_crashing_end_in_failed_with_a_lease_reason_and_can_be_replayed() {
        let publisher = PublisherFixture::new();
        let (server, store) = subscribed_server(&publisher);
        let server = Arc::new(AegsServer {
            management_auth: ManagementAuth::open_for_tests(),
            ..(*server).clone()
        });
        let mut at = now_ms();
        store
            .record_inbox_event("crashy", None, &event(), at)
            .unwrap();
        let selected = store
            .matching_for_delivery("dev.chariox.github", "event", "scope")
            .await
            .unwrap();
        store
            .select_inbox_interests("crashy", &selected, at)
            .unwrap();
        // Each attempt is claimed and its process dies before reporting back.
        for attempt in 1..=8 {
            let claimed = store
                .claim_all_inbox_deliveries("crashy", None, at)
                .unwrap();
            assert_eq!(claimed.len(), 1, "attempt {attempt} is within the budget");
            at += 60 * 60_000;
        }
        server.deliver_due_inbox(at).await.unwrap();
        assert_eq!(
            publisher.calls.load(Ordering::SeqCst),
            0,
            "no ninth attempt"
        );
        let stored = store.inbox_event("crashy").unwrap().unwrap();
        assert_eq!(stored.status, crate::InboxStatus::Failed);
        assert_eq!(stored.attempts, 8);
        assert!(
            stored
                .last_error
                .as_deref()
                .unwrap_or_default()
                .contains("lease expired"),
            "{:?}",
            stored.last_error
        );
        let (_, page) = management_post(
            &server,
            "/v1/events/query",
            "owner",
            serde_json::json!({"generator_id": "dev.chariox.github", "owner_id": "owner", "limit": 10}),
        )
        .await;
        assert_eq!(page["events"][0]["status"], "failed");
        let (status, replayed) = replay_as(&server, "owner", "crashy").await;
        assert_eq!(status, 200, "{replayed}");
        assert_eq!(replayed["event"]["status"], "published");
        assert_eq!(publisher.accepted.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn failed_event_of_a_legacy_subscription_is_listed_and_replayed_by_its_owner_only() {
        let publisher = PublisherFixture::new();
        publisher.refusals.store(usize::MAX, Ordering::SeqCst);
        let (server, store) = server_without_connection_rows(&publisher);
        post_signed_webhook_at(server.clone(), "/webhooks/github", Some("legacy-1")).await;
        let mut at = now_ms();
        for _ in 0..20 {
            at += 11 * 60_000;
            server.deliver_due_inbox(at).await.unwrap();
        }
        assert_eq!(
            store.inbox_event("legacy-1").unwrap().unwrap().status,
            crate::InboxStatus::Failed
        );
        publisher.refusals.store(0, Ordering::SeqCst);

        let listed = owner_events(&server, "kernel").await;
        assert_eq!(listed["events"][0]["delivery_id"], "legacy-1");
        assert_eq!(listed["events"][0]["status"], "failed");
        assert!(owner_events(&server, "stranger").await["events"]
            .as_array()
            .unwrap()
            .is_empty());
        let (status, _) = replay_as(&server, "stranger", "legacy-1").await;
        assert_eq!(status, 404);
        assert_eq!(publisher.accepted.load(Ordering::SeqCst), 0);

        let (status, replayed) = replay_as(&server, "kernel", "legacy-1").await;
        assert_eq!(status, 200, "{replayed}");
        assert_eq!(replayed["event"]["status"], "published");
        assert_eq!(accepted_interests(&publisher), vec!["interest"]);
    }

    #[tokio::test]
    async fn legacy_subscription_without_a_connection_is_published_once_and_never_retried() {
        let publisher = PublisherFixture::new();
        let (server, store) = server_without_connection_rows(&publisher);
        let (status, body) =
            post_signed_webhook_at(server.clone(), "/webhooks/github", Some("settled-2")).await;
        assert_eq!((status, &body["matched_interest_count"]), (202, &1.into()));
        assert_eq!(publisher.accepted.load(Ordering::SeqCst), 1);
        let rows = store.inbox_deliveries("settled-2").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, crate::DeliveryStatus::Published);
        assert_eq!(rows[0].attempts, 1);
        assert_eq!(rows[0].last_error, None);
        assert_settled(&server, &store, &publisher, "settled-2").await;
        assert_eq!(publisher.accepted.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn two_owners_sharing_an_interest_are_each_published_once_and_never_retried() {
        let publisher = PublisherFixture::new();
        let (server, store) = subscribed_server(&publisher);
        store
            .upsert_ready_connection(
                "connection-b",
                "owner-b",
                "github",
                &serde_json::Value::Null,
                1,
            )
            .unwrap();
        let shared_b = crate::SubscriptionClaim {
            event_interest_key: "interest".into(),
            ..claim_b()
        };
        store
            .reconcile(
                "kernel",
                "dev.chariox.github",
                &[subscription_claim(), shared_b],
            )
            .unwrap();
        let (status, body) =
            post_signed_webhook_at(server.clone(), "/webhooks/github", Some("settled-3")).await;
        assert_eq!((status, &body["matched_interest_count"]), (202, &2.into()));
        assert_eq!(accepted_interests(&publisher), vec!["interest", "interest"]);
        let rows = store.inbox_deliveries("settled-3").unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows
            .iter()
            .all(|row| row.status == crate::DeliveryStatus::Published && row.attempts == 1));
        assert_settled(&server, &store, &publisher, "settled-3").await;
    }

    #[tokio::test]
    async fn revocation_waits_for_the_attempt_in_flight_and_the_event_is_published_once() {
        let publisher = PublisherFixture::with_blocked_publication(true);
        let (server, store) = subscribed_server(&publisher);
        let receiving = tokio::spawn(post_signed_webhook(server.clone(), Some("settled-4")));
        tokio::time::timeout(Duration::from_secs(2), async {
            while publisher.calls.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
        assert!(store
            .revoke_connection("connection", "owner", now_ms())
            .is_err());
        publisher.hold.store(false, Ordering::SeqCst);
        let (status, body) = receiving.await.unwrap();
        assert_eq!((status, &body["matched_interest_count"]), (202, &1.into()));
        assert!(store
            .revoke_connection("connection", "owner", now_ms())
            .unwrap());
        assert_eq!(publisher.accepted.load(Ordering::SeqCst), 1);
        assert_settled(&server, &store, &publisher, "settled-4").await;
    }

    #[tokio::test]
    async fn revoked_before_the_retry_the_event_is_fenced_not_published_and_not_retried() {
        let publisher = PublisherFixture::new();
        publisher.refusals.store(1, Ordering::SeqCst);
        let (server, store) = subscribed_server(&publisher);
        let (_, body) = post_signed_webhook(server.clone(), Some("settled-5")).await;
        assert_eq!(body["status"], "pending");
        assert!(store
            .revoke_connection("connection", "owner", now_ms())
            .unwrap());
        server
            .deliver_due_inbox(now_ms() + 60 * 60_000)
            .await
            .unwrap();
        assert_eq!(publisher.accepted.load(Ordering::SeqCst), 0);
        let rows = store.inbox_deliveries("settled-5").unwrap();
        assert_eq!(rows[0].status, crate::DeliveryStatus::Fenced);
        assert_settled(&server, &store, &publisher, "settled-5").await;
        assert_eq!(publisher.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn replayed_event_is_published_once_by_the_replay_and_never_retried() {
        let publisher = PublisherFixture::new();
        publisher.refusals.store(1, Ordering::SeqCst);
        let (server, store) = subscribed_server(&publisher);
        let server = Arc::new(AegsServer {
            management_auth: ManagementAuth::open_for_tests(),
            ..(*server).clone()
        });
        let (_, body) = post_signed_webhook(server.clone(), Some("settled-6")).await;
        assert_eq!(body["status"], "pending");
        let (status, replayed) = replay_as(&server, "owner", "settled-6").await;
        assert_eq!(status, 200, "{replayed}");
        assert_eq!(replayed["event"]["status"], "published");
        assert_eq!(publisher.accepted.load(Ordering::SeqCst), 1);
        assert_settled(&server, &store, &publisher, "settled-6").await;
        assert_eq!(publisher.calls.load(Ordering::SeqCst), 2);
    }
}
