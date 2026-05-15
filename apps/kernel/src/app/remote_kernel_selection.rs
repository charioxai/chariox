use arroba_relay::protocol::RelayKernelPresence;

use crate::error::DaemonError;

pub(super) fn select_remote_kernel(
    kernels: Vec<RelayKernelPresence>,
    machine_ref: &str,
    provider: &str,
) -> Option<RelayKernelPresence> {
    kernels
        .into_iter()
        .filter(|kernel| {
            kernel.machine_id == machine_ref
                || kernel.machine_alias.as_deref() == Some(machine_ref)
                || kernel.relay_alias.as_deref() == Some(machine_ref)
                || kernel.kernel_alias.as_deref() == Some(machine_ref)
        })
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
            message: format!("kernel `{kernel_ref}` is not accepting worker agents"),
        });
    }
    if !kernel
        .available_providers
        .iter()
        .any(|candidate| candidate == provider)
    {
        return Err(DaemonError::LocalTransport {
            operation: "select remote kernel",
            message: format!("kernel `{kernel_ref}` cannot host provider `{provider}`"),
        });
    }
    Ok(kernel)
}
