use super::*;

fn refused_by_cloud(code: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "cloud relay request",
        message: format!("cloud relay request failed with 429: cloud_api_code={code}: refused"),
    }
}

#[test]
fn a_permanent_cloud_refusal_is_not_retryable_over_the_relay() {
    for code in ["budget_exceeded", "authorization_denied", "invalid_request"] {
        let mapped = map_relay_error(&refused_by_cloud(code));
        assert_eq!(mapped.code, "transport_error", "{code}");
        assert!(!mapped.retryable, "{code}");
    }
}

#[test]
fn a_transient_cloud_refusal_stays_retryable_over_the_relay() {
    for code in [
        "capacity_exceeded",
        "rate_limited",
        "dependency_unavailable",
    ] {
        assert!(map_relay_error(&refused_by_cloud(code)).retryable, "{code}");
    }
}
