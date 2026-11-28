//! Compatibility, account and stale-binding policy for remote native provider launches.
//!
//! Row 577: a leased launch carries no provider credential. A cold launch first makes sure
//! the worker holds its own tracked copy of the account (`ensure_account`); the worker then
//! launches from that copy. A launch that reuses a live worker run needs no account step.

use std::future::Future;

use crate::agent::RemoteAgentBinding;
use crate::error::DaemonError;

// The account callback receives true only after a definitive missing-account response;
// ordinary cold preparation reuses a recorded copy, while recovery replaces a lost one.
pub(super) async fn launch_with_one_binding_refresh<
    Response,
    EnsureAccount,
    EnsureAccountFuture,
    Send,
    SendFuture,
    Refresh,
    RefreshFuture,
>(
    initial_binding: RemoteAgentBinding,
    mut ensure_account: EnsureAccount,
    mut send: Send,
    mut refresh: Refresh,
) -> Result<(Response, RemoteAgentBinding), DaemonError>
where
    EnsureAccount: FnMut(bool) -> EnsureAccountFuture,
    EnsureAccountFuture: Future<Output = Result<(), DaemonError>>,
    Send: FnMut(RemoteAgentBinding) -> SendFuture,
    SendFuture: Future<Output = Result<Response, DaemonError>>,
    Refresh: FnMut() -> RefreshFuture,
    RefreshFuture: Future<Output = Result<RemoteAgentBinding, DaemonError>>,
{
    ensure_compatible_binding(&initial_binding)?;
    account_for_binding(&initial_binding, &mut ensure_account).await?;
    let mut binding = initial_binding;
    let mut account_reinstall_used = false;
    let mut binding_refresh_used = false;
    loop {
        match send(binding.clone()).await {
            Ok(response) => return Ok((response, binding)),
            Err(error)
                if !account_reinstall_used
                    && super::remote_prompt_worker_submission_runtime::remote_prompt_error_is_account_missing(&error) =>
            {
                // Only this definitive pre-admission refusal permits account repair and replay.
                account_reinstall_used = true;
                ensure_account(true).await?;
            }
            Err(error)
                if !binding_refresh_used
                    && super::remote_prompt_worker_submission_runtime::remote_prompt_error_should_refresh_binding(&error) =>
            {
                binding_refresh_used = true;
                binding = refresh().await?;
                ensure_compatible_binding(&binding)?;
                account_for_binding(&binding, &mut ensure_account).await?;
            }
            Err(error) => return Err(error),
        }
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

async fn account_for_binding<EnsureAccount, EnsureAccountFuture>(
    binding: &RemoteAgentBinding,
    ensure_account: &mut EnsureAccount,
) -> Result<(), DaemonError>
where
    EnsureAccount: FnMut(bool) -> EnsureAccountFuture,
    EnsureAccountFuture: Future<Output = Result<(), DaemonError>>,
{
    if binding.active_worker_provider_run_id.is_some() {
        return Ok(());
    }
    ensure_account(false).await
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;

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
            worker_worktree_id: None,
        }
    }

    #[tokio::test]
    async fn remote_native_launch_rejects_missing_or_old_protocol_before_account_or_relay() {
        let current = crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION;
        for protocol in [
            None,
            Some(crate::transport::relay_peer::MINIMUM_RELAY_PEER_RUNTIME_VERSION - 1),
        ] {
            let account_calls = Arc::new(AtomicUsize::new(0));
            let send_calls = Arc::new(AtomicUsize::new(0));
            let refresh_calls = Arc::new(AtomicUsize::new(0));
            let result = launch_with_one_binding_refresh(
                binding(protocol, None),
                {
                    let account_calls = Arc::clone(&account_calls);
                    move |_recover_missing| {
                        account_calls.fetch_add(1, Ordering::SeqCst);
                        std::future::ready(Ok(()))
                    }
                },
                {
                    let send_calls = Arc::clone(&send_calls);
                    move |_| {
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
            assert_eq!(account_calls.load(Ordering::SeqCst), 0);
            assert_eq!(send_calls.load(Ordering::SeqCst), 0);
            assert_eq!(refresh_calls.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn active_remote_claude_native_run_needs_no_account_step() {
        let account_calls = Arc::new(AtomicUsize::new(0));
        let result = launch_with_one_binding_refresh(
            binding(
                Some(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION),
                Some("worker-run-1"),
            ),
            {
                let account_calls = Arc::clone(&account_calls);
                move |_recover_missing| {
                    account_calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Err(DaemonError::InvalidConfig {
                        field: "provider account",
                        message: "must not be asked for a live worker run",
                    }))
                }
            },
            |_| std::future::ready(Ok("reused")),
            || {
                std::future::ready(Err(DaemonError::InternalInvariant {
                    operation: "refresh test binding",
                    message: "successful reuse must not refresh".to_string(),
                }))
            },
        )
        .await
        .expect("an active worker run must not depend on the home account");

        assert_eq!(result.0, "reused");
        assert_eq!(account_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn cold_remote_native_launch_preserves_the_account_step_refusal() {
        let send_calls = Arc::new(AtomicUsize::new(0));
        let result = launch_with_one_binding_refresh(
            binding(
                Some(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION),
                None,
            ),
            |_recover_missing| {
                std::future::ready(Err(DaemonError::InvalidConfig {
                    field: "provider account",
                    message: "the account could not be copied to the worker",
                }))
            },
            {
                let send_calls = Arc::clone(&send_calls);
                move |_| {
                    send_calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok("must-not-launch"))
                }
            },
            || std::future::ready(Ok(binding(None, None))),
        )
        .await;

        let error = result.expect_err("a cold launch without the account must not be sent");
        assert!(error
            .to_string()
            .contains("the account could not be copied to the worker"));
        assert_eq!(send_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn stale_native_hint_repairs_missing_copy_once_before_retry() {
        let current = crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION;
        for repeated_missing in [false, true] {
            let account_calls = Arc::new(AtomicUsize::new(0));
            let send_calls = Arc::new(AtomicUsize::new(0));
            let result = launch_with_one_binding_refresh(
                binding(Some(current), Some("gone-worker-run")),
                {
                    let calls = Arc::clone(&account_calls);
                    move |recover_missing| {
                        assert!(recover_missing, "stale hints must force repair of an installed copy");
                        calls.fetch_add(1, Ordering::SeqCst);
                        std::future::ready(Ok(()))
                    }
                },
                {
                    let calls = Arc::clone(&send_calls);
                    move |_| {
                        let first = calls.fetch_add(1, Ordering::SeqCst) == 0;
                        std::future::ready(if first || repeated_missing {
                            Err(DaemonError::RelayTransport {
                                operation: "read relay peer response",
                                code: crate::transport::relay_peer::REMOTE_PROVIDER_ACCOUNT_MISSING_CODE.into(),
                                message: "worker no longer holds the selected account".into(),
                                retryable: false,
                            })
                        } else {
                            Ok("launched-from-copy")
                        })
                    }
                },
                || std::future::ready(Err(DaemonError::InternalInvariant {
                    operation: "refresh test binding",
                    message: "a missing account does not require a new lease".into(),
                })),
            ).await;
            assert_eq!(account_calls.load(Ordering::SeqCst), 1);
            assert_eq!(send_calls.load(Ordering::SeqCst), 2);
            if repeated_missing {
                assert!(
                    result.is_err(),
                    "a repeated missing response must stop, not loop"
                );
            } else {
                assert_eq!(result.unwrap().0, "launched-from-copy");
            }
        }
    }

    #[tokio::test]
    async fn stale_remote_native_lease_refreshes_once_and_revalidates_the_account() {
        let current = crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION;
        let account_calls = Arc::new(AtomicUsize::new(0));
        let send_calls = Arc::new(AtomicUsize::new(0));
        let refresh_calls = Arc::new(AtomicUsize::new(0));
        let result = launch_with_one_binding_refresh(
            binding(Some(current), Some("stale-worker-run")),
            {
                let account_calls = Arc::clone(&account_calls);
                move |_recover_missing| {
                    account_calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(()))
                }
            },
            {
                let send_calls = Arc::clone(&send_calls);
                move |binding| {
                    let attempt = send_calls.fetch_add(1, Ordering::SeqCst);
                    if attempt == 0 {
                        assert_eq!(binding.leased_agent_id, "leased-agent-1");
                        std::future::ready(Err(DaemonError::ExecutionLeaseNotFound {
                            lease_id: "lease-1".to_string(),
                        }))
                    } else {
                        assert_eq!(binding.leased_agent_id, "leased-agent-2");
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
        assert_eq!(account_calls.load(Ordering::SeqCst), 1);
        assert_eq!(send_calls.load(Ordering::SeqCst), 2);
        assert_eq!(refresh_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn stale_remote_native_lease_rejects_incompatible_refreshed_binding_before_retry() {
        let current = crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION;
        let account_calls = Arc::new(AtomicUsize::new(0));
        let send_calls = Arc::new(AtomicUsize::new(0));
        let result = launch_with_one_binding_refresh(
            binding(Some(current), Some("stale-worker-run")),
            {
                let account_calls = Arc::clone(&account_calls);
                move |_recover_missing| {
                    account_calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(()))
                }
            },
            {
                let send_calls = Arc::clone(&send_calls);
                move |_| {
                    send_calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Err::<(), _>(DaemonError::LeasedAgentNotFound {
                        leased_agent_id: "leased-agent-1".to_string(),
                    }))
                }
            },
            move || {
                std::future::ready(Ok(binding(
                    Some(crate::transport::relay_peer::MINIMUM_RELAY_PEER_RUNTIME_VERSION - 1),
                    None,
                )))
            },
        )
        .await;

        let error = result.expect_err("an incompatible rebound worker must not be retried");
        assert!(error.to_string().contains("incompatible or legacy"));
        assert_eq!(account_calls.load(Ordering::SeqCst), 0);
        assert_eq!(send_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn repeated_stale_remote_native_lease_error_is_not_refreshed_twice() {
        let current = crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION;
        let send_calls = Arc::new(AtomicUsize::new(0));
        let refresh_calls = Arc::new(AtomicUsize::new(0));
        let result = launch_with_one_binding_refresh(
            binding(Some(current), Some("stale-worker-run")),
            |_recover_missing| std::future::ready(Ok(())),
            {
                let send_calls = Arc::clone(&send_calls);
                move |_| {
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
