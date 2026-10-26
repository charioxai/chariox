//! MP-08/MP-10/MP-11: A4's coordinator allocation, strict read-only shape.
use crate::local::*;
use chariox_app_runtime::view_catalog::{ViewCatalog, ViewWarning};
use serde_json::json;
use sha2::{Digest, Sha256};

#[test]
fn app_view_inspection_protocol_646_245_snapshot() {
    let request = LocalDaemonRequest::InspectAppView(InspectAppViewRequest {
        session_id: "room".into(),
        target_id: None,
    });
    let explicit = LocalDaemonRequest::InspectAppView(InspectAppViewRequest {
        session_id: "room".into(),
        target_id: Some("app-tab".into()),
    });
    for request in [&request, &explicit] {
        assert_eq!(
            *request,
            serde_json::from_value::<LocalDaemonRequest>(serde_json::to_value(request).unwrap())
                .unwrap()
        );
    }
    for extra in [
        "owner_id",
        "installation_id",
        "generation",
        "actions",
        "source",
    ] {
        let mut value = serde_json::to_value(&request).unwrap();
        value["InspectAppView"][extra] = json!("untrusted");
        assert!(serde_json::from_value::<LocalDaemonRequest>(value).is_err());
    }
    let response = LocalDaemonResponse::AppViewInspected {
        installation_id: "dev".into(),
        target_id: "app-tab".into(),
        generation: "9007199254740993".into(),
        development: true,
        catalog_json: serde_json::to_string(&ViewCatalog {
            actions: vec![],
            registrations: vec![],
            state: json!({"ok":true,"value":{"count":1}}),
            warnings: vec![ViewWarning {
                code: "undeclared_control".into(),
                message: "Visible control 'button#missing' has no matching declared action".into(),
            }],
        })
        .unwrap(),
    };
    let peer_command =
        crate::transport::room_browser_controller::RoomBrowserControllerCommand::AppView {
            request: crate::runtime::browser_controller_app_view::BrowserAppViewRequest::Inspect {
                target_id: "app-tab".into(),
            },
        };
    assert_eq!(
        peer_command,
        serde_json::from_value(serde_json::to_value(&peer_command).unwrap()).unwrap()
    );
    let snapshot = json!({"local_protocol":LOCAL_DAEMON_PROTOCOL_VERSION,"peer_protocol":crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,"requests":[request,explicit], "peer_command":peer_command,"response":response});
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("app-view-inspection-646.json")).unwrap();
    assert_eq!(snapshot, expected);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "a812af6ae5fba8838fd56eb684c2c0cea089b29c87d9e72cb991963c6a4495f7"
    );
    let command = crate::runtime::browser_controller_app_view::BrowserAppViewRequest::Inspect {
        target_id: "app-tab".into(),
    };
    assert_eq!(
        serde_json::to_value(&command).unwrap(),
        json!({"op":"inspect","target_id":"app-tab"})
    );
    assert_eq!(command.method(), "browser.app.inspect");
    assert_eq!(command.params(), json!({"target_id":"app-tab"}));
}
