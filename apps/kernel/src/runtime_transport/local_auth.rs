//! Local credentials on the kernel websocket handshake.
//!
//! Kernels started with `CHARIOX_KERNEL_LOCAL_AUTH_TOKEN(_FILE)` (managed and
//! hosted workers) require that host token, unchanged. Every other kernel (the
//! laptop kernel) generates a fresh token at each start, writes it to an
//! owner-only file in its state directory, and accepts it on the same
//! `Authorization: Bearer` header. Until enforcement ships, the laptop kernel
//! runs in log mode: upgrades without its token, or with a wrong one, are still
//! accepted and only logged, rate-limited and without any token value.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use rand::RngCore;
use serde_json::Value;
use tokio_tungstenite::tungstenite::http::HeaderValue;

use crate::config::DaemonConfig;
use crate::runtime::command::KernelCommandSource;

/// Recognizable prefix, so log redaction and secret scanners can find a leak.
pub(crate) const KERNEL_LOCAL_AUTH_TOKEN_PREFIX: &str = "chx_kat_";
const KERNEL_LOCAL_AUTH_TOKEN_RANDOM_BYTES: usize = 32;
const LOCAL_AUTH_WARNING_INTERVAL: Duration = Duration::from_secs(60);
const LOCAL_AUTH_LOG_COMPONENT: &str = "daemon.runtime_transport.local_auth";

/// How the kernel websocket checks the local credential of an upgrade.
#[derive(Clone)]
pub(crate) enum KernelLocalAuth {
    /// No credential configured: only the test-harness entry point without a
    /// host token runs like this. Upgrades are accepted unchecked.
    Unconfigured,
    /// The host controller's token. Upgrades without it are refused.
    HostToken(Arc<str>),
    /// The laptop kernel's generated token, in log mode.
    LocalToken(Arc<LocalTokenAuth>),
}

/// What an accepted connection presented on its handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KernelLocalCredential {
    HostToken,
    LocalToken,
    /// No `Authorization` header (accepted in log mode).
    Missing,
    /// An `Authorization` header that is not this kernel's token (accepted in
    /// log mode).
    Wrong,
    Unchecked,
}

impl KernelLocalCredential {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::HostToken => "host_token",
            Self::LocalToken => "local_token",
            Self::Missing => "missing",
            Self::Wrong => "wrong",
            Self::Unchecked => "unchecked",
        }
    }
}

impl KernelLocalAuth {
    /// The host token if one is configured; otherwise unchecked. Used by the
    /// test-harness entry point, which publishes no local presence either.
    pub(crate) fn host_token_or_unconfigured(host_token: Option<Arc<str>>) -> Self {
        match host_token {
            Some(token) => Self::HostToken(token),
            None => Self::Unconfigured,
        }
    }

    /// The credential for a kernel that serves local clients on `local_addr`.
    /// Without a host token this generates the laptop token and writes its
    /// file; the returned guard removes the file when the server stops.
    pub(crate) fn for_local_kernel(
        host_token: Option<Arc<str>>,
        local_addr: std::io::Result<SocketAddr>,
    ) -> (Self, Option<LocalAuthTokenFile>) {
        if let Some(token) = host_token {
            return (Self::HostToken(token), None);
        }
        let auth = Arc::new(LocalTokenAuth::new(generate_kernel_local_auth_token()));
        let token_file = match local_addr {
            Ok(local_addr) => {
                let path = DaemonConfig::default_kernel_local_auth_token_path(local_addr.port());
                match LocalAuthTokenFile::write(path.clone(), &auth.token) {
                    Ok(file) => {
                        crate::logging::info_with_fields(
                            LOCAL_AUTH_LOG_COMPONENT,
                            "kernel local auth token written",
                            serde_json::json!({ "path": path, "enforcement": "log" }),
                        );
                        Some(file)
                    }
                    Err(error) => {
                        crate::logging::warn_with_fields(
                            LOCAL_AUTH_LOG_COMPONENT,
                            "failed to write the kernel local auth token; local clients cannot present it",
                            serde_json::json!({ "path": path, "error": error.to_string() }),
                        );
                        None
                    }
                }
            }
            Err(error) => {
                crate::logging::warn_with_fields(
                    LOCAL_AUTH_LOG_COMPONENT,
                    "kernel websocket address unavailable; local auth token not written",
                    serde_json::json!({ "error": error.to_string() }),
                );
                None
            }
        };
        (Self::LocalToken(auth), token_file)
    }

