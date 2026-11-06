//! Management authentication by the kernel's identity key.
//!
//! A kernel signs every management request (method, path, generator, owner id, body, time and
//! a single-use nonce) with its relay identity key. The AEGS verifies the signature, derives the
//! kernel id from the key, applies the operator's admission policy and then asks the store
//! whether that kernel belongs to the request's secret owner id. No token is issued by anyone.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chariox_event_protocol::kernel_identity::{
    kernel_id_for_public_key, verify, ManagementRequest, KERNEL_KEY_HEADER, KERNEL_NONCE_HEADER,
    KERNEL_SIGNATURE_HEADER, KERNEL_TIMESTAMP_HEADER, OWNER_ID_HEADER,
};
use hyper::header::HeaderMap;
use hyper::StatusCode;

/// How far a request's timestamp may be from the AEGS clock (five minutes: a user's machine may
/// be a little off). The nonce makes a request single-use inside this window; outside it the
/// request is stale.
const SIGNATURE_WINDOW_MS: u64 = 300_000;
const MAX_REMEMBERED_NONCES: usize = 100_000;
const MAX_NONCE_BYTES: usize = 128;
const MAX_OWNER_ID_BYTES: usize = 256;
const MAX_ALLOWLIST_BYTES: u64 = 1024 * 1024;
const DEFAULT_OPEN_MAX_OWNERS_PER_KERNEL: usize = 8;
const DEFAULT_OPEN_MAX_REQUESTS_PER_MINUTE: u32 = 600;
const MAX_RATE_WINDOWS: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuthenticatedKernel {
    pub(crate) kernel_id: String,
    pub(crate) owner_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    SignatureRequired,
    SignatureInvalid,
    SignatureStale,
    SignatureReplayed,
    NonceMemoryFull,
    OwnerRequired,
    OwnerInvalid,
    NotAdmitted,
    RateLimited,
}

impl Refusal {
    pub(crate) fn status_and_code(self) -> (StatusCode, &'static str) {
        match self {
            Self::SignatureRequired => (StatusCode::UNAUTHORIZED, "kernel_signature_required"),
            Self::SignatureInvalid => (StatusCode::UNAUTHORIZED, "kernel_signature_invalid"),
            Self::SignatureStale => (StatusCode::UNAUTHORIZED, "kernel_signature_stale"),
            Self::SignatureReplayed => (StatusCode::UNAUTHORIZED, "kernel_signature_replayed"),
            Self::NonceMemoryFull => (StatusCode::SERVICE_UNAVAILABLE, "nonce_memory_full"),
            Self::OwnerRequired => (StatusCode::FORBIDDEN, "owner_required"),
            Self::OwnerInvalid => (StatusCode::FORBIDDEN, "owner_invalid"),
            Self::NotAdmitted => (StatusCode::FORBIDDEN, "kernel_not_admitted"),
            Self::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
        }
    }

    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::SignatureRequired => {
                "management requests are signed with the kernel identity key"
            }
            Self::SignatureInvalid => {
                "the signature does not match the request and the key it names"
            }
            Self::SignatureStale => "the request timestamp is outside the accepted window",
            Self::SignatureReplayed => "the request nonce was already used",
            Self::NonceMemoryFull => "too many recent requests; retry shortly",
            Self::OwnerRequired => "the request must name the owner id it acts for",
            Self::OwnerInvalid => "the owner id is longer than 256 bytes or not printable ASCII",
            Self::NotAdmitted => "the operator's admission policy does not list this kernel",
            Self::RateLimited => "the kernel exceeded its request budget",
        }
    }
}

/// Which kernels may use this AEGS at all. Isolation between users never depends on it: the
/// secret owner id and the per-connection audience decide who sees what.
#[derive(Debug, Clone)]
pub(crate) enum Admission {
    Open {
        max_owners_per_kernel: usize,
        max_requests_per_minute: u32,
    },
    /// Only kernel ids listed in a JSON object file (`{"<kernel_id>": "<label>"}`), read on
    /// every request, so removing an id takes effect without a restart.
    Allowlist { file: PathBuf },
}

