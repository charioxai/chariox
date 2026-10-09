use super::*;
use sha2::{Digest, Sha256};

fn sample_event_connection(
    status: crate::local::EventConnectionStatus,
) -> crate::local::EventConnection {
    crate::local::EventConnection {
        generator_id: "dev.chariox.github".to_string(),
        connection_id: "connection-1".to_string(),
        status,
        lifecycle_state: crate::local::EventConnectionLifecycleState::ConnectedRestricted,
        scopes: vec![crate::local::EventConnectionScope {
            id: "pull_requests:read".to_string(),
            label: "Read pull requests".to_string(),
            granted: true,
            required: true,
        }],
        resources: vec![crate::local::EventConnectedResource {
            id: "repository-1".to_string(),
            name: "charioxai/chariox".to_string(),
            kind: "repository".to_string(),
        }],
        attached_trigger_count: 2,
        metadata: serde_json::json!({"account": "chariox"}),
        expires_at_ms: None,
        created_at_ms: 1_700_000,
        updated_at_ms: 1_800_000,
        last_validated_at_ms: Some(1_800_000),
        last_successful_health_check_at_ms: Some(1_800_000),
        last_accepted_event_at_ms: Some(1_799_000),
        problem_code: None,
        problem_message: None,
        recovery_action: None,
        test_event_supported: true,
    }
}