    pub(crate) fn required(&self) -> bool {
        matches!(self, Self::HostToken(_))
    }

    pub(crate) fn mode_label(&self) -> &'static str {
        match self {
            Self::Unconfigured => "unconfigured",
            Self::HostToken(_) => "host_token",
            Self::LocalToken(_) => "local_token_log",
        }
    }

    /// Decide an upgrade from its `Authorization` header. `None` refuses it.
    pub(crate) fn admit(
        &self,
        authorization: Option<&HeaderValue>,
    ) -> Option<KernelLocalCredential> {
        match self {
            Self::Unconfigured => Some(KernelLocalCredential::Unchecked),
            Self::HostToken(token) => presented_token_matches(authorization, token)
                .then_some(KernelLocalCredential::HostToken),
            Self::LocalToken(auth) => Some(if authorization.is_none() {
                KernelLocalCredential::Missing
            } else if presented_token_matches(authorization, &auth.token) {
                KernelLocalCredential::LocalToken
            } else {
                KernelLocalCredential::Wrong
            }),
        }
    }

    /// Record an accepted connection; warns, rate-limited, when a laptop
    /// kernel connection did not present its token.
    pub(crate) fn record(&self, credential: KernelLocalCredential, peer_addr: Option<SocketAddr>) {
        let Self::LocalToken(auth) = self else {
            return;
        };
        if let Some(warning) = auth.observe(credential, peer_addr, Instant::now()) {
            crate::logging::warn_with_fields(
                LOCAL_AUTH_LOG_COMPONENT,
                warning.message,
                warning.fields,
            );
        }
    }
}

fn presented_token_matches(authorization: Option<&HeaderValue>, expected_token: &str) -> bool {
    kernel_local_authorization_matches(
        authorization.and_then(|value| value.to_str().ok()),
        expected_token,
    )
}

fn kernel_local_authorization_matches(value: Option<&str>, expected_token: &str) -> bool {
    let Some(token) = value.and_then(|value| value.strip_prefix("Bearer ")) else {
        return false;
    };
    let expected = expected_token.as_bytes();
    let supplied = token.as_bytes();
    let mut difference = expected.len() ^ supplied.len();
    for (index, byte) in expected.iter().enumerate() {
        difference |= usize::from(*byte ^ supplied.get(index).copied().unwrap_or_default());
    }
    difference == 0
}

pub(crate) fn generate_kernel_local_auth_token() -> String {
    let mut random = [0_u8; KERNEL_LOCAL_AUTH_TOKEN_RANDOM_BYTES];
    rand::rngs::OsRng.fill_bytes(&mut random);
    format!(
        "{KERNEL_LOCAL_AUTH_TOKEN_PREFIX}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random)
    )
}

/// The laptop token and its log-mode bookkeeping.
pub(crate) struct LocalTokenAuth {
    token: Arc<str>,
    authenticated: AtomicU64,
    warnings: Mutex<LocalAuthWarningState>,
}

#[derive(Default)]
struct LocalAuthWarningState {
    missing: LocalAuthWarningClass,
    wrong: LocalAuthWarningClass,
}

#[derive(Default)]
struct LocalAuthWarningClass {
    total: u64,
    suppressed: u64,
    last_warned_at: Option<Instant>,
}

pub(crate) struct LocalAuthWarning {
    pub(crate) message: &'static str,
    pub(crate) fields: Value,
}

impl LocalTokenAuth {
    pub(crate) fn new(token: String) -> Self {
        Self {
            token: Arc::from(token),
            authenticated: AtomicU64::new(0),
            warnings: Mutex::new(LocalAuthWarningState::default()),
        }
    }

