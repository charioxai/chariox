use super::*;

#[test]
fn slice_creation_preserves_explicit_display_backend_on_the_wire() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 280);
    let request: LocalDaemonRequest = serde_json::from_value(serde_json::json!({
        "CreateSlice": {
            "name": "headed", "display_mode": "headed", "display_backend": "selkies"
        }
    }))
    .expect("Selkies slice request should decode");
    let serialized = serde_json::to_value(request).expect("request should encode");
    assert_eq!(
        serialized.pointer("/CreateSlice/display_backend"),
        Some(&serde_json::json!("selkies"))
    );
}

#[test]
fn legacy_slice_requests_keep_novnc_and_unknown_backends_fail_closed() {
    let request: crate::local::CreateSliceRequest =
        serde_json::from_value(serde_json::json!({"name": "legacy", "display_mode": "headed"}))
            .expect("legacy request should decode");
    assert_eq!(
        request.display_backend,
        crate::slice::SliceDisplayBackend::Novnc
    );
    assert!(serde_json::to_value(request)
        .unwrap()
        .get("display_backend")
        .is_none());
    assert!(serde_json::from_value::<crate::local::CreateSliceRequest>(
        serde_json::json!({"name": "invalid", "display_backend": "unknown"}),
    )
    .is_err());
}

#[test]
fn local_daemon_protocol_selkies_endpoint_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 280);
    let response = LocalDaemonResponse::SliceDisplayEndpoint {
        endpoint: crate::slice::SliceDisplayEndpoint {
            slice_id: "slice-1".to_string(),
            kind: crate::slice::SliceDisplayEndpointKind::Selkies,
            url: "http://127.0.0.1:45500/".to_string(),
            access: crate::slice::SliceDisplayEndpointAccess::Local,
            expires_at_ms: None,
            capabilities: vec!["view", "websocket", "h264", "software_encoding"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        },
    };
    let value = serde_json::to_value(response).expect("endpoint should encode");
    let roundtrip: LocalDaemonResponse =
        serde_json::from_value(value.clone()).expect("endpoint should decode");
    assert!(matches!(
        roundtrip,
        LocalDaemonResponse::SliceDisplayEndpoint { .. }
    ));
    let serialized = serde_json::to_string(&value).unwrap();
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "63d4a1fab36398178f7b3c4d984409f4f1ba3dedb7a5da5512693914e2d1de02"
    );
}
