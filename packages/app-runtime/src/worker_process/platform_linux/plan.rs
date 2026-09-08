//! Fixed namespace layout assembled only from the platform's retained objects.
//! These private bindings are not an alternative runtime enrollment API.
use super::{Result, WorkerError};
use std::{
    ffi::CString,
    fs::File,
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::PathBuf,
};

pub(super) const ROOTS: [&str; 4] = ["/app/package", "/app/data", "/app/tmp", "/runtime"];

pub(super) struct Binding {
    pub path: PathBuf,
    pub object: File,
}
impl Binding {
    fn path(&self) -> Result<&str> {
        let value = self.path.to_str().ok_or(WorkerError::Preparation)?;
        if !self.path.is_absolute()
            || value.len() > 1024
            || value.bytes().any(|value| value.is_ascii_control())
            || std::fs::canonicalize(&self.path).map_err(|_| WorkerError::Preparation)? != self.path
        {
            return Err(WorkerError::Preparation);
        }
        let named = std::fs::symlink_metadata(&self.path).map_err(|_| WorkerError::Preparation)?;
        let held = self
            .object
            .metadata()
            .map_err(|_| WorkerError::Preparation)?;
        if named.dev() != held.dev() || named.ino() != held.ino() {
            return Err(WorkerError::Preparation);
        }
        Ok(value)
    }
}

/// Host libraries must come from the installer's validated platform graph. Only
/// exact loader/library destinations are admitted; whole /usr or /lib is never
/// bound. Source trust/pinning is still the enrollment lease's responsibility.
pub(super) fn arguments(
    roots: &[Binding; 4],
    libraries: &[(String, Binding)],
) -> Result<Vec<CString>> {
    if libraries.is_empty() || libraries.len() > 8 {
        return Err(WorkerError::Preparation);
    }
    let mut args: Vec<String> = [
        "chariox-bwrap",
        "--unshare-user",
        "--unshare-pid",
        "--unshare-net",
        "--unshare-ipc",
        "--unshare-uts",
        "--unshare-cgroup",
        "--disable-userns",
        "--cap-drop",
        "ALL",
        "--new-session",
        "--die-with-parent",
        "--as-pid-1",
        "--clearenv",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for (index, binding) in roots.iter().enumerate() {
        let path = binding.path()?;
        let metadata = binding
            .object
            .metadata()
            .map_err(|_| WorkerError::Preparation)?;
        if !metadata.is_dir() {
            return Err(WorkerError::Preparation);
        }
        let mut mount = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
        if unsafe { libc::fstatvfs(binding.object.as_raw_fd(), mount.as_mut_ptr()) } != 0 {
            return Err(WorkerError::Preparation);
        }
        let mount = unsafe { mount.assume_init() };
        let mut required = libc::ST_NODEV | libc::ST_NOSUID;
        if index != 3 {
            required |= libc::ST_NOEXEC;
        }
        if index == 0 || index == 3 {
            required |= libc::ST_RDONLY;
        }
        if mount.f_flag & required != required {
            return Err(WorkerError::Preparation);
        }
        args.extend([
            if index == 0 || index == 3 {
                "--ro-bind"
            } else {
                "--bind"
            }
            .into(),
            path.into(),
            ROOTS[index].into(),
        ]);
    }
    let mut destinations = std::collections::BTreeSet::new();
    for (destination, binding) in libraries {
        if !library_destination(destination)
            || !destinations.insert(destination)
            || !binding
                .object
                .metadata()
                .map_err(|_| WorkerError::Preparation)?
                .is_file()
        {
            return Err(WorkerError::Preparation);
        }
        args.extend([
            "--ro-bind".into(),
            binding.path()?.into(),
            destination.clone(),
        ]);
    }
    args.extend(
        [
            "--dev-bind",
            "/dev/null",
            "/dev/null",
            "--chdir",
            "/app/data",
            "--remount-ro",
            "/",
            "--",
            "/runtime/chariox-app-worker",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    if args.len() > 126 || args.iter().map(|v| v.len() + 1).sum::<usize>() > 32768 {
        return Err(WorkerError::Preparation);
    }
    args.into_iter()
        .map(|value| CString::new(value).map_err(|_| WorkerError::Preparation))
        .collect()
}

fn library_destination(value: &str) -> bool {
    let (directory, name) = value.rsplit_once('/').unwrap_or(("", ""));
    match directory {
        "/lib64" => name == "ld-linux-x86-64.so.2",
        "/lib" => name == "ld-linux-aarch64.so.1",
        "/lib/x86_64-linux-gnu" | "/lib/aarch64-linux-gnu" => matches!(
            name,
            "libc.so.6"
                | "libm.so.6"
                | "libstdc++.so.6"
                | "libgcc_s.so.1"
                | "libpthread.so.0"
                | "libdl.so.2"
                | "librt.so.1"
        ),
        _ => false,
    }
}
