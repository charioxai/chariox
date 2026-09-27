//! Compatibility, credential, and stale-binding policy for remote native provider launches.

use std::future::Future;

use crate::agent::RemoteAgentBinding;
use crate::error::DaemonError;
use crate::transport::relay_peer::RemoteProviderLaunchCredential;

pub(super) async fn launch_with_one_binding_refresh<
    Response,
    ResolveCredential,
    ResolveCredentialFuture,
    Send,
    SendFuture,
    Refresh,
    RefreshFuture,
>(
    initial_binding: RemoteAgentBinding,
    mut resolve_credential: ResolveCredential,
    mut send: Send,
    mut refresh: Refresh,
) -> Result<(Response, RemoteAgentBinding), DaemonError>
where
    ResolveCredential: FnMut() -> ResolveCredentialFuture,
    ResolveCredentialFuture:
        Future<Output = Result<Option<RemoteProviderLaunchCredential>, DaemonError>>,
    Send: FnMut(RemoteAgentBinding, Option<RemoteProviderLaunchCredential>) -> SendFuture,
    SendFuture: Future<Output = Result<Response, DaemonError>>,
    Refresh: FnMut() -> RefreshFuture,
    RefreshFuture: Future<Output = Result<RemoteAgentBinding, DaemonError>>,
{
    ensure_compatible_binding(&initial_binding)?;
    let skipped_credential_for_active_run = initial_binding.active_worker_provider_run_id.is_some();
    let credential = credential_for_binding(&initial_binding, &mut resolve_credential).await?;
    let mut initial_result = send(initial_binding.clone(), credential).await;
    if skipped_credential_for_active_run && launch_requires_provider_credential(&initial_result) {
        let credential = resolve_credential().await?;
        initial_result = send(initial_binding.clone(), credential).await;
    }
    match initial_result {
        Ok(response) => Ok((response, initial_binding)),
        Err(error)
            if super::remote_prompt_worker_submission_runtime::remote_prompt_error_should_refresh_binding(
                &error,
            ) =>
        {
            let refreshed_binding = refresh().await?;
            ensure_compatible_binding(&refreshed_binding)?;
            let skipped_credential_for_active_run =
                refreshed_binding.active_worker_provider_run_id.is_some();
            let credential =
                credential_for_binding(&refreshed_binding, &mut resolve_credential).await?;
            let mut refreshed_result = send(refreshed_binding.clone(), credential).await;
            if skipped_credential_for_active_run
                && launch_requires_provider_credential(&refreshed_result)
            {
                let credential = resolve_credential().await?;
                refreshed_result = send(refreshed_binding.clone(), credential).await;
            }
            let response = refreshed_result?;
            Ok((response, refreshed_binding))
        }
        Err(error) => Err(error),
    }
}

fn launch_requires_provider_credential<Response>(result: &Result<Response, DaemonError>) -> bool {
    let Err(error) = result else {
        return false;
    };
    let required_code =
        crate::transport::relay_peer::REMOTE_PROVIDER_LAUNCH_CREDENTIAL_REQUIRED_CODE;
    match error {
        DaemonError::LocalTransport { message, .. } => message.contains(required_code),
        DaemonError::RelayTransport { code, message, .. } => {
            code == required_code || message.contains(required_code)
        }
        _ => false,
    }
}

pub(super) fn ensure_compatible_binding(binding: &RemoteAgentBinding) -> Result<(), DaemonError> {
    if binding.relay_peer_protocol_compatible() {
        return Ok(());
    }
    Err(DaemonError::LocalTransport {
        operation: "launch remote native provider run",
        message: format!(
            "remote worker `{}` has an incompatible or legacy relay peer protocol {:?}; rebind the remote agent before launch (current protocol {})",
            binding.worker_kernel_id,
            binding.relay_peer_protocol_version,
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        ),
    })
}

