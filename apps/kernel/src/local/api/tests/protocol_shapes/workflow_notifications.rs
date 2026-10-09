//! MP-08 / MP-10 protocol 437 snapshot/hash and authority-shape guard.
use crate::local::*;
use sha2::{Digest, Sha256};
#[test]
fn workflow_notification_437_shapes_have_no_caller_owner_or_ancestry() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 484);
    let register = LocalDaemonRequest::RegisterWorkflowNotificationSource(
        RegisterWorkflowNotificationSourceRequest {
            session_id: "s".into(),
            workflow_ref: "w".into(),
            enabled: true,
            output_fields: None,
        },
    );
    let attach =
        LocalDaemonRequest::AttachWorkflowNotification(AttachWorkflowNotificationRequest {
            delivery_mode: crate::local::NotificationDeliveryMode::Queue,
            session_id: "t".into(),
            source_id: "src".into(),
            publication_ref: "p".into(),
            queue_ref: None,
            ttl_days: 7,
            events: WorkflowNotificationEvents::Both,
            filters: serde_json::Value::Null,
        });
    let list = LocalDaemonRequest::ListWorkflowNotifications(ListWorkflowNotificationsRequest {
        session_id: "t".into(),
    });
    let source = WorkflowNotificationSource {
        source_id: "src".into(),
        owner_user_id: "u".into(),
        kernel_id: "k".into(),
        session_id: "s".into(),
        workflow_id: "w".into(),
        enabled: true,
        available: true,
        name: "reviewer".into(),
        output_fields: vec![],
    };
    let sub = WorkflowNotificationSubscription {
        delivery_mode: crate::local::NotificationDeliveryMode::Queue,
        subscription_id: "sub".into(),
        source_id: "src".into(),
        owner_user_id: "u".into(),
        target_kernel_id: "k".into(),
        target_kind: WorkflowNotificationTargetKind::WorkflowEndpoint,
        session_id: "t".into(),
        workflow_id: "v".into(),
        publication_id: "p".into(),
        endpoint_id: "e".into(),
        queue_id: "q".into(),
        ttl_days: 7,
        source_available: true,
        source_kernel_id: "k".into(),
        events: WorkflowNotificationEvents::Both,
        filters: serde_json::Value::Null,
    };
    let env = WorkflowNotificationEnvelope {
        source_id: "src".into(),
        occurrence_id: "run".into(),
        output: Some(crate::session::WorkflowOutputPayload::new("output", vec![])),
        status: WorkflowNotificationStatus::Success,
        subject: None,
        fields: serde_json::json!({"status":"success"}),
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
                source_id: source.source_id,
                kernel_id: source.kernel_id,
                session_id: source.session_id,
                workflow_id: source.workflow_id,
                name: source.name,
                events: WorkflowNotificationEvents::Both,
                fields: vec![],
                available: true,
            }],
            subscriptions: vec![sub],
            diagnostics: vec![WorkflowNotificationDiagnostic {
                source_id: "src".into(),
                occurrence_id: "run".into(),
                code: "workflow_notification_loop_dropped".into(),
            }],
        },
    ];
    let wire = serde_json::json!({"requests":[register,attach,list,LocalDaemonRequest::DetachWorkflowNotification(DetachWorkflowNotificationRequest {session_id:"t".into(),subscription_id:"sub".into()})],"responses":responses,"envelope":env,"acks":[WorkflowNotificationAck::Accepted,WorkflowNotificationAck::Duplicate,WorkflowNotificationAck::Expired,WorkflowNotificationAck::Filtered,WorkflowNotificationAck::LoopDropped]});
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&wire).unwrap())),
        "fc9a63afb17e25eb759afa6ab4872d6cb55a21bca7dc3a8bfa4d9e9fc3cf466d"
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
    let injection: LocalDaemonRequest = serde_json::from_value(serde_json::json!({"AttachWorkflowNotification":{"session_id":"t","source_id":"src","publication_ref":"p","delivery_mode":"inject"}})).unwrap();
    assert!(matches!(
        injection,
        LocalDaemonRequest::AttachWorkflowNotification(AttachWorkflowNotificationRequest {
            delivery_mode: NotificationDeliveryMode::Inject,
            events: WorkflowNotificationEvents::Success,
            ..
        })
    ));
    assert!(
        serde_json::from_value::<WorkflowNotificationTargetKind>(serde_json::json!(
            "agent_session"
        ))
        .is_err()
    );
    let decoded:LocalDaemonRequest=serde_json::from_value(serde_json::json!({"AttachWorkflowNotification":{"session_id":"t","source_id":"src","publication_ref":"p"}})).unwrap();
    assert!(matches!(
        decoded,
        LocalDaemonRequest::AttachWorkflowNotification(AttachWorkflowNotificationRequest {
            ttl_days: 7,
            ..
        })
    ));
}
