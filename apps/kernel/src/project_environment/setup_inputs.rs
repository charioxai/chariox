//! MP-08 / MP-10 / MP-11: Setup reads stay in owned, nondumpable workers.
//! Value bytes use the private output pipe, never request/result files or logs.
use super::resolver::{environment_error, open_workspace_file, read_environment_file};
use crate::error::DaemonError;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::Path,
};
use zeroize::Zeroizing;
const FRAME: &[u8] = b"\0chariox-setup-public\0";

pub(super) fn write_public_file(root: &Path, relative: &str) -> Result<(), DaemonError> {
    #[cfg(test)]
    stall("CHARIOX_SETUP_PUBLIC_STALL_MARKER");
    let text = read_environment_file(root, relative)?;
    let mut pipe = std::io::stdout().lock();
    pipe.write_all(FRAME)
        .and_then(|_| pipe.write_all(&(text.len() as u64).to_be_bytes()))
        .and_then(|_| pipe.write_all(text.as_bytes()))
        .and_then(|_| pipe.flush())
        .map_err(|_| environment_error("Setup public read unavailable"))
}
pub(super) fn decode_public_file(bytes: &[u8]) -> Result<Zeroizing<String>, DaemonError> {
    // The test harness writes its own banner before entering the self-exec worker.
    #[cfg(test)]
    let payload = bytes
        .windows(FRAME.len())
        .position(|part| part == FRAME)
        .map(|at| &bytes[at + FRAME.len()..]);
    #[cfg(not(test))]
    let payload = bytes.strip_prefix(FRAME);
    let payload = payload.ok_or_else(|| environment_error("Setup public read invalid"))?;
    let (size, text) = payload
        .split_at_checked(8)
        .ok_or_else(|| environment_error("Setup public read invalid"))?;
    let size = u64::from_be_bytes(size.try_into().expect("fixed width"));
    if size != text.len() as u64 {
        return Err(environment_error("Setup public read invalid"));
    }
    let text =
        std::str::from_utf8(text).map_err(|_| environment_error("Setup public read invalid"))?;
    Ok(Zeroizing::new(text.to_owned()))
}
pub(super) fn verify_source(
    root: &Path,
    relative: &str,
    expected: Option<&str>,
) -> Result<(), DaemonError> {
    #[cfg(test)]
    stall("CHARIOX_SETUP_SOURCE_STALL_MARKER");
    let mut file = open_workspace_file(root, relative)?;
    let mut digest = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = Zeroizing::new([0u8; 8192]);
    loop {
        let n = file
            .read(&mut *buffer)
            .map_err(|_| environment_error("Setup source unavailable"))?;
        if n == 0 {
            break;
        }
        bytes += n as u64;
        if bytes > 16 * 1024 * 1024 {
            return Err(environment_error("Setup source exceeds bounds"));
        }
        digest.update(&buffer[..n]);
    }
    let actual = format!("{:x}", digest.finalize());
    if expected.is_none_or(|expected| expected.trim_start_matches("sha256:") != actual) {
        return Err(environment_error(
            "Setup source changed; review before executing",
        ));
    }
    Ok(())
}
#[cfg(test)]
fn stall(key: &str) {
    if let Some(marker) = std::env::var_os(key) {
        #[cfg(target_os = "linux")]
        assert_eq!(unsafe { libc::prctl(libc::PR_GET_DUMPABLE, 0, 0, 0, 0) }, 0);
        std::fs::write(marker, std::process::id().to_string()).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

pub(crate) fn setup_public_file(
    config: &crate::config::DaemonConfig,
    root: &Path,
    relative: &str,
    deadline: std::time::Instant,
    cancelled: impl FnMut() -> bool,
) -> Result<Zeroizing<String>, DaemonError> {
    use super::check_io_worker::{run_with_cancel, Request, Response};
    match run_with_cancel(
        config,
        Request::SetupRead {
            root: root.into(),
            relative: relative.into(),
        },
        deadline,
        cancelled,
    )? {
        Response::PublicRead(value) => Ok(value),
        _ => Err(environment_error("Setup public read invalid")),
    }
}
pub(crate) fn setup_source_digest(
    config: &crate::config::DaemonConfig,
    root: &Path,
    relative: &str,
    expected: Option<&str>,
    deadline: std::time::Instant,
    cancelled: impl FnMut() -> bool,
) -> Result<(), DaemonError> {
    use super::check_io_worker::{run_with_cancel, Request, Response};
    match run_with_cancel(
        config,
        Request::SetupSource {
            root: root.into(),
            relative: relative.into(),
            expected: expected.map(str::to_owned),
        },
        deadline,
        cancelled,
    )? {
        Response::SetupSource => Ok(()),
        _ => Err(environment_error("Setup source read invalid")),
    }
}
pub(crate) fn setup_paths(
    config: &crate::config::DaemonConfig,
    workspace: &str,
    project: &str,
    deadline: std::time::Instant,
    cancelled: impl FnMut() -> bool,
) -> Result<(std::path::PathBuf, std::path::PathBuf), DaemonError> {
    use super::check_io_worker::{run_with_cancel, Request, Response};
    let home = config
        .user_config_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| environment_error("Setup kernel home unavailable"))?;
    match run_with_cancel(
        config,
        Request::SetupPaths {
            workspace: workspace.into(),
            home: home.into(),
            project: project.into(),
            worker: config.host_machine_id.clone(),
        },
        deadline,
        cancelled,
    )? {
        Response::SetupPaths {
            workspace,
            preparation,
        } => Ok((workspace, preparation)),
        _ => Err(environment_error("Setup workspace preparation invalid")),
    }
}
