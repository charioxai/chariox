//! MP-08 / MP-10 / MP-11: bootstrap waits on home launch receipt, never tool replay.

use std::{future::Future, time::Duration};

use crate::error::DaemonError;

pub(super) async fn wait_for_native_binding<T, F, Fut>(
    native: bool,
    payload: &serde_json::Value,
    budget: Duration,
    mut send: F,
) -> Result<T, DaemonError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, DaemonError>>,
{
    let bootstrap = native
        && matches!(
            payload.get("method").and_then(serde_json::Value::as_str),
            Some("initialize" | "notifications/initialized" | "tools/list")
        );
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        match send().await {
            Err(error)
                if bootstrap
                    && matches!(&error, DaemonError::RelayTransport {
                        operation: "read relay peer response", code, retryable: true, ..
                    } if code == "native_provider_binding_pending") =>
            {
                let now = tokio::time::Instant::now();
                if now >= deadline {
                    return Err(error);
                }
                tokio::time::sleep(Duration::from_millis(100).min(deadline - now)).await;
                if tokio::time::Instant::now() >= deadline {
                    return Err(error);
                }
            }
            result => return result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn binding_not_ready() -> DaemonError {
        DaemonError::RelayTransport {
            operation: "read relay peer response",
            code: "native_provider_binding_pending".into(),
            message: "local transport `authorize forwarded home context` failed: worker provider run does not match active remote agent binding".into(),
            retryable: true,
        }
    }

    #[tokio::test]
    async fn mp08_mp10_native_bootstrap_waits_for_home_receipt() {
        let attempts = Cell::new(0);
        let result = wait_for_native_binding(
            true,
            &serde_json::json!({"method":"initialize"}),
            Duration::from_secs(1),
            || {
                attempts.set(attempts.get() + 1);
                std::future::ready(if attempts.get() < 3 {
                    Err(binding_not_ready())
                } else {
                    Ok("home binding committed")
                })
            },
        )
        .await;
        assert_eq!(result.unwrap(), "home binding committed");
        assert_eq!(attempts.get(), 3);
    }

    #[tokio::test]
    async fn mp11_tool_calls_and_ordinary_clients_are_never_replayed() {
        for (native, method) in [(true, "tools/call"), (false, "initialize")] {
            let attempts = Cell::new(0);
            let result: Result<(), _> = wait_for_native_binding(
                native,
                &serde_json::json!({"method":method}),
                Duration::from_secs(1),
                || {
                    attempts.set(attempts.get() + 1);
                    std::future::ready(Err(binding_not_ready()))
                },
            )
            .await;
            assert!(result.is_err());
            assert_eq!(attempts.get(), 1);
        }
    }

    #[tokio::test]
    async fn mp11_bootstrap_retry_uses_a_code_instead_of_diagnostic_wording() {
        let attempts = Cell::new(0);
        let result = wait_for_native_binding(
            true,
            &serde_json::json!({"method":"initialize"}),
            Duration::from_secs(1),
            || {
                attempts.set(attempts.get() + 1);
                std::future::ready(if attempts.get() == 1 {
                    Err(DaemonError::RelayTransport {
                        operation: "read relay peer response",
                        code: "native_provider_binding_pending".into(),
                        message: "A revised human diagnostic".into(),
                        retryable: true,
                    })
                } else {
                    Ok(())
                })
            },
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(attempts.get(), 2);
    }

    #[tokio::test]
    async fn mp11_unrelated_authorization_errors_are_never_retried() {
        let attempts = Cell::new(0);
        let result: Result<(), _> = wait_for_native_binding(
            true,
            &serde_json::json!({"method":"tools/list"}),
            Duration::from_secs(1),
            || {
                attempts.set(attempts.get() + 1);
                std::future::ready(Err(DaemonError::RelayTransport {
                    operation: "read relay peer response",
                    code: "transport_error".into(),
                    message: "worker kernel does not match home agent binding".into(),
                    retryable: true,
                }))
            },
        )
        .await;
        assert!(result.is_err());
        assert_eq!(attempts.get(), 1);
    }

    #[tokio::test]
    async fn mp11_stale_run_binding_is_rejected_after_bounded_bootstrap_wait() {
        let attempts = Cell::new(0);
        let started = tokio::time::Instant::now();
        let result: Result<(), _> = wait_for_native_binding(
            true,
            &serde_json::json!({"method":"initialize"}),
            Duration::from_millis(150),
            || {
                attempts.set(attempts.get() + 1);
                std::future::ready(Err(binding_not_ready()))
            },
        )
        .await;
        assert!(result.is_err());
        assert!(attempts.get() > 1);
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
