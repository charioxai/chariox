use crate::local::*;
use sha2::{Digest, Sha256};

#[test]
fn app_installation_protocol_shapes_are_versioned_and_preserve_generations() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 485);
    let release = AppReleaseSummary {
        version: "1.0.0".into(),
        publisher_id: "publisher".into(),
        package_digest: format!("sha256:{:064x}", 1),
        schema_version: 1,
    };
    let requests = vec![
        LocalDaemonRequest::ListAppInstallations(ListAppInstallationsRequest {
            after: None,
            limit: Some(50),
        }),
        LocalDaemonRequest::GetAppInstallation(AppInstallationRequest {
            installation_id: "todo".into(),
        }),
        LocalDaemonRequest::GetAppInstallationJournal(AppInstallationRequest {
            installation_id: "todo".into(),
        }),
    ];
    let responses = vec![
        LocalDaemonResponse::AppInstallationsListed {
            installations: vec![],
            next_cursor: None,
        },
        LocalDaemonResponse::AppInstallation {
            installation: AppInstallationSummary {
                installation_id: "todo".into(),
                app_id: "com.chariox.todo".into(),
                generation: "9223372036854775807".into(),
                active_release: Some(release.clone()),
                pending_generation: Some("9223372036854775806".into()),
                admission_paused: false,
                data_kept: true,
            },
        },
        LocalDaemonResponse::AppInstallationJournal {
            installation_id: "todo".into(),
            updates: vec![AppUpdateSummary {
                base_generation: "0".into(),
                generation: "1".into(),
                release,
                phase: AppUpdatePhase::Staged,
                decision: AppCapabilityDecisionStatus::Pending,
                created_at_ms: 1,
                updated_at_ms: 2,
            }],
        },
        LocalDaemonResponse::AppRequestFailed {
            code: AppRequestErrorCode::NotFound,
        },
    ];
    for request in &requests {
        assert_eq!(
            &serde_json::from_slice::<LocalDaemonRequest>(&serde_json::to_vec(request).unwrap())
                .unwrap(),
            request
        );
    }
    for response in &responses {
        assert_eq!(
            &serde_json::from_slice::<LocalDaemonResponse>(&serde_json::to_vec(response).unwrap())
                .unwrap(),
            response
        );
    }
    let snapshot = serde_json::json!({"requests": requests, "responses": responses});
    let encoded = serde_json::to_vec(&snapshot).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&encoded)),
        "49ca2c2378eddcfc193c7397ef4920ee8844f8cf4d7f20d04687190752116471"
    );
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"ListAppInstallations": {"owner_id": "other"}})
    )
    .is_err());
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"GetAppInstallation": {"installation_id": "todo", "path": "/host"}})
    )
    .is_err());
}

#[test]
fn app_package_upload_protocol_shapes_bind_retry_bytes_and_opaque_handles() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 485);
    let handle = format!("upload_{}", "a".repeat(64));
    let digest = format!("sha256:{:064x}", 1);
    let requests = vec![
        LocalDaemonRequest::BeginAppPackageUpload(BeginAppPackageUploadRequest {
            request_id: "retry".into(),
            expected_size: 4,
            sha256: digest.clone(),
        }),
        LocalDaemonRequest::PutAppPackageUploadChunk(PutAppPackageUploadChunkRequest {
            handle: handle.clone(),
            offset: 0,
            data_base64: "dGVzdA==".into(),
            chunk_sha256: digest.clone(),
        }),
        LocalDaemonRequest::GetAppPackageUpload(AppPackageUploadRequest {
            handle: handle.clone(),
        }),
        LocalDaemonRequest::AbortAppPackageUpload(AppPackageUploadRequest {
            handle: handle.clone(),
        }),
    ];
    let mut responses = Vec::new();
    for phase in [
        AppPackageUploadPhase::Receiving,
        AppPackageUploadPhase::Finalized,
        AppPackageUploadPhase::Aborted,
    ] {
        responses.push(LocalDaemonResponse::AppPackageUploadStatus {
            upload: AppPackageUploadSummary {
                handle: handle.clone(),
                phase,
                expected_size: 4,
                accepted_bytes: 4,
                sha256: digest.clone(),
                expires_at_ms: 1_800_000,
            },
        });
    }
    for code in [
        AppRequestErrorCode::InvalidRequest,
        AppRequestErrorCode::Unauthorized,
        AppRequestErrorCode::NotFound,
        AppRequestErrorCode::Busy,
        AppRequestErrorCode::LimitExceeded,
        AppRequestErrorCode::Conflict,
        AppRequestErrorCode::DigestMismatch,
        AppRequestErrorCode::StorageUnavailable,
    ] {
        responses.push(LocalDaemonResponse::AppRequestFailed { code });
    }
    for request in &requests {
        assert_eq!(
            &serde_json::from_slice::<LocalDaemonRequest>(&serde_json::to_vec(request).unwrap())
                .unwrap(),
            request
        );
    }
    for response in &responses {
        assert_eq!(
            &serde_json::from_slice::<LocalDaemonResponse>(&serde_json::to_vec(response).unwrap())
                .unwrap(),
            response
        );
    }
    let snapshot = serde_json::json!({"requests": requests, "responses": responses});
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "eb09d2a4c9ede9b7936ffc58c3d369f65bd4dd93253fea60ec419d663be63c6a"
    );
    for field in ["owner_id", "path", "expires_at_ms"] {
        let mut forged = serde_json::to_value(&requests[0]).unwrap();
        forged["BeginAppPackageUpload"][field] = serde_json::json!("caller-supplied");
        assert!(serde_json::from_value::<LocalDaemonRequest>(forged).is_err());
    }
}

