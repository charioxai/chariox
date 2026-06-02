use arroba_relay::protocol::RelayKernelPresence;

use crate::error::DaemonError;

pub(super) fn select_remote_kernel(
    kernels: Vec<RelayKernelPresence>,
    machine_ref: &str,
    provider: &str,
) -> Option<RelayKernelPresence> {
    kernels
        .into_iter()
        .filter(|kernel| kernel_presence_matches_machine_ref(kernel, machine_ref))
        .filter(|kernel| kernel.accepting_remote_leases)
        .filter(|kernel| {
            kernel
                .available_providers
                .iter()
                .any(|candidate| candidate == provider)
        })
        .min_by_key(|kernel| {
            (
                kernel.leased_agent_count,
                kernel.local_session_count,
                kernel.kernel_id.clone(),
            )
        })
}

pub(super) fn no_remote_kernel_available_message(
    kernels: &[RelayKernelPresence],
    machine_ref: &str,
    provider: &str,
) -> String {
    let candidates = kernels
        .iter()
        .filter(|kernel| kernel_presence_matches_machine_ref(kernel, machine_ref))
        .collect::<Vec<_>>();
    if kernels.is_empty() || candidates.is_empty() {
        return format!(
            "no live remote kernel found for machine `{machine_ref}`; next: run `/machine kernels {machine_ref}`, reconnect that machine, or choose another worker",
        );
    }
    if candidates
        .iter()
        .all(|kernel| !kernel.accepting_remote_leases)
    {
        return format!(
            "no live remote kernel on machine `{machine_ref}` is accepting remote agents ({kernels}); next: enable remote leases on {kernels} or choose another worker",
            kernels = format_kernel_targets(candidates.iter().copied()),
        );
    }
    let accepting = candidates
        .iter()
        .copied()
        .filter(|kernel| kernel.accepting_remote_leases)
        .collect::<Vec<_>>();
    if accepting
        .iter()
        .all(|kernel| kernel.available_providers.is_empty())
    {
        return format!(
            "no accepting remote kernel on machine `{machine_ref}` advertises provider CLIs ({kernels}); next: configure provider CLIs on {kernels} or choose another worker",
            kernels = format_kernel_targets(accepting.iter().copied()),
        );
    }
    let provider_summary = accepting
        .iter()
        .map(|kernel| {
            let providers = if kernel.available_providers.is_empty() {
                "no providers".to_string()
            } else {
                kernel.available_providers.join(",")
            };
            format!("{}={providers}", kernel_label(kernel))
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "no accepting remote kernel on machine `{machine_ref}` can host provider `{provider}` ({provider_summary}); next: choose a worker with `{provider}` or change the agent provider",
    )
}

pub(super) fn kernel_presence_matches_ref(kernel: &RelayKernelPresence, kernel_ref: &str) -> bool {
    kernel.kernel_id == kernel_ref
        || kernel.kernel_alias.as_deref() == Some(kernel_ref)
        || kernel.relay_alias.as_deref() == Some(kernel_ref)
}

pub(super) fn ensure_kernel_can_host_provider(
    kernel: RelayKernelPresence,
    kernel_ref: &str,
    provider: &str,
) -> Result<RelayKernelPresence, DaemonError> {
    if !kernel.accepting_remote_leases {
        return Err(DaemonError::LocalTransport {
            operation: "select remote kernel",
            message: format!(
                "kernel `{kernel_ref}` is not accepting worker agents; next: enable remote leases on `{kernel_ref}` or choose another worker",
            ),
        });
    }
    if !kernel
        .available_providers
        .iter()
        .any(|candidate| candidate == provider)
    {
        return Err(DaemonError::LocalTransport {
            operation: "select remote kernel",
            message: format!(
                "kernel `{kernel_ref}` cannot host provider `{provider}`; available providers: {providers}; next: choose a worker with `{provider}` or change the agent provider",
                providers = if kernel.available_providers.is_empty() {
                    "none".to_string()
                } else {
                    kernel.available_providers.join(",")
                },
            ),
        });
    }
    Ok(kernel)
}

fn kernel_presence_matches_machine_ref(kernel: &RelayKernelPresence, machine_ref: &str) -> bool {
    kernel.machine_id == machine_ref
        || kernel.machine_alias.as_deref() == Some(machine_ref)
        || kernel.relay_alias.as_deref() == Some(machine_ref)
        || kernel.kernel_alias.as_deref() == Some(machine_ref)
}

fn format_kernel_targets<'a>(kernels: impl Iterator<Item = &'a RelayKernelPresence>) -> String {
    let labels = kernels.map(kernel_label).collect::<Vec<_>>();
    match labels.as_slice() {
        [] => "the listed worker kernel".to_string(),
        [label] => format!("kernel `{label}`"),
        _ => format!("kernels `{}`", labels.join("`, `")),
    }
}

