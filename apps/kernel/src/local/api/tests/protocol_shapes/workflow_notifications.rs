//! MP-08 / MP-10 protocol 437 snapshot/hash and authority-shape guard.
use crate::local::*;
use sha2::{Digest, Sha256};
#[test]
fn workflow_notification_437_shapes_have_no_caller_owner_or_ancestry() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 437);
    let register = LocalDaemonRequest::RegisterWorkflowNotificationSource(
        RegisterWorkflowNotificationSourceRequest {
            session_id: "s".into(),
            workflow_ref: "w".into(),
            enabled: true,
        },
    );
    let attach =
        LocalDaemonRequest::AttachWorkflowNotification(AttachWorkflowNotificationRequest {
            session_id: "t".into(),
            source_id: "src".into(),
            publication_ref: "p".into(),
            queue_ref: None,
            ttl_days: 7,
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
    };
    let sub = WorkflowNotificationSubscription {
        subscription_id: "sub".into(),
        source_id: "src".into(),
        owner_user_id: "u".into(),
        target_kernel_id: "k".into(),
        session_id: "t".into(),
        workflow_id: "v".into(),
        publication_id: "p".into(),
        endpoint_id: "e".into(),
        queue_id: "q".into(),
        ttl_days: 7,
        source_available: true,
    };
    let env = WorkflowNotificationEnvelope {
        source_id: "src".into(),
        occurrence_id: "run".into(),
        output: crate::session::WorkflowOutputPayload::new("output", vec![]),
        ancestry: vec!["[\"k\",\"s\",\"w\"]".into()],
        deadline_ms: 604800001,
    };
    let responses = vec![
        LocalDaemonResponse::WorkflowNotificationSourceRegistered {
            source: source.clone(),
        },
        LocalDaemonResponse::WorkflowNotificationAttached {
            subscription: sub.clone(),
        },
        LocalDaemonResponse::WorkflowNotifications {
            sources: vec![source],
            subscriptions: vec![sub],
            diagnostics: vec![WorkflowNotificationDiagnostic {
                source_id: "src".into(),
                occurrence_id: "run".into(),
                code: "workflow_notification_loop_dropped".into(),
            }],
        },
    ];
    let wire = serde_json::json!({"requests":[register,attach,list],"responses":responses,"envelope":env,"acks":[WorkflowNotificationAck::Accepted,WorkflowNotificationAck::Duplicate,WorkflowNotificationAck::Expired]});
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&wire).unwrap())),
        "3bede980abb2156011cb846c293080db6ed001326fd441a76d87b452e71ffce8"
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
    let decoded:LocalDaemonRequest=serde_json::from_value(serde_json::json!({"AttachWorkflowNotification":{"session_id":"t","source_id":"src","publication_ref":"p"}})).unwrap();
    assert!(matches!(
        decoded,
        LocalDaemonRequest::AttachWorkflowNotification(AttachWorkflowNotificationRequest {
            ttl_days: 7,
            ..
        })
    ));
}
