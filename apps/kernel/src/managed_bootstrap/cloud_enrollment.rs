//! MP-08/MP-09/MP-11: bootstrap credentials also authenticate managed lifecycle reports.
use crate::config::DaemonConfig;
use crate::error::DaemonError;

/// Exchanged and confirmed children use the same Cloud bootstrap authority.
/// They are new enrollments, never predecessor kernels needing legacy migration.
/// Cloud still authenticates the credential and binds relay ownership on issuance.
pub(crate) fn uses_bootstrap_cloud_enrollment(config: &DaemonConfig) -> Result<bool, DaemonError> {
    let Some(profile) = config.cloud_relay.as_ref() else {
        return Ok(false);
    };
    if let Some(path) = std::env::var_os(super::worker::ACTIVITY_RECEIPT_ENV) {
        super::worker::activity_allocation(std::path::Path::new(&path), config, profile)?;
        return Ok(true);
    }
    let Some(receipt) =
        super::managed_kernel_receipt_from_env(super::default_managed_bootstrap_receipt_path())?
    else {
        return Ok(false);
    };
    if receipt.kernel_id != config.daemon_id
        || receipt.machine_id != config.host_machine_id
        || receipt.relay_public_key != config.relay_public_key
    {
        return Err(super::bootstrap_error(
            "managed Cloud enrollment does not match the running kernel identity",
        ));
    }
    super::validate_profile(profile, &receipt)?;
    Ok(true)
}
