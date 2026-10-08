use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn cloud_fixture(status: u16) -> (PersistedCloudRelayProfile, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let profile = PersistedCloudRelayProfile {
        api_url: format!("http://{}", listener.local_addr().unwrap()),
        kernel_credential: Some("synthetic-private-credential".into()),
        ..Default::default()
    };
    let server = tokio::spawn(async move {
        // A second response lets the old exporter fall back, exposing lost retryability.
        for code in [status, 403] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 16384];
            assert!(stream.read(&mut request).await.unwrap() > 0);
            let body =
                r#"{"error":{"code":"fixture_failure","message":"synthetic-private-payload"}}"#;
            stream.write_all(format!("HTTP/1.1 {code} Failure\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
    });
    (profile, server)
}

fn assert_safe_retry(error: DaemonError, expected: bool) {
    assert!(!error.to_string().contains("synthetic-private"));
    assert!(
        matches!(error, DaemonError::ManagedContext { retryable, .. } if retryable == expected)
    );
}

#[tokio::test]
async fn owner_cloud_temporary_http_and_connection_failures_remain_retryable() {
    for status in [408, 429, 500, 503, 403] {
        let (profile, server) = cloud_fixture(status).await;
        let result = kernel_cloud_request::<Directory>(
            &profile,
            "/v1/owner-managed-context-tickets/peers",
            None,
        )
        .await;
        server.abort();
        assert_safe_retry(result.err().unwrap(), status != 403);
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let profile = PersistedCloudRelayProfile {
        api_url: format!("http://{address}"),
        kernel_credential: Some("synthetic-private-credential".into()),
        ..Default::default()
    };
    assert_safe_retry(
        kernel_cloud_request::<Directory>(
            &profile,
            "/v1/owner-managed-context-tickets/peers",
            None,
        )
        .await
        .err()
        .unwrap(),
        true,
    );
}

#[tokio::test]
async fn owner_cloud_export_propagates_temporary_consumed_binding_failure() {
    for status in [429, 503] {
        let (mut profile, server) = cloud_fixture(status).await;
        profile.realm_id = "realm".into();
        profile.machine_id = Some("source-machine".into());
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(profile);
        let selection = super::super::owner_managed::OwnerManagedTransfer {
            target: ManagedContextTransferTarget {
                relay_realm_id: "realm".into(),
                machine_id: "target-machine".into(),
                kernel_id: "target-kernel".into(),
                relay_public_key: "public-target".into(),
                key_thumbprint: "public-thumbprint".into(),
            },
            context_selection: super::super::owner_managed::OwnerManagedContextSelection {
                kernel_context: super::super::owner_managed::OwnerManagedKernelSelection::Empty,
                development_setup:
                    super::super::owner_managed::OwnerManagedDevelopmentSelection::Empty,
            },
        };
        let ticket = ManagedContextTransferTicket {
            environment_id: String::new(),
            target: selection.target.clone(),
            context_plan: crate::managed_bootstrap::ManagedKernelContextPlan::for_owner_managed(
                &config, &selection,
            )
            .unwrap(),
        };
        let error = authorize_export(&config, &ticket)
            .await
            .err()
            .expect("outage must not authorize");
        server.abort();
        assert_safe_retry(error, true);
    }
}
