//! MP-08 / MP-10 protocol 437 snapshot/hash and authority-shape guard.
use crate::local::*;
use sha2::{Digest, Sha256};
#[test]
fn workflow_notification_437_shapes_have_no_caller_owner_or_ancestry() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let register = LocalDaemonRequest::RegisterWorkflowNotificationSource(
        RegisterWorkflowNotificationSourceRequest {
            session_id: "s".into(),
            workflow_ref: "w".into(),
            enabled: true,
            exit_node_id: "exit-1".into(),
        },
    );
    let attach =
        LocalDaemonRequest::AttachWorkflowNotification(AttachWorkflowNotificationRequest {
            session_id: "t".into(),
            source_id: "src".into(),
            publication_ref: "p".into(),
        });
    let list = LocalDaemonRequest::ListWorkflowNotifications(ListWorkflowNotificationsRequest {
        session_id: "t".into(),
    });
    let source = WorkflowNotificationSource {
        kind: crate::local::NotificationSourceKind::WorkflowExit,
        source_id: "src".into(),
        owner_user_id: "u".into(),
        kernel_id: "k".into(),
        session_id: "s".into(),
        workflow_id: "w".into(),
        exit_node_id: "node-2".into(),
        enabled: true,
        available: true,
        name: "reviewer".into(),
        exit_ordinal: 2,
        display_name: "reviewer-2".into(),
    };
    let sub = WorkflowNotificationSubscription {
        subscription_id: "sub".into(),
        source_id: "src".into(),
        owner_user_id: "u".into(),
        created_by_user_id: "u".into(),
        target_kernel_id: "k".into(),
        target_kind: WorkflowNotificationTargetKind::WorkflowEndpoint,
        session_id: "t".into(),
        workflow_id: "v".into(),
        publication_id: "p".into(),
        endpoint_id: "e".into(),
        queue_id: "q".into(),
        source_available: true,
        source_kernel_id: "k".into(),
        agent_id: None,
        trigger_id: None,
    };
    let agent_sub = WorkflowNotificationSubscription {
        subscription_id: "agent-sub".into(),
        target_kind: WorkflowNotificationTargetKind::Agent,
        workflow_id: String::new(),
        publication_id: "trigger".into(),
        endpoint_id: "agent".into(),
        agent_id: Some("agent".into()),
        trigger_id: Some("trigger".into()),
        ..sub.clone()
    };
    let env = WorkflowNotificationEnvelope {
        source_id: "src".into(),
        display_name: "reviewer-2".into(),
        occurrence_id: "run".into(),
        output: crate::session::WorkflowOutputPayload::new("output", vec![]),
        ancestry: vec!["[\"k\",\"s\",\"w\"]".into()],
        deadline_ms: 604800001,
    };
    let responses = vec![
        LocalDaemonResponse::WorkflowNotificationDetached {
            subscription_id: "sub".into(),
        },
        LocalDaemonResponse::WorkflowNotificationSourceRegistered {
            source: source.clone(),
        },
        LocalDaemonResponse::WorkflowNotificationAttached {
            subscription: sub.clone(),
        },
        LocalDaemonResponse::WorkflowNotifications {
            sources: vec![crate::local::WorkflowNotificationSourceSummary {
                kind: crate::local::NotificationSourceKind::WorkflowExit,
                source_id: source.source_id,
                kernel_id: source.kernel_id,
                session_id: source.session_id,
                workflow_id: source.workflow_id,
                exit_node_id: source.exit_node_id,
                exit_ordinal: source.exit_ordinal,
                name: source.name,
                display_name: source.display_name,
                available: true,
            }],
            subscriptions: vec![sub.clone(), agent_sub],
            listeners: vec![sub],
            diagnostics: vec![WorkflowNotificationDiagnostic {
                source_id: "src".into(),
                occurrence_id: "run".into(),
                code: "workflow_notification_loop_dropped".into(),
            }],
        },
    ];
    let wire = serde_json::json!({"requests":[register,attach,list,LocalDaemonRequest::DetachWorkflowNotification(DetachWorkflowNotificationRequest {session_id:"t".into(),subscription_id:"sub".into()})],"responses":responses,"envelope":env,"acks":[WorkflowNotificationAck::Accepted,WorkflowNotificationAck::Duplicate,WorkflowNotificationAck::Expired,WorkflowNotificationAck::LoopDropped]});
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&wire).unwrap())),
        "56864f4ded54b2efa2692e303e71bc5ca9487136540bffd84932b43640d1326c"
    );
    for field in ["owner_user_id", "ancestry", "output"] {
        let mut request =
            serde_json::json!({"session_id":"t","source_id":"src","publication_ref":"p"});
        request[field] = serde_json::json!("forged");
        assert!(serde_json::from_value::<LocalDaemonRequest>(
            serde_json::json!({"AttachWorkflowNotification":request})
        )
        .is_err());
    }
    // A subscription has no options: every former option field is an unknown field.
    for (field, value) in [
        ("delivery_mode", serde_json::json!("inject")),
        ("events", serde_json::json!("both")),
        ("filters", serde_json::json!({})),
        ("ttl_days", serde_json::json!(7)),
        ("queue_ref", serde_json::json!("q")),
    ] {
        let mut request =
            serde_json::json!({"session_id":"t","source_id":"src","publication_ref":"p"});
        request[field] = value;
        assert!(
            serde_json::from_value::<LocalDaemonRequest>(
                serde_json::json!({"AttachWorkflowNotification":request})
            )
            .is_err(),
            "{field} must be refused"
        );
    }
    assert!(serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
        "RegisterWorkflowNotificationSource":{"session_id":"t","workflow_ref":"w","exit_node_id":"exit-1","enabled":true,"output_fields":["a"]}
    }))
    .is_err());
    assert!(
        serde_json::from_value::<WorkflowNotificationTargetKind>(serde_json::json!(
            "agent_session"
        ))
        .is_err()
    );
    let decoded:LocalDaemonRequest=serde_json::from_value(serde_json::json!({"AttachWorkflowNotification":{"session_id":"t","source_id":"src","publication_ref":"p"}})).unwrap();
    assert!(matches!(
        decoded,
        LocalDaemonRequest::AttachWorkflowNotification(AttachWorkflowNotificationRequest { .. })
    ));
}