#[test]
fn app_worker_control_and_automation_shapes_are_versioned_and_owner_free() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 485);
    let requests = vec![
        LocalDaemonRequest::GetAppWorker(AppWorkerRequest {
            installation_id: "todo".into(),
        }),
        LocalDaemonRequest::ControlAppWorker(ControlAppWorkerRequest {
            installation_id: "todo".into(),
            action: AppWorkerAction::Restart,
        }),
        LocalDaemonRequest::ListAppAutomations(AppWorkerRequest {
            installation_id: "todo".into(),
        }),
        LocalDaemonRequest::ConfigureAppAutomation(ConfigureAppAutomationRequest {
            delivery_mode: crate::local::NotificationDeliveryMode::Queue,
            installation_id: "todo".into(),
            automation_id: "reminders".into(),
            expected_revision: 0,
            event_name: "todo_due".into(),
            session_id: "session-1".into(),
            publication_ref: "todo-reminders".into(),
            queue_ref: None,
            scheduled: true,
        }),
        LocalDaemonRequest::DisableAppAutomation(DisableAppAutomationRequest {
            installation_id: "todo".into(),
            automation_id: "reminders".into(),
            expected_revision: 1,
        }),
    ];
    let automation = AppAutomationSummary {
        delivery_mode: crate::local::NotificationDeliveryMode::Queue,
        automation_id: "reminders".into(),
        revision: 1,
        event_name: "todo_due".into(),
        event_version: 1,
        session_id: "session-1".into(),
        publication_id: "publication-1".into(),
        endpoint_id: "endpoint-1".into(),
        queue_id: "queue-1".into(),
        scheduled: true,
        status: AppAutomationStatus::Active,
    };
    let responses = vec![
        LocalDaemonResponse::AppWorker {
            worker: AppWorkerSummary {
                installation_id: "todo".into(),
                phase: AppWorkerPhase::Dormant,
                enabled: true,
                failure: None,
                updated_at_ms: Some(1),
            },
        },
        LocalDaemonResponse::AppWorker {
            worker: AppWorkerSummary {
                installation_id: "todo".into(),
                phase: AppWorkerPhase::Quarantined,
                enabled: true,
                failure: Some("app_worker_exited".into()),
                updated_at_ms: Some(1),
            },
        },
        LocalDaemonResponse::AppAutomations {
            installation_id: "todo".into(),
            automations: vec![automation.clone()],
        },
        LocalDaemonResponse::AppAutomation {
            installation_id: "todo".into(),
            automation,
        },
    ];
    for request in &requests {
        let encoded = serde_json::to_vec(request).unwrap();
        assert_eq!(
            &serde_json::from_slice::<LocalDaemonRequest>(&encoded).unwrap(),
            request
        );
    }
    for response in &responses {
        let encoded = serde_json::to_vec(response).unwrap();
        assert_eq!(
            &serde_json::from_slice::<LocalDaemonResponse>(&encoded).unwrap(),
            response
        );
    }
    let snapshot = serde_json::json!({"requests": requests, "responses": responses});
    assert_eq!(
        snapshot["responses"][1]["AppWorker"]["worker"]["phase"],
        "quarantined"
    );
    let digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
    );
    assert_eq!(
        digest,
        "d6ef051ad7e4cda125c1ac1f3db9fd2e082056a697e0de681b777eec0ce81a06"
    );
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"ControlAppWorker": {"installation_id": "todo", "action": "start", "owner_id": "other"}})
    )
    .is_err());
}