fn kernel_label(kernel: &RelayKernelPresence) -> String {
    kernel
        .relay_alias
        .as_deref()
        .or(kernel.kernel_alias.as_deref())
        .unwrap_or(&kernel.kernel_id)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_kernel_unavailable_message_names_disabled_leases() {
        let message = no_remote_kernel_available_message(
            &[kernel("kernel-1", false, &["codex"])],
            "machine-1",
            "codex",
        );

        assert_eq!(
            message,
            "no live remote kernel on machine `machine-1` is accepting remote agents (kernel `kernel-1`); next: enable remote leases on kernel `kernel-1` or choose another worker",
        );
    }

    #[test]
    fn remote_kernel_unavailable_message_names_missing_provider_clis() {
        let message = no_remote_kernel_available_message(
            &[kernel("kernel-1", true, &[])],
            "machine-1",
            "codex",
        );

        assert_eq!(
            message,
            "no accepting remote kernel on machine `machine-1` advertises provider CLIs (kernel `kernel-1`); next: configure provider CLIs on kernel `kernel-1` or choose another worker",
        );
    }

    #[test]
    fn remote_kernel_unavailable_message_names_wrong_provider() {
        let message = no_remote_kernel_available_message(
            &[kernel("kernel-1", true, &["opencode"])],
            "machine-1",
            "codex",
        );

        assert_eq!(
            message,
            "no accepting remote kernel on machine `machine-1` can host provider `codex` (kernel-1=opencode); next: choose a worker with `codex` or change the agent provider",
        );
    }

    #[test]
    fn remote_kernel_unavailable_message_names_missing_machine() {
        let message = no_remote_kernel_available_message(&[], "machine-1", "codex");

        assert_eq!(
            message,
            "no live remote kernel found for machine `machine-1`; next: run `/machine kernels machine-1`, reconnect that machine, or choose another worker",
        );
    }

    #[test]
    fn explicit_kernel_validation_errors_are_actionable() {
        let disabled = ensure_kernel_can_host_provider(
            kernel("kernel-1", false, &["codex"]),
            "worker",
            "codex",
        )
        .expect_err("disabled worker should be rejected")
        .to_string();
        assert_eq!(
            disabled,
            "local transport `select remote kernel` failed: kernel `worker` is not accepting worker agents; next: enable remote leases on `worker` or choose another worker",
        );

        let wrong_provider = ensure_kernel_can_host_provider(
            kernel("kernel-1", true, &["opencode"]),
            "worker",
            "codex",
        )
        .expect_err("wrong provider should be rejected")
        .to_string();
        assert_eq!(
            wrong_provider,
            "local transport `select remote kernel` failed: kernel `worker` cannot host provider `codex`; available providers: opencode; next: choose a worker with `codex` or change the agent provider",
        );
    }

    fn kernel(
        kernel_id: &str,
        accepting_remote_leases: bool,
        providers: &[&str],
    ) -> RelayKernelPresence {
        RelayKernelPresence {
            kernel_id: kernel_id.to_string(),
            machine_id: "machine-1".to_string(),
            machine_alias: None,
            relay_alias: None,
            kernel_alias: None,
            public_key: "public-key".to_string(),
            capabilities: Vec::new(),
            available_providers: providers
                .iter()
                .map(|provider| provider.to_string())
                .collect(),
            accepting_remote_leases,
            leased_agent_count: 0,
            local_session_count: 0,
        }
    }
}