    #[cfg(test)]
    pub(crate) fn token(&self) -> &str {
        &self.token
    }

    #[cfg(test)]
    pub(crate) fn counts(&self) -> (u64, u64, u64) {
        let warnings = self
            .warnings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (
            self.authenticated.load(Ordering::Relaxed),
            warnings.missing.total,
            warnings.wrong.total,
        )
    }

    /// Count a connection. Returns the warning to log for a missing or wrong
    /// token, at most once per class and interval; it never holds a token.
    pub(crate) fn observe(
        &self,
        credential: KernelLocalCredential,
        peer_addr: Option<SocketAddr>,
        now: Instant,
    ) -> Option<LocalAuthWarning> {
        let mut warnings = self
            .warnings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (class, message) = match credential {
            KernelLocalCredential::LocalToken => {
                self.authenticated.fetch_add(1, Ordering::Relaxed);
                return None;
            }
            KernelLocalCredential::Missing => (
                &mut warnings.missing,
                "kernel websocket connection without the local auth token accepted (log mode)",
            ),
            KernelLocalCredential::Wrong => (
                &mut warnings.wrong,
                "kernel websocket connection with a wrong local auth token accepted (log mode)",
            ),
            KernelLocalCredential::HostToken | KernelLocalCredential::Unchecked => return None,
        };
        class.total += 1;
        if class
            .last_warned_at
            .is_some_and(|last| now.saturating_duration_since(last) < LOCAL_AUTH_WARNING_INTERVAL)
        {
            class.suppressed += 1;
            return None;
        }
        let suppressed = std::mem::take(&mut class.suppressed);
        class.last_warned_at = Some(now);
        let total = class.total;
        Some(LocalAuthWarning {
            message,
            fields: serde_json::json!({
                "credential": credential.as_str(),
                "enforcement": "log",
                "transport_source": KernelCommandSource::LocalCli,
                "peer_addr": peer_addr.map(|addr| addr.to_string()),
                "peer_loopback": peer_addr.map(|addr| addr.ip().is_loopback()),
                "suppressed_since_last_warning": suppressed,
                "total_since_start": total,
                "authenticated_since_start": self.authenticated.load(Ordering::Relaxed),
                "warning_interval_secs": LOCAL_AUTH_WARNING_INTERVAL.as_secs(),
            }),
        })
    }
}

/// The laptop kernel's token file. Each start replaces it; dropping the guard
/// removes it unless another kernel has replaced it since.
pub(crate) struct LocalAuthTokenFile {
    path: PathBuf,
    #[cfg(unix)]
    identity: (u64, u64),
}

impl LocalAuthTokenFile {
    pub(crate) fn write(path: PathBuf, token: &str) -> std::io::Result<Self> {
        let directory = path
            .parent()
            .ok_or_else(|| std::io::Error::other("local auth token path has no parent"))?;
        prepare_private_directory(directory)?;
        crate::config::write_private_file(&path, format!("{token}\n").as_bytes())?;
        #[cfg(unix)]
        let identity = {
            use std::os::unix::fs::MetadataExt;
            let metadata = std::fs::symlink_metadata(&path)?;
            (metadata.dev(), metadata.ino())
        };
        Ok(Self {
            path,
            #[cfg(unix)]
            identity,
        })
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for LocalAuthTokenFile {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let Ok(metadata) = std::fs::symlink_metadata(&self.path) else {
                return;
            };
            if (metadata.dev(), metadata.ino()) != self.identity {
                return;
            }
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Create the token directory owner-only and refuse one that another user
/// owns or that is a symlink.
fn prepare_private_directory(directory: &Path) -> std::io::Result<()> {
    if let Some(parent) = directory.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let metadata = std::fs::symlink_metadata(directory)?;
    if !metadata.is_dir() {
        return Err(std::io::Error::other(
            "local auth token directory is not a directory",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err(std::io::Error::other(
                "local auth token directory is not owned by the kernel user",
            ));
        }
        if metadata.permissions().mode() & 0o077 != 0 {
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