#[test]
fn app_view_shapes_are_versioned_and_name_no_owner_or_asset() {
    use crate::runtime::browser_controller_app_view::{BrowserAppViewError, BrowserAppViewRequest};
    use crate::transport::room_browser_controller::RoomBrowserControllerCommand;
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 485);
    let request = LocalDaemonRequest::OpenAppView(OpenAppViewRequest {
        session_id: "session-1".into(),
        installation_id: "todo".into(),
    });
    let response = LocalDaemonResponse::AppViewOpened {
        installation_id: "todo".into(),
        target_id: "target-1".into(),
        origin: "https://a0.app.chariox.internal".into(),
        bound_agent_id: Some("agent-1".into()),
    };
    let commands = vec![
        RoomBrowserControllerCommand::AppView {
            request: BrowserAppViewRequest::Calls,
        },
        RoomBrowserControllerCommand::AppView {
            request: BrowserAppViewRequest::Respond {
                target_id: "target-1".into(),
                call_id: "1".into(),
                result: None,
                error: Some(BrowserAppViewError {
                    code: "APP_ERROR".into(),
                    message: "failed".into(),
                }),
            },
        },
    ];
    let encoded = serde_json::to_vec(&request).unwrap();
    assert_eq!(
        serde_json::from_slice::<LocalDaemonRequest>(&encoded).unwrap(),
        request
    );
    let snapshot =
        serde_json::json!({"request": request, "response": response, "commands": commands});
    let digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
    );
    assert_eq!(
        digest,
        "f0edd46cd7fa68987b5d4c3ef166d2417d75d5f5dd8053d626ebbb323c4292b2"
    );
    assert!(
        serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
            "OpenAppView": {"session_id": "s", "installation_id": "todo", "owner_id": "other"}
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
            "OpenAppView": {"session_id": "s", "installation_id": "todo", "assets": []}
        }))
        .is_err()
    );
}

#[test]
fn app_view_panel_choice_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 485);
    let request = LocalDaemonRequest::SetAppViewPanel(crate::local::SetAppViewPanelRequest {
        session_id: "session-1".into(),
        installation_id: "todo".into(),
        placement: Some(crate::session::AppPanelPlacement::Bottom),
        minimized: None,
        reset: false,
    });
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        serde_json::json!({"SetAppViewPanel": {"session_id": "session-1", "installation_id": "todo", "placement": "bottom"}})
    );
    let response = LocalDaemonResponse::AppViewPanelSet {
        installation_id: "todo".into(),
        placement: None,
        minimized: true,
    };
    assert_eq!(
        serde_json::to_value(&response).unwrap(),
        serde_json::json!({"AppViewPanelSet": {"installation_id": "todo", "placement": null, "minimized": true}})
    );
    assert!(
        serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
            "SetAppViewPanel": {"session_id": "s", "installation_id": "todo", "placement": "left"}
        }))
        .is_err()
    );
}

#[test]
fn app_uninstall_shape_is_versioned_and_names_no_owner() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 485);
    let request = LocalDaemonRequest::UninstallApp(UninstallAppRequest {
        installation_id: "todo".into(),
        expected_generation: "3".into(),
        delete_data: false,
    });
    let encoded = serde_json::to_vec(&request).unwrap();
    assert_eq!(
        serde_json::from_slice::<LocalDaemonRequest>(&encoded).unwrap(),
        request
    );
    let digest = format!("{:x}", Sha256::digest(&encoded));
    assert_eq!(
        digest,
        "43dd00eda2df4b70378a7fc6056cdf37173361a38225ebbeaa8e826e7272d46e"
    );
    assert!(serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
        "UninstallApp": {"installation_id": "todo", "expected_generation": "3", "owner_id": "other"}
    }))
    .is_err());
    assert!(matches!(
        serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
            "SetAppViewPanel": {"session_id": "s", "installation_id": "todo", "reset": true}
        })),
        Ok(LocalDaemonRequest::SetAppViewPanel(
            crate::local::SetAppViewPanelRequest { reset: true, .. }
        ))
    ));
    // Protocol 363: deleting the data is explicit and omitted when not asked.
    let delete = LocalDaemonRequest::UninstallApp(UninstallAppRequest {
        installation_id: "todo".into(),
        expected_generation: "3".into(),
        delete_data: true,
    });
    assert_eq!(
        serde_json::to_value(&delete).unwrap(),
        serde_json::json!({"UninstallApp": {
            "installation_id": "todo", "expected_generation": "3", "delete_data": true
        }})
    );
}