async fn credential_for_binding<ResolveCredential, ResolveCredentialFuture>(
    binding: &RemoteAgentBinding,
    resolve_credential: &mut ResolveCredential,
) -> Result<Option<RemoteProviderLaunchCredential>, DaemonError>
where
    ResolveCredential: FnMut() -> ResolveCredentialFuture,
    ResolveCredentialFuture:
        Future<Output = Result<Option<RemoteProviderLaunchCredential>, DaemonError>>,
{
    if binding.active_worker_provider_run_id.is_some() {
        return Ok(None);
    }
    resolve_credential().await
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;
    use crate::transport::relay_peer::RemoteCredentialSecretInput;

    fn binding(
        protocol: Option<u32>,
        active_worker_provider_run_id: Option<&str>,
    ) -> RemoteAgentBinding {
        RemoteAgentBinding {
            worker_kernel_id: "worker-kernel-1".to_string(),
            worker_machine_id: "worker-machine-1".to_string(),
            execution_lease_id: "lease-1".to_string(),
            leased_agent_id: "leased-agent-1".to_string(),
            active_worker_provider_run_id: active_worker_provider_run_id.map(str::to_string),
            relay_url: None,
            relay_token: None,
            relay_peer_protocol_version: protocol,
        }
    }

    fn credential() -> RemoteProviderLaunchCredential {
        RemoteProviderLaunchCredential {
            provider: "claude".to_string(),
            account_profile: "work".to_string(),
            secret_input: RemoteCredentialSecretInput::new("test-setup-token".to_string()),
        }
    }

    #[tokio::test]
    async fn remote_native_launch_rejects_missing_or_old_protocol_before_credentials_or_relay() {
        let current = crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION;
        for protocol in [None, Some(current.saturating_sub(1))] {
            let credential_calls = Arc::new(AtomicUsize::new(0));
            let send_calls = Arc::new(AtomicUsize::new(0));
            let refresh_calls = Arc::new(AtomicUsize::new(0));
            let result = launch_with_one_binding_refresh(
                binding(protocol, None),
                {
                    let credential_calls = Arc::clone(&credential_calls);
                    move || {
                        credential_calls.fetch_add(1, Ordering::SeqCst);
                        std::future::ready(Ok(Some(credential())))
                    }
                },
                {
                    let send_calls = Arc::clone(&send_calls);
                    move |_, _| {
                        send_calls.fetch_add(1, Ordering::SeqCst);
                        std::future::ready(Ok("launched"))
                    }
                },
                {
                    let refresh_calls = Arc::clone(&refresh_calls);
                    move || {
                        refresh_calls.fetch_add(1, Ordering::SeqCst);
                        std::future::ready(Ok(binding(Some(current), None)))
                    }
                },
            )
            .await;

            let error = result.expect_err("legacy bindings must fail before launch side effects");
            assert!(error.to_string().contains("incompatible or legacy"));
            assert_eq!(credential_calls.load(Ordering::SeqCst), 0);
            assert_eq!(send_calls.load(Ordering::SeqCst), 0);
            assert_eq!(refresh_calls.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn active_remote_claude_native_run_does_not_resolve_a_home_vault_token() {
        let credential_calls = Arc::new(AtomicUsize::new(0));
        let result = launch_with_one_binding_refresh(
            binding(
                Some(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION),
                Some("worker-run-1"),
            ),
            {
                let credential_calls = Arc::clone(&credential_calls);
                move || {
                    credential_calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Err(DaemonError::InvalidConfig {
                        field: "provider account credential",
                        message: "test vault token is absent",
                    }))
                }
            },
            |_, credential| {
                assert!(
                    credential.is_none(),
                    "an already-running worker provider must not receive a home credential"
                );
                std::future::ready(Ok("reused"))
            },
            || {
                std::future::ready(Err(DaemonError::InternalInvariant {
                    operation: "refresh test binding",
                    message: "successful reuse must not refresh".to_string(),
                }))
            },
        )
        .await
        .expect("an active worker run must not depend on a home vault token");

        assert_eq!(result.0, "reused");
        assert_eq!(credential_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn stale_active_run_hint_retries_once_when_worker_requires_cold_credential() {
        let credential_calls = Arc::new(AtomicUsize::new(0));
        let send_calls = Arc::new(AtomicUsize::new(0));
        let refresh_calls = Arc::new(AtomicUsize::new(0));
        let result = launch_with_one_binding_refresh(
            binding(
                Some(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION),
                Some("stale-worker-run"),
            ),
            {
                let credential_calls = Arc::clone(&credential_calls);
                move || {
                    credential_calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(Some(credential())))
                }
            },
            {
                let send_calls = Arc::clone(&send_calls);
                move |_, credential| {
                    let attempt = send_calls.fetch_add(1, Ordering::SeqCst);
                    if attempt == 0 {
                        assert!(credential.is_none());
                        std::future::ready(Err(DaemonError::RelayTransport {
                            operation: "read relay peer response",
                            code: crate::transport::relay_peer::REMOTE_PROVIDER_LAUNCH_CREDENTIAL_REQUIRED_CODE
                                .to_string(),
                            message: "the hinted worker run is no longer live".to_string(),
                            retryable: false,
                        }))
                    } else {
                        assert!(credential.is_some());
                        std::future::ready(Ok("launched-after-credential"))
                    }
                }
            },
            {
                let refresh_calls = Arc::clone(&refresh_calls);
                move || {
                    refresh_calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(binding(None, None)))
                }
            },
        )
        .await
        .expect("a stale active-run hint should request the cold-launch credential once");

        assert_eq!(result.0, "launched-after-credential");
        assert_eq!(credential_calls.load(Ordering::SeqCst), 1);
        assert_eq!(send_calls.load(Ordering::SeqCst), 2);
        assert_eq!(refresh_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn cold_remote_native_launch_preserves_credential_resolution_failure() {
        let send_calls = Arc::new(AtomicUsize::new(0));
        let result = launch_with_one_binding_refresh(
            binding(
                Some(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION),
                None,
            ),
            || {
                std::future::ready(Err(DaemonError::InvalidConfig {
                    field: "provider account credential",
                    message: "cold launch requires the configured setup token",
                }))
            },
            {
                let send_calls = Arc::clone(&send_calls);
                move |_, _| {
                    send_calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok("must-not-launch"))
                }
            },
            || std::future::ready(Ok(binding(None, None))),
        )
        .await;

        let error = result.expect_err("cold launch must preserve credential failure");
        assert!(error
            .to_string()
            .contains("cold launch requires the configured setup token"));
        assert_eq!(send_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn stale_remote_native_lease_refreshes_once_and_revalidates_launch_needs() {
        let current = crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION;
        let credential_calls = Arc::new(AtomicUsize::new(0));
        let send_calls = Arc::new(AtomicUsize::new(0));
        let refresh_calls = Arc::new(AtomicUsize::new(0));
        let result = launch_with_one_binding_refresh(
            binding(Some(current), Some("stale-worker-run")),
            {
                let credential_calls = Arc::clone(&credential_calls);
                move || {
                    credential_calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(Some(credential())))
                }
            },
            {
                let send_calls = Arc::clone(&send_calls);
                move |binding, credential| {
                    let attempt = send_calls.fetch_add(1, Ordering::SeqCst);
                    if attempt == 0 {
                        assert_eq!(binding.leased_agent_id, "leased-agent-1");
                        assert!(credential.is_none());
                        std::future::ready(Err(DaemonError::ExecutionLeaseNotFound {
                            lease_id: "lease-1".to_string(),
                        }))
                    } else {
                        assert_eq!(binding.leased_agent_id, "leased-agent-2");
                        assert!(credential.is_some());
                        std::future::ready(Ok("launched-after-refresh"))
                    }
                }
            },
            {
                let refresh_calls = Arc::clone(&refresh_calls);
                move || {
                    refresh_calls.fetch_add(1, Ordering::SeqCst);
                    let mut refreshed = binding(Some(current), None);
                    refreshed.execution_lease_id = "lease-2".to_string();
                    refreshed.leased_agent_id = "leased-agent-2".to_string();
                    std::future::ready(Ok(refreshed))
                }
            },
        )
        .await
        .expect("one revisioned stale-binding refresh should recover launch");

        assert_eq!(result.0, "launched-after-refresh");
        assert_eq!(result.1.leased_agent_id, "leased-agent-2");
        assert_eq!(credential_calls.load(Ordering::SeqCst), 1);
        assert_eq!(send_calls.load(Ordering::SeqCst), 2);
        assert_eq!(refresh_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn stale_remote_native_lease_rejects_incompatible_refreshed_binding_before_retry() {
        let current = crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION;
        let credential_calls = Arc::new(AtomicUsize::new(0));
        let send_calls = Arc::new(AtomicUsize::new(0));
        let result = launch_with_one_binding_refresh(
            binding(Some(current), Some("stale-worker-run")),
            {
                let credential_calls = Arc::clone(&credential_calls);
                move || {
                    credential_calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(Some(credential())))
                }
            },
            {
                let send_calls = Arc::clone(&send_calls);
                move |_, credential| {
                    send_calls.fetch_add(1, Ordering::SeqCst);
                    assert!(credential.is_none());
                    std::future::ready(Err::<(), _>(DaemonError::LeasedAgentNotFound {
                        leased_agent_id: "leased-agent-1".to_string(),
                    }))
                }
            },
            move || std::future::ready(Ok(binding(Some(current.saturating_sub(1)), None))),
        )
        .await;

        let error = result.expect_err("an incompatible rebound worker must not be retried");
        assert!(error.to_string().contains("incompatible or legacy"));
        assert_eq!(credential_calls.load(Ordering::SeqCst), 0);
        assert_eq!(send_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn repeated_stale_remote_native_lease_error_is_not_refreshed_twice() {
        let current = crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION;
        let send_calls = Arc::new(AtomicUsize::new(0));
        let refresh_calls = Arc::new(AtomicUsize::new(0));
        let result = launch_with_one_binding_refresh(
            binding(Some(current), Some("stale-worker-run")),
            || std::future::ready(Ok(None)),
            {
                let send_calls = Arc::clone(&send_calls);
                move |_, _| {
                    send_calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Err::<(), _>(DaemonError::ExecutionLeaseNotFound {
                        lease_id: "still-stale".to_string(),
                    }))
                }
            },
            {
                let refresh_calls = Arc::clone(&refresh_calls);
                move || {
                    refresh_calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(binding(Some(current), Some("refreshed-run"))))
                }
            },
        )
        .await;

        assert!(matches!(
            result,
            Err(DaemonError::ExecutionLeaseNotFound { .. })
        ));
        assert_eq!(send_calls.load(Ordering::SeqCst), 2);
        assert_eq!(refresh_calls.load(Ordering::SeqCst), 1);
    }
}
