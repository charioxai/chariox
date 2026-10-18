//! MP-08/MP-09/MP-11: bootstrap credentials also authenticate managed lifecycle reports.
use crate::config::DaemonConfig;
use crate::error::DaemonError;

/// Only receipt-bound managed lifecycle capabilities survive kernel enrollment.
/// Cloud independently authenticates and enrolls the kernel through its normal
/// key-pinned ownership flow; this credential signs managed lifecycle reports.
pub(crate) fn managed_lifecycle_credential_is_bound(
    config: &DaemonConfig,
) -> Result<bool, DaemonError> {
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