#[test]
fn app_logs_shape_is_versioned_and_names_no_owner() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 485);
    let request = LocalDaemonRequest::GetAppLogs(GetAppLogsRequest {
        installation_id: "todo".into(),
        after_sequence: Some("7".into()),
        limit: Some(50),
    });
    let response = LocalDaemonResponse::AppLogs {
        installation_id: "todo".into(),
        entries: vec![AppLogEntrySummary {
            sequence: "8".into(),
            at_ms: 1,
            level: "info".into(),
            message: "hello".into(),
            fields: serde_json::json!({"a": 1}),
        }],
    };
    let encoded = serde_json::to_vec(&request).unwrap();
    assert_eq!(
        serde_json::from_slice::<LocalDaemonRequest>(&encoded).unwrap(),
        request
    );
    let snapshot = serde_json::json!({"request": request, "response": response});
    let digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
    );
    assert_eq!(
        digest,
        "c8f54294ac11779770905df9b2512f7a0ff3fccf55a28b23ffd672fe4bc19483"
    );
    assert!(
        serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
            "GetAppLogs": {"installation_id": "todo", "owner_id": "other"}
        }))
        .is_err()
    );
}

#[test]
fn app_update_shape_is_versioned_fenced_and_names_no_owner() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 485);
    let request = LocalDaemonRequest::BeginAppUpdate(BeginAppUpdateRequest {
        session_id: "session-1".into(),
        request_id: "update-1".into(),
        installation_id: "todo".into(),
        expected_generation: "3".into(),
        upload_handle: format!("upload_{}", "a".repeat(64)),
        expected_package_digest: format!("sha256:{:064x}", 1),
    });
    let encoded = serde_json::to_vec(&request).unwrap();
    assert_eq!(
        serde_json::from_slice::<LocalDaemonRequest>(&encoded).unwrap(),
        request
    );
    let digest = format!("{:x}", Sha256::digest(&encoded));
    assert_eq!(
        digest,
        "413ddaba55fa0d7ed72db9ffa2e4186cef09cbc6a1127521372b6fdf76172efb"
    );
    assert!(
        serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
            "BeginAppUpdate": {"session_id": "s", "request_id": "r", "installation_id": "todo",
                "expected_generation": "3", "upload_handle": "u", "expected_package_digest": "d",
                "owner_id": "other"}
        }))
        .is_err()
    );
}