impl Admission {
    pub(crate) fn from_env(loopback: bool) -> Result<Self, String> {
        let policy = std::env::var("CHARIOX_AEGS_ADMISSION")
            .ok()
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty());
        match policy.as_deref() {
            Some("open") => Self::open_from_env(),
            None if loopback => Self::open_from_env(),
            Some("allowlist") => {
                let file = std::env::var_os("CHARIOX_AEGS_ALLOWLIST_FILE")
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from)
                    .ok_or(
                        "CHARIOX_AEGS_ADMISSION=allowlist requires CHARIOX_AEGS_ALLOWLIST_FILE",
                    )?;
                Ok(Self::Allowlist { file })
            }
            Some(other) => Err(format!(
                "CHARIOX_AEGS_ADMISSION `{other}` is not open or allowlist"
            )),
            None => Err(
                "a non-loopback AEGS requires CHARIOX_AEGS_ADMISSION=open or allowlist".to_string(),
            ),
        }
    }

    fn open_from_env() -> Result<Self, String> {
        Ok(Self::Open {
            max_owners_per_kernel: parse_positive(
                "CHARIOX_AEGS_OPEN_MAX_OWNERS_PER_KERNEL",
                DEFAULT_OPEN_MAX_OWNERS_PER_KERNEL,
            )?,
            max_requests_per_minute: parse_positive(
                "CHARIOX_AEGS_OPEN_MAX_REQUESTS_PER_MINUTE",
                DEFAULT_OPEN_MAX_REQUESTS_PER_MINUTE,
            )?,
        })
    }

    pub(crate) fn max_owners_per_kernel(&self) -> Option<usize> {
        match self {
            Self::Open {
                max_owners_per_kernel,
                ..
            } => Some(*max_owners_per_kernel),
            Self::Allowlist { .. } => None,
        }
    }

    async fn admits(&self, kernel_id: &str) -> bool {
        match self {
            Self::Open { .. } => true,
            Self::Allowlist { file } => match read_allowlist(file).await {
                Ok(listed) => listed.contains(kernel_id),
                Err(error) => {
                    eprintln!("AEGS allowlist {} is unreadable: {error}", file.display());
                    false
                }
            },
        }
    }
}

fn parse_positive<T: std::str::FromStr + PartialOrd + Default>(
    name: &str,
    default: T,
) -> Result<T, String> {
    let Some(value) = std::env::var(name).ok().filter(|value| !value.is_empty()) else {
        return Ok(default);
    };
    let parsed = value
        .parse::<T>()
        .map_err(|_| format!("invalid {name}: not a number"))?;
    if parsed <= T::default() {
        return Err(format!("{name} must be greater than zero"));
    }
    Ok(parsed)
}

async fn read_allowlist(file: &PathBuf) -> Result<BTreeSet<String>, String> {
    let metadata = tokio::fs::metadata(file)
        .await
        .map_err(|error| error.to_string())?;
    if metadata.len() > MAX_ALLOWLIST_BYTES {
        return Err("the file exceeds 1 MiB".to_string());
    }
    let text = tokio::fs::read_to_string(file)
        .await
        .map_err(|error| error.to_string())?;
    let entries: std::collections::BTreeMap<String, serde_json::Value> =
        serde_json::from_str(&text).map_err(|error| error.to_string())?;
    Ok(entries.into_keys().collect())
}

#[derive(Clone)]
pub(crate) struct ManagementAuth {
    admission: Admission,
    seen_nonces: Arc<Mutex<HashMap<(String, String), u64>>>,
    request_windows: Arc<Mutex<HashMap<String, (u64, u32)>>>,
}

impl ManagementAuth {
    pub(crate) fn new(admission: Admission) -> Self {
        Self {
            admission,
            seen_nonces: Arc::default(),
            request_windows: Arc::default(),
        }
    }

    pub(crate) fn max_owners_per_kernel(&self) -> Option<usize> {
        self.admission.max_owners_per_kernel()
    }