#[test]
fn local_daemon_protocol_event_publication_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 459);
    let requests = vec![
        LocalDaemonRequest::GetEventGeneratorCatalogLanding(
            crate::local::GetEventGeneratorCatalogLandingRequest { limit: 12 },
        ),
        LocalDaemonRequest::SearchEventGeneratorCatalog(
            crate::local::SearchEventGeneratorCatalogRequest {
                query: "github".to_string(),
                category: Some("Developer tools".to_string()),
                verification: Some("chariox".to_string()),
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
            generator_id: "dev.chariox.github".to_string(),
            version: Some("1.0.0".to_string()),
        }),
        LocalDaemonRequest::BrowseEventGeneratorEvents(
            crate::local::BrowseEventGeneratorEventsRequest {
                generator_id: "dev.chariox.github".to_string(),
                query: Some("review".to_string()),
                cursor: Some("opaque-event-cursor".to_string()),
                limit: 50,
            },
        ),
        LocalDaemonRequest::GetEventDeliveryStatus(crate::local::GetEventDeliveryStatusRequest),
        LocalDaemonRequest::StartEventGeneratorAuthorization(
            crate::local::StartEventGeneratorAuthorizationRequest {
                generator_id: "dev.chariox.github".to_string(),
                return_url: Some("https://terminal.chariox.com/events/callback".to_string()),
            },
        ),
        LocalDaemonRequest::ListEventGeneratorResources(
            crate::local::ListEventGeneratorResourcesRequest {
                generator_id: "dev.chariox.github".to_string(),
                connection_id: "connection-1".to_string(),
                query: Some("chariox".to_string()),
                cursor: Some("opaque-resource-cursor".to_string()),
                limit: 20,
            },
        ),
        LocalDaemonRequest::ListEventConnections(crate::local::ListEventConnectionsRequest {
            generator_id: Some("dev.chariox.github".to_string()),
            cursor: Some("offset-20".to_string()),
            limit: 20,
        }),
        LocalDaemonRequest::GetEventConnection(crate::local::GetEventConnectionRequest {
            connection_id: "connection-1".to_string(),
        }),
        LocalDaemonRequest::InstallEventConnection(crate::local::InstallEventConnectionRequest {
            generator_id: "dev.chariox.github".to_string(),
            return_url: Some("https://terminal.chariox.com/notifications/callback".to_string()),
        }),
        LocalDaemonRequest::ObserveEventConnectionAuthorization(
            crate::local::ObserveEventConnectionAuthorizationRequest {
                authorization_id: "event-authorization-1".to_string(),
            },
        ),
        LocalDaemonRequest::RefreshEventConnection(crate::local::RefreshEventConnectionRequest {
            connection_id: "connection-1".to_string(),
        }),
        LocalDaemonRequest::TestEventConnection(crate::local::TestEventConnectionRequest {
            connection_id: "connection-1".to_string(),
            event_type: Some("pull_request.opened".to_string()),
        }),
        LocalDaemonRequest::ReconnectEventConnection(
            crate::local::ReconnectEventConnectionRequest {
                connection_id: "connection-1".to_string(),
                return_url: Some("https://terminal.chariox.com/notifications/callback".to_string()),
            },
        ),
        LocalDaemonRequest::ListEventConnectionResources(
            crate::local::ListEventConnectionResourcesRequest {
                connection_id: "connection-1".to_string(),
                query: Some("chariox".to_string()),
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
                    generator_id: "dev.chariox.github".to_string(),
                    version: "1.0.0".to_string(),
                    name: "GitHub".to_string(),
                    summary: "GitHub events.".to_string(),
                    provider: "GitHub".to_string(),
                    publisher: crate::local::EventGeneratorParty {
                        id: "dev.chariox".to_string(),
                        name: "Chariox".to_string(),
                        url: Some("https://chariox.com".to_string()),
                    },
                    operator: crate::local::EventGeneratorParty {
                        id: "hosted.chariox".to_string(),
                        name: "Chariox hosted service".to_string(),
                        url: Some("https://chariox.com".to_string()),
                    },
                    verification: "chariox".to_string(),
                    manifest_digest: format!("sha256:{}", "b".repeat(64)),
                    protocol_version: 3,
                    categories: vec!["developer-tools".to_string()],
                    installed_count: 0,
                    recommended: true,
                    availability: "development_preview".to_string(),
                    management_url: Some("https://aegs.example.test".to_string()),
                }],
                next_cursor: Some("opaque-next-catalog-cursor".to_string()),
                categories: Vec::new(),
                facets: Vec::new(),
                stale: false,
            },
        },
        LocalDaemonResponse::EventGeneratorAuthorizationStarted {
            flow: crate::local::EventGeneratorAuthorizationFlow {
                generator_id: "dev.chariox.github".to_string(),
                status: "user_action_required".to_string(),
                connection_id: Some("connection-pending-1".to_string()),
                authorization_url: Some(
                    "https://github.com/apps/chariox/installations/new".to_string(),
                ),
                user_code: None,
                expires_at_ms: Some(1_800_000),
            },
        },
        LocalDaemonResponse::EventGeneratorResourcesPage {
            page: crate::local::EventGeneratorResourcePage {
                resources: vec![crate::local::EventGeneratorResource {
                    id: "repository-1".to_string(),
                    name: "charioxai/chariox".to_string(),
                    kind: "repository".to_string(),
                    connection_scope: "charioxai/chariox".to_string(),
                    filter: None,
                }],
                next_cursor: Some("opaque-next-resource-cursor".to_string()),
            },
        },
        LocalDaemonResponse::EventConnectionsPage {
            page: crate::local::EventConnectionPage {
                connections: vec![sample_event_connection(
                    crate::local::EventConnectionStatus::Ready,
                )],
                next_cursor: None,
            },
        },
        LocalDaemonResponse::EventConnectionAuthorizationStarted {
            authorization: crate::local::EventConnectionAuthorization {
                authorization_id: "event-authorization-1".to_string(),
                generator_id: "dev.chariox.github".to_string(),
                connection_id: Some("connection-1".to_string()),
                status: "user_action_required".to_string(),
                authorization_url: Some(
                    "https://github.com/apps/chariox/installations/new".to_string(),
                ),
                user_code: None,
                expires_at_ms: Some(1_900_000),
                created_at_ms: 1_800_000,
            },
        },
        LocalDaemonResponse::EventConnection {
            connection: sample_event_connection(crate::local::EventConnectionStatus::Ready),
        },
        LocalDaemonResponse::EventConnectionAuthorizationObserved {
            authorization: crate::local::EventConnectionAuthorization {
                authorization_id: "event-authorization-1".to_string(),
                generator_id: "dev.chariox.github".to_string(),
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
                    id: "C123".to_string(),
                    name: "#general".to_string(),
                    kind: "slack_channel".to_string(),
                    connection_scope: "T123".to_string(),
                    filter: Some(serde_json::json!({"event.channel": "C123"})),
                }],
                next_cursor: None,
            },
        },
        LocalDaemonResponse::EventConnectionTested {
            result: chariox_event_protocol::AegsConnectionTestEventResponse {
                occurrence_id: "test-1".to_string(),
                accepted: true,
                message: None,
            },
        },
        LocalDaemonResponse::EventConnectionDependencies {
            connection_id: "connection-1".to_string(),
            dependencies: vec![crate::local::EventConnectionDependency {
                installation_id: "app_slack".to_string(),
                route_id: Some("mentions".to_string()),
                active: true,
            }],
        },
        LocalDaemonResponse::EventConnectionRemoved {
            connection: sample_event_connection(crate::local::EventConnectionStatus::Revoked),
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
            "https://github.com/apps/chariox/installations/new"
        ))
    );
    assert_eq!(
        snapshot
            .pointer("/responses/3/EventGeneratorResourcesPage/page/resources/0/connection_scope"),
        Some(&serde_json::json!("charioxai/chariox"))
    );
    // A resource's filter is optional and omitted when absent (protocol 362).
    assert_eq!(
        snapshot.pointer("/responses/3/EventGeneratorResourcesPage/page/resources/0/filter"),
        None
    );
    let serialized = serde_json::to_string(&snapshot).unwrap();
    assert!(serialized.contains(r#""connection_scope":"T123","filter":{"event.channel":"C123"}"#));
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "dd821bbcb628277dd0691269138772b0d8c6b22f30193663f6142f5af3c2027b"
    );
}

#[test]
fn workflow_turn_context_shape_is_versioned() {
    // Protocol 365 removed the event context and action capabilities.
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 459);
    let context = crate::execution_lease::RemoteWorkflowTurnContext {
        home_kernel_id: "home".to_string(),
        home_session_id: "room".to_string(),
        home_agent_id: "agent".to_string(),
        workflow_run_id: "run".to_string(),
        workflow_node_run_id: "node".to_string(),
        delivery_token: "token".to_string(),
    };
    let value = serde_json::to_value(&context).unwrap();
    assert_eq!(
        value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        [
            "delivery_token",
            "home_agent_id",
            "home_kernel_id",
            "home_session_id",
            "workflow_node_run_id",
            "workflow_run_id",
        ]
    );
    // Older peers still send the removed capabilities.
    let mut legacy = value.clone();
    legacy["event_context_enabled"] = serde_json::json!(true);
    legacy["event_actions_enabled"] = serde_json::json!(true);
    assert_eq!(
        serde_json::from_value::<crate::execution_lease::RemoteWorkflowTurnContext>(legacy)
            .unwrap(),
        context
    );
    let serialized = serde_json::to_string(&value).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(serialized.as_bytes())),
        "ee11932387891cb53da8ddaa37805476972a9323531f648f88d5fa8f522e1c21"
    );
}