#[test]
fn app_inbox_shapes_are_versioned_and_name_no_owner() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 485);
    let requests = vec![
        LocalDaemonRequest::CreateAppInboxRoute(CreateAppInboxRouteRequest {
            installation_id: "todo".into(),
            route_id: "mail".into(),
            event_name: "todo_requested".into(),
            source_event_type: "dev.chariox.dummy/dummy.test".into(),
            source_event_version: 1,
            connection: None,
        }),
        LocalDaemonRequest::RemoveAppInboxRoute(AppInboxRouteRequest {
            installation_id: "todo".into(),
            route_id: "mail".into(),
        }),
        LocalDaemonRequest::ListAppInboxRoutes(AppWorkerRequest {
            installation_id: "todo".into(),
        }),
        LocalDaemonRequest::TestAppInboxRoute(TestAppInboxRouteRequest {
            installation_id: "todo".into(),
            route_id: "mail".into(),
            occurrence_id: "occ-1".into(),
            payload: serde_json::json!({"title": "x"}),
        }),
    ];
    let responses = vec![
        LocalDaemonResponse::AppInboxRoutes {
            installation_id: "todo".into(),
            routes: vec![AppInboxRouteSummary {
                route_id: "mail".into(),
                event_name: "todo_requested".into(),
                source_event_type: "dev.chariox.dummy/dummy.test".into(),
                source_event_version: 1,
                active: true,
                connection: None,
                pending: 1,
                delivered: 2,
                failed: 0,
                expired: 0,
            }],
        },
        LocalDaemonResponse::AppInboxOccurrenceAccepted {
            installation_id: "todo".into(),
            route_id: "mail".into(),
            occurrence_id: "occ-1".into(),
            duplicate: false,
        },
    ];
    for request in &requests {
        let encoded = serde_json::to_vec(request).unwrap();
        assert_eq!(
            &serde_json::from_slice::<LocalDaemonRequest>(&encoded).unwrap(),
            request
        );
    }
    let snapshot = serde_json::json!({"requests": requests, "responses": responses});
    let digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
    );
    assert_eq!(
        digest,
        "9447b2103c913c1ca4099b2ba6452aa7d137ede5a5f49108d28acafd0271bba1"
    );
    assert!(
        serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
            "CreateAppInboxRoute": {"installation_id": "todo", "route_id": "r", "event_name": "e",
                "source_event_type": "t", "source_event_version": 1, "owner_id": "other"}
        }))
        .is_err()
    );
}

#[test]
fn app_inbox_routes_from_generator_connections_are_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 485);
    let connection = AppInboxConnection {
        generator_id: "dev.chariox.slack".into(),
        connection_id: "connection-1".into(),
        connection_scope: "team:T1".into(),
        filter: serde_json::json!({"channel": "C1"}),
    };
    let request = LocalDaemonRequest::CreateAppInboxRoute(CreateAppInboxRouteRequest {
        installation_id: "slack".into(),
        route_id: "mentions".into(),
        event_name: "mentioned".into(),
        source_event_type: "app.mentioned".into(),
        source_event_version: 1,
        connection: Some(connection.clone()),
    });
    let response = LocalDaemonResponse::AppInboxRoutes {
        installation_id: "slack".into(),
        routes: vec![AppInboxRouteSummary {
            route_id: "mentions".into(),
            event_name: "mentioned".into(),
            source_event_type: "app.mentioned".into(),
            source_event_version: 1,
            active: true,
            connection: Some(connection),
            pending: 0,
            delivered: 0,
            failed: 0,
            expired: 0,
        }],
    };
    let encoded = serde_json::to_vec(&request).unwrap();
    assert_eq!(
        serde_json::from_slice::<LocalDaemonRequest>(&encoded).unwrap(),
        request
    );
    let snapshot = serde_json::json!({"request": request, "response": response});
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "9702eea10502d2d51589f8492940f498b70f83a0f36f069979c46b9af2c09d0d"
    );
    // A connection names no owner.
    assert!(
        serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
            "CreateAppInboxRoute": {"installation_id": "slack", "route_id": "r", "event_name": "e",
                "source_event_type": "t", "source_event_version": 1,
                "connection": {"generator_id": "g", "connection_id": "c",
                    "connection_scope": "s", "owner_id": "other"}}
        }))
        .is_err()
    );
}

#[test]
fn app_connection_grants_are_versioned_and_name_no_owner() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 485);
    let requests = vec![
        LocalDaemonRequest::GrantAppConnection(GrantAppConnectionRequest {
            installation_id: "slack".into(),
            generator_id: "dev.chariox.slack".into(),
            connection_id: "connection-1".into(),
        }),
        LocalDaemonRequest::RevokeAppConnection(RevokeAppConnectionRequest {
            installation_id: "slack".into(),
            connection_id: "connection-1".into(),
        }),
        LocalDaemonRequest::ListAppConnections(AppWorkerRequest {
            installation_id: "slack".into(),
        }),
    ];
    let response = LocalDaemonResponse::AppConnections {
        installation_id: "slack".into(),
        connections: vec![AppConnectionSummary {
            generator_id: "dev.chariox.slack".into(),
            connection_id: "connection-1".into(),
            granted_at_ms: 5,
            actions: vec!["notification.reply".into()],
        }],
    };
    for request in &requests {
        let encoded = serde_json::to_vec(request).unwrap();
        assert_eq!(
            &serde_json::from_slice::<LocalDaemonRequest>(&encoded).unwrap(),
            request
        );
    }
    let snapshot = serde_json::json!({"requests": requests, "response": response});
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "472b8bc15a7f4aaca635342bba979a2eeee2d10deea2a701e0508e9ba2ef5f84"
    );
    assert!(
        serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
            "GrantAppConnection": {"installation_id": "slack", "generator_id": "g",
                "connection_id": "c", "owner_id": "other"}
        }))
        .is_err()
    );
}

