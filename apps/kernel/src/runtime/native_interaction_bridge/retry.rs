//! MP-08 / MP-10 / MP-11: a transport outage is not a permission decision.
use super::*;
use crate::transport::relay_peer::RelayPeerResponse;

pub(super) async fn retry_forwarded_native_interaction<F, Fut>(
    response_timeout: Duration,
    mut send: F,
) -> Result<RelayPeerResponse, DaemonError>
where
    F: FnMut(Duration) -> Fut,
    Fut: std::future::Future<Output = Result<RelayPeerResponse, DaemonError>>,
{
    let deadline = tokio::time::Instant::now() + response_timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout_at(deadline, send(remaining)).await {
            Ok(Ok(response)) => return Ok(response),
            Ok(Err(error)) if transport_is_recoverable(&error) => {
                tokio::select! {
                    _ = tokio::time::sleep_until(deadline) => break,
                    _ = tokio::time::sleep(Duration::from_millis(500)) => {},
                }
            }
            Ok(Err(error)) => return Err(error),
            Err(_) => break,
        }
    }
    Ok(RelayPeerResponse::NativeInteractionResolved {
        resolution: ProviderNativeInteractionResolution {
            status: "timed_out".into(),
            choice_id: None,
            reply: None,
        },
    })
}

fn transport_is_recoverable(error: &DaemonError) -> bool {
    match error {
        DaemonError::RelayTransport { retryable, .. } => *retryable,
        DaemonError::LocalTransport { operation, message } => {
            if *operation == "get_live_kernel" {
                return message.contains("not currently visible on relay")
                    || message.contains("did not appear on relay");
            }
            // These legacy transport seams wrap socket errors as LocalTransport.
            // Never retry authentication, decoding, lease or admission refusals.
            matches!(
                *operation,
                "connect temporary relay peer socket"
                    | "write temporary relay register"
                    | "write temporary relay peer request"
                    | "read temporary relay peer response"
                    | "connect relay metadata socket"
                    | "write relay metadata request"
                    | "read relay metadata response"
            ) && {
                let message = message.to_ascii_lowercase();
                [
                    "connection reset",
                    "connection closed",
                    "connection abort",
                    "closed temporary peer connection",
                    "closed metadata connection",
                    "broken pipe",
                    "connection refused",
                    "timed out",
                    "network is unreachable",
                    "no route to host",
                ]
                .iter()
                .any(|reason| message.contains(reason))
            }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reset() -> DaemonError {
        DaemonError::LocalTransport {
            operation: "read temporary relay peer response",
            message: "WebSocket protocol error: Connection reset without closing handshake".into(),
        }
    }

    #[tokio::test]
    async fn native_permission_transport_retries_resets_and_keeps_the_response() {
        let expected = ProviderNativeInteractionResolution {
            status: "answered".into(),
            choice_id: Some("allow_once".into()),
            reply: None,
        };
        let mut attempts = 0;
        let mut budgets = Vec::new();
        let response = retry_forwarded_native_interaction(Duration::from_secs(3), |remaining| {
            attempts += 1;
            budgets.push(remaining);
            std::future::ready(if attempts < 3 {
                Err(reset())
            } else {
                Ok(RelayPeerResponse::NativeInteractionResolved {
                    resolution: expected.clone(),
                })
            })
        })
        .await
        .unwrap();
        let RelayPeerResponse::NativeInteractionResolved { resolution } = response else {
            panic!("native response")
        };
        assert_eq!(resolution, expected);
        assert_eq!(attempts, 3);
        assert!(budgets.windows(2).all(|pair| pair[1] < pair[0]));
    }

    #[tokio::test]
    async fn native_permission_transport_outage_ends_at_the_original_deadline() {
        let started = tokio::time::Instant::now();
        let response = retry_forwarded_native_interaction(Duration::from_millis(650), |_| {
            std::future::ready(Err(reset()))
        })
        .await
        .unwrap();
        let RelayPeerResponse::NativeInteractionResolved { resolution } = response else {
            panic!("native response")
        };
        assert_eq!(resolution.status, "timed_out");
        assert_eq!(resolution.choice_id, None);
        assert!(started.elapsed() < Duration::from_millis(1000));
    }

    #[tokio::test]
    async fn native_permission_transport_does_not_retry_authority_or_decode_errors() {
        for operation in [
            "authenticate temporary relay peer response",
            "decode temporary relay peer response",
            "relay_forward_native_interaction",
        ] {
            let mut attempts = 0;
            let result = retry_forwarded_native_interaction(Duration::from_secs(3), |_| {
                attempts += 1;
                std::future::ready(Err(DaemonError::LocalTransport {
                    operation,
                    message: "connection reset is not authority".into(),
                }))
            })
            .await;
            assert!(result.is_err());
            assert_eq!(attempts, 1);
        }
        assert!(!transport_is_recoverable(&DaemonError::RelayTransport {
            operation: "read temporary relay peer response",
            code: "peer_forbidden".into(),
            message: "permission denied".into(),
            retryable: false,
        }));
    }
}
