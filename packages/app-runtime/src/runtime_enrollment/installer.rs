//! Root installer for Linux runtime generations. External installer authority
//! selects the exact Ed25519 key and inventory digest; artifacts cannot enroll
//! their own key. This does not execute or attest the native sandbox.
mod files;
mod recovery;
#[cfg(test)]
mod tests;
use super::{
    filesystem,
    graph::{Modes, VerifiedGraph},
    manifest, Enrollment, EnrollmentError, Result, INVENTORY, LEASE, MAX_BUNDLE, SIGNATURE,
};
use crate::private_fs::Dir;
use ed25519_dalek::VerifyingKey;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs::File,
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::{Path, PathBuf},
};

const ENROLLMENT: &str = "runtime-enrollment.json";
const PENDING: &str = ".runtime-enrollment.pending";
const STAGING: &str = ".staging";
const RETIRING: &str = ".retiring-";
const MAX_GENERATIONS: usize = 8;
const DISK_RESERVE: u64 = 128 * 1024 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallReceipt {
    pub revision: u64,
    pub inventory_sha256: String,
    pub target: String,
}
/// Only fixed root-owned Linux paths are exposed outside this module.
pub struct RuntimeInstaller;
impl RuntimeInstaller {
    #[cfg(target_os = "linux")]
    pub fn install(
        source: &Path,
        trusted_key: [u8; 32],
        expected_digest: &str,
    ) -> Result<InstallReceipt> {
        require_root()?;
        let roots = Roots::production()?;
        roots.install(source, trusted_key, expected_digest, &mut |_| Ok(()))
    }
    #[cfg(target_os = "linux")]
    pub fn cleanup(inactive_digest: &str) -> Result<()> {
        require_root()?;
        Roots::production()?.cleanup(inactive_digest, &mut |_| Ok(()))
    }
}
#[cfg(target_os = "linux")]
fn require_root() -> Result<()> {
    if unsafe { libc::getuid() } != 0 || unsafe { libc::geteuid() } != 0 {
        return Err(EnrollmentError::Identity);
    }
    Ok(())
}
struct Roots {
    runtimes: PathBuf,
    enrollment: PathBuf,
    uid: u32,
    target: String,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Checkpoint {
    StageCopied,
    GenerationPublished,
    EnrollmentRenamed,
    RetiringPayload,
}
type Hook<'a> = &'a mut dyn FnMut(Checkpoint) -> Result<()>;
struct Context {
    versions: Dir,
    enrollment: Dir,
    _lock: File,
}
impl Roots {
    #[cfg(target_os = "linux")]
    fn production() -> Result<Self> {
        let etc = files::dir(Path::new("/etc"), 0)?;
        let etc_chariox = files::create_or_open(&etc, "chariox", 0)?;
        files::create_or_open(&etc_chariox, "apps", 0)?;
        let usr = files::dir(Path::new("/usr/lib"), 0)?;
        let usr_chariox = files::create_or_open(&usr, "chariox", 0)?;
        files::create_or_open(&usr_chariox, "app-runtimes", 0)?;
        Ok(Self {
            runtimes: "/usr/lib/chariox/app-runtimes".into(),
            enrollment: "/etc/chariox/apps".into(),
            uid: 0,
            target: manifest::target()?.into(),
        })
    }
    fn context(&self) -> Result<Context> {
        let enrollment = files::dir(&self.enrollment, self.uid)?;
        let lock = files::lock(&enrollment, self.uid)?;
        let versions = files::dir(&self.runtimes, self.uid)?;
        // Complete uncertain rename durability before reading, acknowledging or
        // cleaning a previous attempt's visible state.
        enrollment.sync().map_err(files::fs_error)?;
        versions.sync().map_err(files::fs_error)?;
        Ok(Context {
            versions,
            enrollment,
            _lock: lock,
        })
    }
    fn current(&self, context: &Context) -> Result<Option<Enrollment>> {
        let directory =
            filesystem::Directory::from_file(context.enrollment.0.try_clone()?, self.uid);
        let mut file = match directory.file(ENROLLMENT, None) {
            Ok(file) => file,
            Err(EnrollmentError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(None)
            }
            Err(error) => return Err(error),
        };
        let value: Enrollment = serde_json::from_slice(&filesystem::small(&mut file, 4096)?)
            .map_err(|_| EnrollmentError::Contract)?;
        if value.schema != "chariox.app-runtime-enrollment.v1"
            || value.target != self.target
            || value.revision == 0
            || value.revision > i64::MAX as u64
            || !manifest::hex(&value.inventory_sha256, 64)
            || value.runtime_root != self.runtimes.join(&value.inventory_sha256)
        {
            return Err(EnrollmentError::Contract);
        }
        VerifyingKey::from_bytes(&super::decode_hex::<32>(&value.public_key_hex)?)
            .map_err(|_| EnrollmentError::Signature)?;
        Ok(Some(value))
    }
    fn install(
        &self,
        source: &Path,
        key_bytes: [u8; 32],
        digest: &str,
        hook: Hook<'_>,
    ) -> Result<InstallReceipt> {
        if !manifest::hex(digest, 64) {
            return Err(EnrollmentError::Contract);
        }
        let key = VerifyingKey::from_bytes(&key_bytes).map_err(|_| EnrollmentError::Signature)?;
        if key.is_weak() {
            return Err(EnrollmentError::Signature);
        }
        let context = self.context()?;
        let current = self.current(&context)?;
        recovery::recover(&context, self.uid, &self.target, current.as_ref(), hook)?;
        let public_key_hex = key_bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let unchanged = current.as_ref().is_some_and(|current| {
            current.inventory_sha256 == digest && current.public_key_hex == public_key_hex
        });
        let revision = if unchanged {
            current.as_ref().unwrap().revision
        } else {
            current
                .as_ref()
                .map_or(0, |current| current.revision)
                .checked_add(1)
                .filter(|next| *next <= i64::MAX as u64)
                .ok_or(EnrollmentError::Limit)?
        };
        match files::child(&context.versions, digest, self.uid) {
            Ok(existing) => {
                let verified = VerifiedGraph::open(
                    filesystem::Directory::from_file(existing.0.try_clone()?, self.uid),
                    &key,
                    digest,
                    &self.target,
                    Modes::Installed,
                )?;
                recovery::seal_children(&existing, &verified.inventory, self.uid)?;
                files::mode(&existing.0, 0o555)?;
                existing.sync().map_err(files::fs_error)?;
                context.versions.sync().map_err(files::fs_error)?;
            }
            Err(EnrollmentError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut verified = VerifiedGraph::open(
                    filesystem::Directory::source(source)?,
                    &key,
                    digest,
                    &self.target,
                    Modes::Source,
                )?;
                let entries = context
                    .versions
                    .entries(MAX_GENERATIONS + 2)
                    .map_err(files::fs_error)?;
                if entries.len() >= MAX_GENERATIONS {
                    return Err(EnrollmentError::Limit);
                }
                require_disk(
                    &context.versions,
                    verified
                        .inventory
                        .files
                        .iter()
                        .map(|entry| entry.size)
                        .sum::<u64>()
                        + verified.inventory_bytes.len() as u64
                        + 128,
                )?;
                let stage = context
                    .versions
                    .create_child(OsStr::new(STAGING))
                    .map_err(files::fs_error)?;
                context.versions.sync().map_err(files::fs_error)?;
                files::write(&stage, LEASE, b"", self.uid)?;
                for entry in &verified.inventory.files {
                    let input = verified
                        .files
                        .get_mut(&entry.path)
                        .ok_or(EnrollmentError::Identity)?;
                    let mut output = files::output(&stage, &entry.path, self.uid)?;
                    files::copy(
                        input,
                        &mut output,
                        entry.size,
                        &entry.sha256,
                        entry.executable,
                    )?;
                }
                files::write(&stage, INVENTORY, &verified.inventory_bytes, self.uid)?;
                files::write(&stage, SIGNATURE, &verified.signature_bytes, self.uid)?;
                recovery::seal_children(&stage, &verified.inventory, self.uid)?;
                stage.sync().map_err(files::fs_error)?;
                hook(Checkpoint::StageCopied)?;
                // Reverify the copied, held output before its name or enrollment
                // is published; input verification alone cannot certify a copy.
                let copied = VerifiedGraph::open(
                    filesystem::Directory::from_file(stage.0.try_clone()?, self.uid),
                    &key,
                    digest,
                    &self.target,
                    Modes::Installed,
                )?;
                files::rename(&context.versions, STAGING, digest, false)?;
                hook(Checkpoint::GenerationPublished)?;
                files::mode(&stage.0, 0o555)?;
                stage.sync().map_err(files::fs_error)?;
                context.versions.sync().map_err(files::fs_error)?;
                drop(copied);
            }
            Err(error) => return Err(error),
        }
        if !unchanged {
            let enrollment = Enrollment {
                schema: "chariox.app-runtime-enrollment.v1".into(),
                revision,
                target: self.target.clone(),
                runtime_root: self.runtimes.join(digest),
                inventory_sha256: digest.into(),
                public_key_hex,
            };
            files::remove_file(&context.enrollment, PENDING, self.uid)?;
            let bytes = serde_json::to_vec(&enrollment).map_err(|_| EnrollmentError::Contract)?;
            if bytes.len() > 4096 {
                return Err(EnrollmentError::Limit);
            }
            files::write(&context.enrollment, PENDING, &bytes, self.uid)?;
            files::rename(&context.enrollment, PENDING, ENROLLMENT, true)?;
            hook(Checkpoint::EnrollmentRenamed)?;
            context.enrollment.sync().map_err(files::fs_error)?;
        }
        Ok(InstallReceipt {
            revision,
            inventory_sha256: digest.into(),
            target: self.target.clone(),
        })
    }
    fn cleanup(&self, digest: &str, hook: Hook<'_>) -> Result<()> {
        if !manifest::hex(digest, 64) {
            return Err(EnrollmentError::Contract);
        }
        let context = self.context()?;
        let current = self.current(&context)?;
        if current
            .as_ref()
            .is_some_and(|value| value.inventory_sha256 == digest)
        {
            return Err(EnrollmentError::Busy);
        }
        recovery::recover(&context, self.uid, &self.target, current.as_ref(), hook)?;
        // Recovery may leave a retiring directory whose lease is held. Do not
        // report its cleanup complete merely because its old public name is gone.
        match files::child(&context.versions, &format!("{RETIRING}{digest}"), self.uid) {
            Ok(_) => return Err(EnrollmentError::Busy),
            Err(EnrollmentError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let generation = match files::child(&context.versions, digest, self.uid) {
            Ok(generation) => generation,
            Err(EnrollmentError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(())
            }
            Err(error) => return Err(error),
        };
        let lease = filesystem::Directory::from_file(generation.0.try_clone()?, self.uid)
            .file(LEASE, Some(0o444))?;
        files::exclusive(&lease)?;
        let retiring = format!("{RETIRING}{digest}");
        // macOS test fixtures need owner write on the directory being renamed;
        // production only exposes this operation on Linux with root authority.
        files::mode(&generation.0, 0o700)?;
        files::rename(&context.versions, digest, &retiring, false)?;
        context.versions.sync().map_err(files::fs_error)?;
        recovery::remove_generation(
            &context.versions,
            &retiring,
            generation,
            Some(lease),
            self.uid,
            &self.target,
            hook,
        )
    }
}
fn require_disk(dir: &Dir, bytes: u64) -> Result<()> {
    if bytes > MAX_BUNDLE {
        return Err(EnrollmentError::Limit);
    }
    let mut state = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::fstatvfs(dir.0.as_raw_fd(), state.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let state = unsafe { state.assume_init() };
    let free = (state.f_bavail as u64).saturating_mul(state.f_frsize as u64);
    if free < bytes.saturating_add(DISK_RESERVE) {
        return Err(EnrollmentError::Limit);
    }
    Ok(())
}