#[test]
fn an_app_set_is_versioned_and_carries_no_app_data() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 485);
    let request = LocalDaemonRequest::GetAppSet(GetAppSetRequest {});
    assert_eq!(
        serde_json::from_slice::<LocalDaemonRequest>(&serde_json::to_vec(&request).unwrap())
            .unwrap(),
        request
    );
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"GetAppSet": {"owner_id": "other"}})
    )
    .is_err());
    let response = LocalDaemonResponse::AppSet {
        schema: APP_SET_SCHEMA.into(),
        installations: vec![AppSetInstallation {
            installation_id: "todo".into(),
            app_id: "com.chariox.todo".into(),
            release: AppReleaseSummary {
                version: "1.2.0".into(),
                publisher_id: "com.chariox".into(),
                package_digest: format!("sha256:{:064x}", 1),
                schema_version: 1,
            },
            capabilities: serde_json::json!({"network": []}),
            automations: Vec::new(),
            inbox_routes: Vec::new(),
            connections: Vec::new(),
            deployment_id: Some("deployment-1".into()),
        }],
    };
    let snapshot = serde_json::json!({"request": request, "response": response});
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "9d98ad9c929e1aa0c0500e434283ddbfb4bd56771674f0586e57609151376f91"
    );
}

#[test]
fn deployment_app_consent_is_versioned_and_carries_no_approval() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 485);
    let request = LocalDaemonRequest::PrepareDeploymentApps(PrepareDeploymentAppsRequest {
        session_id: "session-1".into(),
        request_id: "deploy-apps-1".into(),
        publication_ref: "publication-1".into(),
        deployment_id: "deployment-1".into(),
        release_id: "release-1".into(),
        package_digest: format!("sha256:{:064x}", 1),
    });
    assert_eq!(
        serde_json::from_slice::<LocalDaemonRequest>(&serde_json::to_vec(&request).unwrap())
            .unwrap(),
        request
    );
    // A caller cannot supply an approval or an owner.
    for extra in ["approved", "owner_id"] {
        let mut value = serde_json::to_value(&request).unwrap();
        value["PrepareDeploymentApps"][extra] = serde_json::json!(true);
        assert!(serde_json::from_value::<LocalDaemonRequest>(value).is_err());
    }
    let response = LocalDaemonResponse::DeploymentAppsConsent {
        consent: DeploymentAppsConsent {
            request_id: "deploy-apps-1".into(),
            interaction_id: format!("app_deploy_{:032x}", 1),
            deployment_id: "deployment-1".into(),
            release_id: "release-1".into(),
            package_digest: format!("sha256:{:064x}", 1),
            status: DeploymentAppsConsentStatus::AwaitingApproval,
            expires_at_ms: 42,
        },
    };
    // Protocol 377: a release's own plan by package digest.
    let preview = LocalDaemonRequest::PreviewDeploymentApps(PreviewDeploymentAppsRequest {
        session_id: "session-1".into(),
        publication_ref: "publication-1".into(),
        package_digest: Some(format!("sha256:{}", "b".repeat(64))),
    });
    let previewed = LocalDaemonResponse::DeploymentAppsPreview {
        publication_id: "publication-1".into(),
        pinned: true,
        plan: Some(serde_json::json!({
            "schema": "chariox.publication-apps.v1",
            "apps": [{"installation_id": "todo", "capabilities": {"network": []}}],
        })),
        release_plan: Some(serde_json::json!({
            "schema": "chariox.publication-apps.v1",
            "apps": [{"installation_id": "todo", "version": "1.0.0", "capabilities": {"network": []}}],
        })),
    };
    assert_eq!(
        serde_json::from_slice::<LocalDaemonRequest>(&serde_json::to_vec(&preview).unwrap())
            .unwrap(),
        preview
    );
    let snapshot = serde_json::json!({
        "request": request, "response": response, "preview": preview, "previewed": previewed,
    });
    assert_eq!(
        snapshot.pointer("/response/DeploymentAppsConsent/consent/status"),
        Some(&serde_json::json!("awaiting_approval"))
    );
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "7c026d45bbbaf8f42d5057227bd1c1ababfb485afaae376856287e03e01cc283"
    );
}

