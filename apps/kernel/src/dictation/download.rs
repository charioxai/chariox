//! Resumable, checksum-verified download of the speech model.
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};

use super::catalog::ModelSpec;

const CHUNK_BYTES: usize = 64 * 1024;

/// Downloads into `<file>.part`, continuing from its length, and publishes the
/// model under its final name only after the size and SHA-256 match the pin.
pub(crate) fn download_model(
    spec: &ModelSpec,
    dir: &Path,
    received: &AtomicU64,
) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|error| format!("cannot create the model folder: {error}"))?;
    let partial = spec.partial_path(dir);
    let mut have = fs::metadata(&partial).map(|meta| meta.len()).unwrap_or(0);
    if have > spec.size_bytes {
        let _ = fs::remove_file(&partial);
        have = 0;
    }
    received.store(have, Ordering::Relaxed);
    if have < spec.size_bytes {
        fetch_remaining(spec, &partial, have, received)?;
    }
    let length = fs::metadata(&partial).map(|meta| meta.len()).unwrap_or(0);
    if length != spec.size_bytes || file_sha256(&partial)? != spec.sha256 {
        let _ = fs::remove_file(&partial);
        return Err(
            "The downloaded model did not match its checksum and was removed. Try again."
                .to_string(),
        );
    }
    fs::rename(&partial, spec.path(dir))
        .map_err(|error| format!("cannot install the model: {error}"))
}

fn fetch_remaining(
    spec: &ModelSpec,
    partial: &Path,
    have: u64,
    received: &AtomicU64,
) -> Result<(), String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(30))
        .build();
    let mut request = agent.get(&spec.url);
    if have > 0 {
        request = request.set("Range", &format!("bytes={have}-"));
    }
    let response = request.call().map_err(|error| {
        format!(
            "The model download failed ({}). Check the connection and try again.",
            describe(&error)
        )
    })?;
    // A server that ignores Range answers 200 with the whole file.
    let resumed = response.status() == 206;
    let mut file = if resumed {
        OpenOptions::new().append(true).open(partial)
    } else {
        received.store(0, Ordering::Relaxed);
        File::create(partial)
    }
    .map_err(|error| format!("cannot write the model: {error}"))?;
    let mut body = response.into_reader();
    let mut buffer = vec![0_u8; CHUNK_BYTES];
    loop {
        let count = body.read(&mut buffer).map_err(|error| {
            format!("The model download was interrupted ({error}). Try again to resume it.")
        })?;
        if count == 0 {
            return Ok(());
        }
        file.write_all(&buffer[..count])
            .map_err(|error| format!("cannot write the model: {error}"))?;
        received.fetch_add(count as u64, Ordering::Relaxed);
    }
}

fn describe(error: &ureq::Error) -> String {
    match error {
        ureq::Error::Status(code, _) => format!("HTTP {code}"),
        ureq::Error::Transport(transport) => transport.kind().to_string(),
    }
}

pub(crate) fn file_sha256(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("cannot read the model: {error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; CHUNK_BYTES];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("cannot read the model: {error}"))?;
        if count == 0 {
            return Ok(hex(&hasher.finalize()));
        }
        hasher.update(&buffer[..count]);
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
