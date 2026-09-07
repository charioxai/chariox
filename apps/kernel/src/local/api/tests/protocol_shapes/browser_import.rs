use super::*;
use crate::local::{
    ApproveBrowserImportRequest, BrowserImportConsentStatus, BrowserImportSelection,
    BrowserImportSourceRequest, CancelBrowserImportRequest, PrepareBrowserImportRequest,
};

#[test]
fn browser_import_relay_response_binds_the_encrypted_request_nonce() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 317);
    let response = crate::transport::kernel_protocol::BrowserImportRelayResponse {
        request_nonce: "AAAAAAAAAAAAAAAA".into(),
        response: serde_json::json!({"BrowserImportConsent":{"request_id":"fixture","status":"prepared"}}),
    };
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        serde_json::json!({
            "request_nonce":"AAAAAAAAAAAAAAAA",
            "response":{"BrowserImportConsent":{"request_id":"fixture","status":"prepared"}}
        })
    );
}

#[test]
fn browser_import_consent_protocol_shape_is_versioned_and_excludes_cookie_payloads() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 317);
    let selection = BrowserImportSelection {
        session_id: "room-1".into(),
        attachment_id: "attachment-1".into(),
        environment_id: "environment-1".into(),
        runtime_generation: 2,
        tab_id: "tab-1".into(),
        document_revision: 3,
        source_store_id: "0".into(),
        domains: vec!["example.com".into()],
        partition_sites: vec![],
        overwrite: false,
    };
    let requests = [
        LocalDaemonRequest::PrepareBrowserImport(PrepareBrowserImportRequest {
            selection: selection.clone(),
        }),
        LocalDaemonRequest::ApproveBrowserImport(ApproveBrowserImportRequest {
            request_id: "request-1".into(),
            selection: selection.clone(),
        }),
        LocalDaemonRequest::ClaimBrowserImportSource(BrowserImportSourceRequest {
            request_id: "request-1".into(),
            selection: selection.clone(),
        }),
        LocalDaemonRequest::AuthorizeBrowserImportSource(BrowserImportSourceRequest {
            request_id: "request-1".into(),
            selection,
        }),
        LocalDaemonRequest::CancelBrowserImport(CancelBrowserImportRequest {
            session_id: "room-1".into(),
            attachment_id: "attachment-1".into(),
            request_id: "request-1".into(),
        }),
    ];
    let expected: serde_json::Value = serde_json::from_str(r#"[
      {"PrepareBrowserImport":{"selection":{"session_id":"room-1","attachment_id":"attachment-1","environment_id":"environment-1","runtime_generation":2,"tab_id":"tab-1","document_revision":3,"source_store_id":"0","domains":["example.com"],"partition_sites":[],"overwrite":false}}},
      {"ApproveBrowserImport":{"request_id":"request-1","selection":{"session_id":"room-1","attachment_id":"attachment-1","environment_id":"environment-1","runtime_generation":2,"tab_id":"tab-1","document_revision":3,"source_store_id":"0","domains":["example.com"],"partition_sites":[],"overwrite":false}}},
      {"ClaimBrowserImportSource":{"request_id":"request-1","selection":{"session_id":"room-1","attachment_id":"attachment-1","environment_id":"environment-1","runtime_generation":2,"tab_id":"tab-1","document_revision":3,"source_store_id":"0","domains":["example.com"],"partition_sites":[],"overwrite":false}}},
      {"AuthorizeBrowserImportSource":{"request_id":"request-1","selection":{"session_id":"room-1","attachment_id":"attachment-1","environment_id":"environment-1","runtime_generation":2,"tab_id":"tab-1","document_revision":3,"source_store_id":"0","domains":["example.com"],"partition_sites":[],"overwrite":false}}},
      {"CancelBrowserImport":{"session_id":"room-1","attachment_id":"attachment-1","request_id":"request-1"}}
    ]"#).unwrap();
    assert_eq!(serde_json::to_value(&requests).unwrap(), expected);
    for (request, expected) in requests.iter().zip(expected.as_array().unwrap()) {
        assert_eq!(
            &serde_json::from_value::<LocalDaemonRequest>(expected.clone()).unwrap(),
            request
        );
    }
    for (status, wire) in [
        (BrowserImportConsentStatus::Prepared, "prepared"),
        (BrowserImportConsentStatus::Approved, "approved"),
        (BrowserImportConsentStatus::SourceClaimed, "source_claimed"),
        (
            BrowserImportConsentStatus::SourceAuthorized,
            "source_authorized",
        ),
        (BrowserImportConsentStatus::Cancelled, "cancelled"),
    ] {
        assert_eq!(
            serde_json::to_value(LocalDaemonResponse::BrowserImportConsent {
                request_id: "request-1".into(),
                status
            })
            .unwrap(),
            serde_json::json!({"BrowserImportConsent":{"request_id":"request-1","status":wire}})
        );
    }
    let mut bad = expected[0].clone();
    bad["PrepareBrowserImport"]["selection"]["cookies"] =
        serde_json::json!([{"value":"must-not-travel"}]);
    assert!(serde_json::from_value::<LocalDaemonRequest>(bad).is_err());
    for (index, kind) in [
        (2, "ClaimBrowserImportSource"),
        (3, "AuthorizeBrowserImportSource"),
    ] {
        let mut bad = expected[index].clone();
        bad[kind]["selection"]["cookies"] = serde_json::json!([{"value":"must-not-travel"}]);
        assert!(serde_json::from_value::<LocalDaemonRequest>(bad).is_err());
        let mut bad = expected[index].clone();
        bad[kind]["approved"] = serde_json::json!(true);
        assert!(serde_json::from_value::<LocalDaemonRequest>(bad).is_err());
    }
    let mut bad = expected[0].clone();
    bad["PrepareBrowserImport"]["approved"] = serde_json::json!(true);
    assert!(serde_json::from_value::<LocalDaemonRequest>(bad).is_err());
}