#[test]
fn app_file_grant_shapes_are_versioned_and_carry_no_host_path() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 485);
    let request = LocalDaemonRequest::GrantAppFile(GrantAppFileRequest {
        session_id: "s".into(),
        operation_id: "file-pick-1".into(),
        files: vec![AppFileContents {
            name: "notes.md".into(),
            contents_base64: "IyBOb3Rlcw==".into(),
        }],
    });
    let response = LocalDaemonResponse::AppFileGranted {
        operation_id: "file-pick-1".into(),
        files: 1,
    };
    let encoded = serde_json::to_vec(&request).unwrap();
    assert_eq!(
        serde_json::from_slice::<LocalDaemonRequest>(&encoded).unwrap(),
        request
    );
    let snapshot = serde_json::json!({"request": request, "response": response});
    let digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
    );
    assert_eq!(
        digest,
        "5979e4d03cd0257191d08dba84097ccdf4cff74c25b6d1ec6a052c42d7a71ea6"
    );
    assert!(
        serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
            "GrantAppFile": {"session_id": "s", "operation_id": "o",
                "files": [{"name": "a.md", "contents_base64": "", "path": "/etc/passwd"}]}
        }))
        .is_err()
    );
}

#[test]
fn app_file_grant_revoke_shapes_are_versioned_and_name_no_owner() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 485);
    let all = LocalDaemonRequest::RevokeAppFileGrants(RevokeAppFileGrantsRequest {
        installation_id: "docs".into(),
        operation_id: None,
    });
    let one = LocalDaemonRequest::RevokeAppFileGrants(RevokeAppFileGrantsRequest {
        installation_id: "docs".into(),
        operation_id: Some("file-pick-1".into()),
    });
    let response = LocalDaemonResponse::AppFileGrantsRevoked {
        installation_id: "docs".into(),
        requests: 2,
        files: 1,
    };
    for request in [&all, &one] {
        assert_eq!(
            serde_json::from_slice::<LocalDaemonRequest>(&serde_json::to_vec(request).unwrap())
                .unwrap(),
            *request
        );
    }
    let snapshot = serde_json::json!({"all": all, "one": one, "response": response});
    assert_eq!(
        snapshot,
        serde_json::json!({
            "all": {"RevokeAppFileGrants": {"installation_id": "docs"}},
            "one": {"RevokeAppFileGrants": {"installation_id": "docs", "operation_id": "file-pick-1"}},
            "response": {"AppFileGrantsRevoked": {"installation_id": "docs", "requests": 2, "files": 1}},
        })
    );
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "64170cdc717596d0b5f0d30832fdaf9bfe3ef6c678b9feaddefe90a68598bd40"
    );
    // The owner comes from the caller; a request cannot name one.
    assert!(
        serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
            "RevokeAppFileGrants": {"installation_id": "docs", "owner_id": "mallory"}
        }))
        .is_err()
    );
}

#[test]
fn app_file_export_shapes_are_versioned_and_name_no_owner() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 485);
    let request = LocalDaemonRequest::SaveAppFileExport(SaveAppFileExportRequest {
        session_id: "s".into(),
        operation_id: "file-export-1".into(),
    });
    let response = LocalDaemonResponse::AppFileExport {
        operation_id: "file-export-1".into(),
        name: "plan.md".into(),
        contents_base64: "IyBQbGFu".into(),
    };
    let encoded = serde_json::to_vec(&request).unwrap();
    assert_eq!(
        serde_json::from_slice::<LocalDaemonRequest>(&encoded).unwrap(),
        request
    );
    let snapshot = serde_json::json!({"request": request, "response": response});
    let digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
    );
    assert_eq!(
        digest,
        "e7b467b9b22ebe9f61a6743fe4665c5157b997f34e849f050c9497f5625cc989"
    );
    assert!(
        serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
            "SaveAppFileExport": {"session_id": "s", "operation_id": "o", "owner_id": "bob"}
        }))
        .is_err()
    );
}

