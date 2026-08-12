use super::*;
use sha2::{Digest, Sha256};

#[test]
fn local_daemon_protocol_event_publication_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 252);
    let requests = vec![
        LocalDaemonRequest::GetEventGeneratorCatalogLanding(
            crate::local::GetEventGeneratorCatalogLandingRequest { limit: 12 },
        ),
        LocalDaemonRequest::SearchEventGeneratorCatalog(
            crate::local::SearchEventGeneratorCatalogRequest {
                query: "github".to_string(),
                category: Some("Developer tools".to_string()),
                verification: Some("arroba".to_string()),
                cursor: Some("opaque-cursor".to_string()),
                limit: 20,
            },
        ),
        LocalDaemonRequest::BrowseEventGeneratorCategory(
            crate::local::BrowseEventGeneratorCategoryRequest {
                category: "Observability".to_string(),
                cursor: None,
                limit: 20,
            },
        ),
        LocalDaemonRequest::GetEventGeneratorDetail(crate::local::GetEventGeneratorDetailRequest {
            generator_id: "dev.arroba.github".to_string(),
            version: Some("1.0.0".to_string()),
        }),
        LocalDaemonRequest::BrowseEventGeneratorEvents(
            crate::local::BrowseEventGeneratorEventsRequest {
                generator_id: "dev.arroba.github".to_string(),
                query: Some("review".to_string()),
                cursor: Some("opaque-event-cursor".to_string()),
                limit: 50,
            },
        ),
        LocalDaemonRequest::CreateWorkflowEventBinding(
            crate::local::CreateWorkflowEventBindingRequest {
                session_id: "session-1".to_string(),
                publication_ref: "publication-1".to_string(),
                generator_id: "dev.arroba.github".to_string(),
                generator_version: "1.0.0".to_string(),
                manifest_digest: format!("sha256:{}", "a".repeat(64)),
                connection_id: "connection-1".to_string(),
                connection_scope: "installation:1".to_string(),
                event_type: "pull_request.opened".to_string(),
                event_type_version: 1,
                filter: serde_json::json!({"repository": "arroba"}),
                environment_id: Some("environment-1".to_string()),
                queue_ref: Some("priority".to_string()),
            },
        ),
        LocalDaemonRequest::ListWorkflowEventBindings(
            crate::local::ListWorkflowEventBindingsRequest {
                session_id: "session-1".to_string(),
                publication_ref: Some("publication-1".to_string()),
            },
        ),
        LocalDaemonRequest::SetWorkflowEventBindingStatus(
            crate::local::SetWorkflowEventBindingStatusRequest {
                session_id: "session-1".to_string(),
                binding_id: "binding-1".to_string(),
                status: crate::session::WorkflowEventBindingStatus::Paused,
            },
        ),
        LocalDaemonRequest::TransferWorkflowEventBinding(
            crate::local::TransferWorkflowEventBindingRequest {
                source_session_id: "session-1".to_string(),
                binding_id: "binding-1".to_string(),
                target_session_id: "session-2".to_string(),
                target_publication_ref: "publication-2".to_string(),
            },
        ),
        LocalDaemonRequest::TestWorkflowEventBinding(
            crate::local::TestWorkflowEventBindingRequest {
                session_id: "session-1".to_string(),
                binding_id: "binding-1".to_string(),
                prompt: Some("Review pull request #42.".to_string()),
            },
        ),
        LocalDaemonRequest::GetEventDeliveryStatus(crate::local::GetEventDeliveryStatusRequest),
        LocalDaemonRequest::StartEventGeneratorAuthorization(
            crate::local::StartEventGeneratorAuthorizationRequest {
                generator_id: "dev.arroba.github".to_string(),
                return_url: Some("https://terminal.arroba.dev/events/callback".to_string()),
            },
        ),
        LocalDaemonRequest::ListEventGeneratorResources(
            crate::local::ListEventGeneratorResourcesRequest {
                generator_id: "dev.arroba.github".to_string(),
                connection_id: "connection-1".to_string(),
                query: Some("arroba".to_string()),
                cursor: Some("opaque-resource-cursor".to_string()),
                limit: 20,
            },
        ),
        LocalDaemonRequest::ListEventConnections(crate::local::ListEventConnectionsRequest {
            generator_id: Some("dev.arroba.github".to_string()),
            cursor: Some("offset-20".to_string()),
            limit: 20,
        }),
        LocalDaemonRequest::GetEventConnection(crate::local::GetEventConnectionRequest {
            connection_id: "connection-1".to_string(),
        }),
        LocalDaemonRequest::InstallEventConnection(crate::local::InstallEventConnectionRequest {
            generator_id: "dev.arroba.github".to_string(),
            return_url: Some("https://terminal.arroba.dev/notifications/callback".to_string()),
        }),
        LocalDaemonRequest::ObserveEventConnectionAuthorization(
            crate::local::ObserveEventConnectionAuthorizationRequest {
                authorization_id: "event-authorization-1".to_string(),
            },
        ),
        LocalDaemonRequest::RefreshEventConnection(crate::local::RefreshEventConnectionRequest {
            connection_id: "connection-1".to_string(),
        }),
        LocalDaemonRequest::ReconnectEventConnection(
            crate::local::ReconnectEventConnectionRequest {
                connection_id: "connection-1".to_string(),
                return_url: Some("https://terminal.arroba.dev/notifications/callback".to_string()),
            },
        ),
        LocalDaemonRequest::ListEventConnectionResources(
            crate::local::ListEventConnectionResourcesRequest {
                connection_id: "connection-1".to_string(),
                query: Some("arroba".to_string()),
                cursor: None,
                limit: 20,
            },
        ),
        LocalDaemonRequest::ListEventConnectionDependencies(
            crate::local::ListEventConnectionDependenciesRequest {
                connection_id: "connection-1".to_string(),
            },
        ),
        LocalDaemonRequest::RemoveEventConnection(crate::local::RemoveEventConnectionRequest {
            connection_id: "connection-1".to_string(),
            confirm: true,
        }),
    ];
    let responses = vec![
        LocalDaemonResponse::EventGeneratorEventsPage {
            page: crate::local::EventGeneratorEventPage {
                events: vec![crate::local::EventGeneratorEventDefinition {
                    event_type: "pull_request.opened".to_string(),
                    version: 1,
                    name: "Pull request opened".to_string(),
                    description: "A pull request was opened.".to_string(),
                    filter_schema: serde_json::json!({"type": "object"}),
                    required_scopes: vec!["pull_requests:read".to_string()],
                }],
                next_cursor: Some("opaque-next-event-cursor".to_string()),
            },
        },
        LocalDaemonResponse::EventGeneratorCatalogPage {
            page: crate::local::EventGeneratorCatalogPage {
                services: vec![crate::local::EventGeneratorCatalogSummary {
                    schema_version: 1,
                    generator_id: "dev.arroba.github".to_string(),
                    version: "1.0.0".to_string(),
                    name: "GitHub".to_string(),
                    summary: "GitHub events.".to_string(),
                    provider: "GitHub".to_string(),
                    publisher: crate::local::EventGeneratorParty {
                        id: "dev.arroba".to_string(),
                        name: "Arroba".to_string(),
                        url: Some("https://arroba.dev".to_string()),
                    },
                    operator: crate::local::EventGeneratorParty {
                        id: "hosted.arroba".to_string(),
                        name: "Arroba hosted service".to_string(),
                        url: Some("https://arroba.dev".to_string()),
                    },
                    verification: "arroba".to_string(),
                    manifest_digest: format!("sha256:{}", "b".repeat(64)),
                    protocol_version: 1,
                    categories: vec!["developer-tools".to_string()],
                    installed_count: 0,
                    recommended: true,
                    availability: "development_preview".to_string(),
                }],
                next_cursor: Some("opaque-next-catalog-cursor".to_string()),
                categories: Vec::new(),
                facets: Vec::new(),
                stale: false,
            },
        },
        LocalDaemonResponse::EventGeneratorAuthorizationStarted {
            flow: crate::local::EventGeneratorAuthorizationFlow {
                generator_id: "dev.arroba.github".to_string(),
                status: "user_action_required".to_string(),
                connection_id: Some("connection-pending-1".to_string()),
                authorization_url: Some(
                    "https://github.com/apps/arroba/installations/new".to_string(),
                ),
                user_code: None,
                expires_at_ms: Some(1_800_000),
            },
        },
        LocalDaemonResponse::EventGeneratorResourcesPage {
            page: crate::local::EventGeneratorResourcePage {
                resources: vec![crate::local::EventGeneratorResource {
                    id: "repository-1".to_string(),
                    name: "arroba/arroba".to_string(),
                    kind: "repository".to_string(),
                    connection_scope: "arroba/arroba".to_string(),
                }],
                next_cursor: Some("opaque-next-resource-cursor".to_string()),
            },
        },
        LocalDaemonResponse::EventConnectionsPage {
            page: crate::local::EventConnectionPage {
                connections: vec![crate::local::EventConnection {
                    generator_id: "dev.arroba.github".to_string(),
                    connection_id: "connection-1".to_string(),
                    status: crate::local::EventConnectionStatus::Ready,
                    metadata: serde_json::json!({"account": "arroba"}),
                    expires_at_ms: None,
                    created_at_ms: 1_700_000,
                    updated_at_ms: 1_800_000,
                    last_validated_at_ms: Some(1_800_000),
                }],
                next_cursor: None,
            },
        },
        LocalDaemonResponse::EventConnectionAuthorizationStarted {
            authorization: crate::local::EventConnectionAuthorization {
                authorization_id: "event-authorization-1".to_string(),
                generator_id: "dev.arroba.github".to_string(),
                connection_id: Some("connection-1".to_string()),
                status: "user_action_required".to_string(),
                authorization_url: Some(
                    "https://github.com/apps/arroba/installations/new".to_string(),
                ),
                user_code: None,
                expires_at_ms: Some(1_900_000),
                created_at_ms: 1_800_000,
            },
        },
        LocalDaemonResponse::EventConnection {
            connection: crate::local::EventConnection {
                generator_id: "dev.arroba.github".to_string(),
                connection_id: "connection-1".to_string(),
                status: crate::local::EventConnectionStatus::Ready,
                metadata: serde_json::json!({"account": "arroba"}),
                expires_at_ms: None,
                created_at_ms: 1_700_000,
                updated_at_ms: 1_800_000,
                last_validated_at_ms: Some(1_800_000),
            },
        },
        LocalDaemonResponse::EventConnectionAuthorizationObserved {
            authorization: crate::local::EventConnectionAuthorization {
                authorization_id: "event-authorization-1".to_string(),
                generator_id: "dev.arroba.github".to_string(),
                connection_id: Some("connection-1".to_string()),
                status: "ready".to_string(),
                authorization_url: None,
                user_code: None,
                expires_at_ms: Some(1_900_000),
                created_at_ms: 1_800_000,
            },
            connection: None,
        },
        LocalDaemonResponse::EventConnectionResourcesPage {
            page: crate::local::EventGeneratorResourcePage {
                resources: vec![crate::local::EventGeneratorResource {
                    id: "repository-1".to_string(),
                    name: "arroba/arroba".to_string(),
                    kind: "repository".to_string(),
                    connection_scope: "arroba/arroba".to_string(),
                }],
                next_cursor: None,
            },
        },
        LocalDaemonResponse::EventConnectionDependencies {
            connection_id: "connection-1".to_string(),
            dependencies: vec![crate::local::WorkflowEventBindingDependency {
                session_id: "session-1".to_string(),
                publication_id: "publication-1".to_string(),
                binding_id: "binding-1".to_string(),
                status: crate::session::WorkflowEventBindingStatus::Active,
            }],
        },
        LocalDaemonResponse::EventConnectionRemoved {
            connection: crate::local::EventConnection {
                generator_id: "dev.arroba.github".to_string(),
                connection_id: "connection-1".to_string(),
                status: crate::local::EventConnectionStatus::Revoked,
                metadata: serde_json::json!({"account": "arroba"}),
                expires_at_ms: None,
                created_at_ms: 1_700_000,
                updated_at_ms: 1_800_000,
                last_validated_at_ms: Some(1_800_000),
            },
            deactivated_bindings: Vec::new(),
        },
    ];
    let snapshot = serde_json::json!({
        "requests": requests,
        "responses": responses,
    });
    assert_eq!(
        snapshot.pointer("/requests/1/SearchEventGeneratorCatalog/cursor"),
        Some(&serde_json::json!("opaque-cursor"))
    );
    assert_eq!(
        snapshot.pointer("/requests/5/CreateWorkflowEventBinding/filter/repository"),
        Some(&serde_json::json!("arroba"))
    );
    assert_eq!(
        snapshot.pointer("/requests/7/SetWorkflowEventBindingStatus/status"),
        Some(&serde_json::json!("paused"))
    );
    assert_eq!(
        snapshot.pointer("/requests/8/TransferWorkflowEventBinding/target_publication_ref"),
        Some(&serde_json::json!("publication-2"))
    );
    assert_eq!(
        snapshot.pointer("/responses/0/EventGeneratorEventsPage/page/events/0/required_scopes/0"),
        Some(&serde_json::json!("pull_requests:read"))
    );
    assert_eq!(
        snapshot.pointer("/responses/1/EventGeneratorCatalogPage/page/services/0/availability"),
        Some(&serde_json::json!("development_preview"))
    );
    assert_eq!(
        snapshot.pointer("/responses/1/EventGeneratorCatalogPage/page/services/0/schema_version"),
        Some(&serde_json::json!(1))
    );
    assert_eq!(
        snapshot.pointer("/responses/2/EventGeneratorAuthorizationStarted/flow/authorization_url"),
        Some(&serde_json::json!(
            "https://github.com/apps/arroba/installations/new"
        ))
    );
    assert_eq!(
        snapshot
            .pointer("/responses/3/EventGeneratorResourcesPage/page/resources/0/connection_scope"),
        Some(&serde_json::json!("arroba/arroba"))
    );
    let serialized = serde_json::to_string(&snapshot).unwrap();
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "79498ab1751810a2194c0ea4f23b137db54a25e9417dfb04c9a8735c837ebc1b"
    );
}
