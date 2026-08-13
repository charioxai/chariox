use chariox_aegs_sdk::CreateAuthorizationRequest;

#[test]
fn create_authorization_request_is_public() {
    let request = CreateAuthorizationRequest {
        state_digest: "state",
        connection_id: "connection",
        owner_id: "owner",
        provider: "provider",
        return_url: Some("https://example.com/callback"),
        expires_at_ms: 2,
        now_ms: 1,
    };

    assert_eq!(request.connection_id, "connection");
}