#[test]
fn app_view_reload_shape_is_versioned_and_names_only_the_tab_and_assets() {
    use crate::runtime::browser_controller_app_view::{BrowserAppViewAsset, BrowserAppViewRequest};
    use crate::transport::room_browser_controller::RoomBrowserControllerCommand;
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 485);
    let command = RoomBrowserControllerCommand::AppView {
        request: BrowserAppViewRequest::Reload {
            target_id: "target-1".into(),
            entry: "index.html".into(),
            assets: vec![BrowserAppViewAsset {
                path: "index.html".into(),
                content_type: "text/html".into(),
                body_base64: "PHA+PC9wPg==".into(),
            }],
        },
    };
    let digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&command).unwrap())
    );
    assert_eq!(
        digest,
        "97ef3e3e7ed8e932428508ed50da700f43b86eb33b1a0be9c3d6f66ace074240"
    );
}

#[test]
fn app_host_copy_link_shapes_are_versioned_and_acceptance_cannot_supply_an_owner_or_payload() {
    use crate::local::{AcceptAppHostActionRequest, AppHostAction};
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 485);
    let request = LocalDaemonRequest::AcceptAppHostAction(AcceptAppHostActionRequest {
        session_id: "s".into(),
        operation_id: "o".into(),
    });
    let copy = LocalDaemonResponse::AppHostActionAccepted {
        operation_id: "o".into(),
        action: AppHostAction::ClipboardWrite {
            text: "Hello\nworld".into(),
        },
    };
    let link = LocalDaemonResponse::AppHostActionAccepted {
        operation_id: "o".into(),
        action: AppHostAction::OpenLink {
            url: "https://example.org/a?x=%20".into(),
        },
    };
    let encoded = serde_json::to_vec(&request).unwrap();
    assert_eq!(
        serde_json::from_slice::<LocalDaemonRequest>(&encoded).unwrap(),
        request
    );
    let snapshot = serde_json::json!({"request":request,"copy":copy,"link":link});
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "032c05f915b459a97519a6fc1733f21d563346dad96360d94c3fa9a26b7c3dec"
    );
    for field in ["owner", "action", "url", "text"] {
        let mut request = serde_json::json!({"session_id":"s", "operation_id":"o"});
        request[field] = serde_json::json!("injected");
        assert!(serde_json::from_value::<LocalDaemonRequest>(
            serde_json::json!({"AcceptAppHostAction":request})
        )
        .is_err());
    }
    assert!(
        serde_json::from_value::<AppHostAction>(serde_json::json!({"kind":"clipboard_read"}))
            .is_err()
    );
}

#[test]
fn saved_snapshot_restore_protocol_410_shape_and_hash() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 485);
    let request = LocalDaemonRequest::RestoreAppDataSnapshot(RestoreAppDataSnapshotRequest {
        installation_id: "todo".into(),
        expected_generation: "3".into(),
        snapshot_id: "snapshot-1".into(),
    });
    let response = LocalDaemonResponse::AppDataSnapshotRestored {
        installation_id: "todo".into(),
        generation: "3".into(),
        snapshot_id: "snapshot-1".into(),
    };
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(serde_json::to_value(&request).unwrap())
            .unwrap(),
        request
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonResponse>(serde_json::to_value(&response).unwrap())
            .unwrap(),
        response
    );
    let shape = serde_json::json!({"request": request, "response": response});
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&shape).unwrap())),
        "c99920b83d0bfc050c3a7696b300460fc65fbe017d1007179e8144cfdf450035"
    );
    for field in ["owner_id", "path", "grant_id", "state"] {
        let mut bad = shape["request"].clone();
        bad["RestoreAppDataSnapshot"][field] = serde_json::json!("foreign");
        assert!(serde_json::from_value::<LocalDaemonRequest>(bad).is_err());
    }
}

#[test]
fn receipt_expiry_protocol_416_refuses_with_a_stable_error_shape() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 485);
    let response = LocalDaemonResponse::AppRequestFailed {
        code: AppRequestErrorCode::ReceiptExpired,
    };
    let value = serde_json::to_value(&response).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"AppRequestFailed": {"code": "receipt_expired"}})
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonResponse>(value.clone()).unwrap(),
        response
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&value).unwrap())),
        "e42f0d699c69420e3ae44790dd188ac2365d21078bc9013ec0049e6c543ae860"
    );
}