    /// Verifies the signature, then the operator's admission and request budget. The caller
    /// still asks the store whether the kernel belongs to the returned owner id.
    pub(crate) async fn authenticate(
        &self,
        headers: &HeaderMap,
        method: &str,
        path: &str,
        generator_id: &str,
        body: &[u8],
        now_ms: u64,
    ) -> Result<AuthenticatedKernel, Refusal> {
        let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
        let (Some(public_key), Some(timestamp), Some(nonce), Some(signature)) = (
            header(KERNEL_KEY_HEADER),
            header(KERNEL_TIMESTAMP_HEADER),
            header(KERNEL_NONCE_HEADER),
            header(KERNEL_SIGNATURE_HEADER),
        ) else {
            return Err(Refusal::SignatureRequired);
        };
        let owner_id = header(OWNER_ID_HEADER).unwrap_or("");
        let timestamp_ms: u64 = timestamp.parse().map_err(|_| Refusal::SignatureInvalid)?;
        if nonce.is_empty() || nonce.len() > MAX_NONCE_BYTES {
            return Err(Refusal::SignatureInvalid);
        }
        if timestamp_ms.abs_diff(now_ms) > SIGNATURE_WINDOW_MS {
            return Err(Refusal::SignatureStale);
        }
        let message = ManagementRequest {
            method,
            path,
            generator_id,
            owner_id,
            timestamp_ms,
            nonce,
            body,
        }
        .message();
        verify(public_key, &message, signature).map_err(|_| Refusal::SignatureInvalid)?;
        let kernel_id = kernel_id_for_public_key(public_key);
        self.remember_nonce(&kernel_id, nonce, timestamp_ms, now_ms)?;
        if owner_id.is_empty() {
            return Err(Refusal::OwnerRequired);
        }
        if owner_id.len() > MAX_OWNER_ID_BYTES
            || !owner_id.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(Refusal::OwnerInvalid);
        }
        if !self.admission.admits(&kernel_id).await {
            return Err(Refusal::NotAdmitted);
        }
        self.spend_request_budget(&kernel_id, now_ms)?;
        Ok(AuthenticatedKernel {
            kernel_id,
            owner_id: owner_id.to_string(),
        })
    }

    /// A verified nonce is remembered until its timestamp leaves the window, so a recorded
    /// request cannot be sent again.
    fn remember_nonce(
        &self,
        kernel_id: &str,
        nonce: &str,
        timestamp_ms: u64,
        now_ms: u64,
    ) -> Result<(), Refusal> {
        let mut seen = self
            .seen_nonces
            .lock()
            .map_err(|_| Refusal::NonceMemoryFull)?;
        if seen.len() >= MAX_REMEMBERED_NONCES {
            seen.retain(|_, expires_at_ms| *expires_at_ms > now_ms);
            if seen.len() >= MAX_REMEMBERED_NONCES {
                return Err(Refusal::NonceMemoryFull);
            }
        }
        let key = (kernel_id.to_string(), nonce.to_string());
        if seen.contains_key(&key) {
            return Err(Refusal::SignatureReplayed);
        }
        seen.insert(key, timestamp_ms.saturating_add(SIGNATURE_WINDOW_MS));
        Ok(())
    }

    fn spend_request_budget(&self, kernel_id: &str, now_ms: u64) -> Result<(), Refusal> {
        let Admission::Open {
            max_requests_per_minute,
            ..
        } = &self.admission
        else {
            return Ok(());
        };
        let mut windows = self
            .request_windows
            .lock()
            .map_err(|_| Refusal::RateLimited)?;
        if windows.len() >= MAX_RATE_WINDOWS {
            windows.retain(|_, (started_at_ms, _)| now_ms.saturating_sub(*started_at_ms) < 60_000);
        }
        if windows.len() >= MAX_RATE_WINDOWS && !windows.contains_key(kernel_id) {
            return Err(Refusal::RateLimited);
        }
        let window = windows.entry(kernel_id.to_string()).or_insert((now_ms, 0));
        if now_ms.saturating_sub(window.0) >= 60_000 {
            *window = (now_ms, 0);
        }
        window.1 += 1;
        if window.1 > *max_requests_per_minute {
            return Err(Refusal::RateLimited);
        }
        Ok(())
    }
}

#[cfg(test)]
impl ManagementAuth {
    /// Open admission with the default budgets, for tests that drive the server directly.
    pub(crate) fn open_for_tests() -> Self {
        Self::new(Admission::Open {
            max_owners_per_kernel: DEFAULT_OPEN_MAX_OWNERS_PER_KERNEL,
            max_requests_per_minute: u32::MAX,
        })
    }
}
