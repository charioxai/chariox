//! Root-only step of the managed bootstrap unit. Once the control plane confirmed the kernel's
//! enrollment the bootstrap token authorises nothing, so root blanks it in the protected envelope the
//! `chariox` supervisor can read but not replace.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use crate::error::DaemonError;

use super::state::{
    default_managed_bootstrap_receipt_path, read_bounded_json, validate_protected_bootstrap_file,
    BootstrapReceipt, BootstrapReceiptStatus, ManagedBootstrapEnvelope,
    ManagedBootstrapGrantBinding, PROTECTED_MANAGED_BOOTSTRAP_PATH,
};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// A restart of the unit starts a new wait, so a machine that confirms later is still covered.
const MAX_WAIT: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Redemption {
    Pending,
    Redeemed,
}

pub fn run_from_env() -> Result<(), DaemonError> {
    redeem_when_confirmed(
        Path::new(PROTECTED_MANAGED_BOOTSTRAP_PATH),
        &default_managed_bootstrap_receipt_path(),
        MAX_WAIT,
        POLL_INTERVAL,
    )
}

fn redeem_when_confirmed(
    envelope_path: &Path,
    receipt_path: &Path,
    max_wait: Duration,
    poll_interval: Duration,
) -> Result<(), DaemonError> {
    let deadline = Instant::now() + max_wait;
    loop {
        match redeem_if_confirmed(envelope_path, receipt_path)? {
            Redemption::Redeemed => return Ok(()),
            Redemption::Pending if Instant::now() >= deadline => {
                return Err(redemption_error(
                    "the control plane has not confirmed the enrollment; the bootstrap token stays until the next start",
                ))
            }
            Redemption::Pending => thread::sleep(poll_interval),
        }
    }
}

pub(super) fn redeem_if_confirmed(
    envelope_path: &Path,
    receipt_path: &Path,
) -> Result<Redemption, DaemonError> {
    let receipt = match BootstrapReceipt::read(receipt_path)? {
        Some(receipt) if receipt.status == BootstrapReceiptStatus::Confirmed => receipt,
        _ => return Ok(Redemption::Pending),
    };
    if envelope_path == Path::new(PROTECTED_MANAGED_BOOTSTRAP_PATH) {
        validate_protected_bootstrap_file(envelope_path, PROTECTED_MANAGED_BOOTSTRAP_PATH)?;
    }
    let envelope =
        read_bounded_json::<ManagedBootstrapEnvelope>(envelope_path, "managed bootstrap envelope")?;
    envelope.validate(true)?;
    if envelope.is_redeemed() {
        return Ok(Redemption::Redeemed);
    }
    // Only a grant bound to this exact confirmed receipt is spent; a newer grant waits for its own.
    if envelope.environment_id != receipt.environment_id
        || ManagedBootstrapGrantBinding::read_for_receipt(receipt_path)?.as_ref()
            != Some(&ManagedBootstrapGrantBinding::for_receipt(
                &envelope, &receipt,
            ))
    {
        return Ok(Redemption::Pending);
    }
    replace_with_redeemed(envelope_path, &envelope.redeemed())?;
    Ok(Redemption::Redeemed)
}

/// Replaces the file atomically with the same mode, so the supervisor never reads a torn envelope.
fn replace_with_redeemed(
    path: &Path,
    redeemed: &ManagedBootstrapEnvelope,
) -> Result<(), DaemonError> {
    let mode = fs::metadata(path).map_err(io_error)?.permissions();
    let bytes = serde_json::to_vec_pretty(redeemed)
        .map_err(|error| redemption_error(&error.to_string()))?;
    let staged = path.with_extension("json.redeem");
    let _ = fs::remove_file(&staged);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&staged)
        .map_err(io_error)?;
    file.write_all(&bytes).map_err(io_error)?;
    file.set_permissions(mode).map_err(io_error)?;
    file.sync_all().map_err(io_error)?;
    fs::rename(&staged, path).map_err(io_error)?;
    if let Some(directory) = path.parent() {
        fs::File::open(directory)
            .and_then(|directory| directory.sync_all())
            .map_err(io_error)?;
    }
    Ok(())
}

fn io_error(error: std::io::Error) -> DaemonError {
    redemption_error(&error.to_string())
}

fn redemption_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "redeem managed bootstrap grant",
        message: message.to_string(),
    }
}
