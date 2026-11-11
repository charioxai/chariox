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
/// How long a read allowlist stays in use. Every request from a key the list does not name
/// would otherwise read and parse the file (up to 1 MiB), and any key can sign a request.
const ALLOWLIST_CACHE_MS: u64 = 5_000;
const DEFAULT_OPEN_MAX_OWNERS_PER_KERNEL: usize = 8;
const DEFAULT_OPEN_MAX_REQUESTS_PER_MINUTE: u32 = 600;
/// Across all kernels: 500 requests a second.
const DEFAULT_OPEN_MAX_REQUESTS_PER_MINUTE_TOTAL: u32 = 30_000;
/// Kernel and owner pairs across the gateway: about 2 500 kernels at the per-kernel cap.
const DEFAULT_OPEN_MAX_MEMBERSHIPS_TOTAL: usize = 20_000;
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
    GatewayRateLimited,
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
            Self::GatewayRateLimited => (StatusCode::TOO_MANY_REQUESTS, "gateway_rate_limited"),
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
            Self::GatewayRateLimited => "the gateway is at its request budget for all kernels",
        }
    }
}

/// Which kernels may use this AEGS at all. Isolation between users never depends on it: the
/// secret owner id and the per-connection audience decide who sees what.
#[derive(Debug, Clone)]
pub(crate) enum Admission {
    /// Per-kernel caps, and gateway-wide caps on requests and on kernel and owner pairs, so
    /// that many free keys cannot add up to more than the gateway carries.
    Open {
        max_owners_per_kernel: usize,
        max_requests_per_minute: u32,
        max_requests_per_minute_total: u32,
        max_memberships_total: usize,
    },
    /// Only kernel ids listed in a JSON object file (`{"<kernel_id>": "<label>"}`), read again
    /// every few seconds, so removing an id takes effect without a restart.
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
            max_requests_per_minute_total: parse_positive(
                "CHARIOX_AEGS_OPEN_MAX_REQUESTS_PER_MINUTE_TOTAL",
                DEFAULT_OPEN_MAX_REQUESTS_PER_MINUTE_TOTAL,
            )?,
            max_memberships_total: parse_positive(
                "CHARIOX_AEGS_OPEN_MAX_MEMBERSHIPS_TOTAL",
                DEFAULT_OPEN_MAX_MEMBERSHIPS_TOTAL,
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

    pub(crate) fn max_memberships_total(&self) -> Option<usize> {
        match self {
            Self::Open {
                max_memberships_total,
                ..
            } => Some(*max_memberships_total),
            Self::Allowlist { .. } => None,
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

/// The allowlist as last read; an unreadable file counts as an empty list until the next read.
#[derive(Default)]
struct AllowlistCache {
    read_at_ms: Option<u64>,
    listed: BTreeSet<String>,
}

#[derive(Clone)]
pub(crate) struct ManagementAuth {
    admission: Admission,
    allowlist: Arc<tokio::sync::Mutex<AllowlistCache>>,
    seen_nonces: Arc<Mutex<HashMap<(String, String), u64>>>,
    request_windows: Arc<Mutex<HashMap<String, (u64, u32)>>>,
    /// The gateway-wide request window; the lock covers a counter update only.
    gateway_window: Arc<Mutex<(u64, u32)>>,
}

impl ManagementAuth {
    pub(crate) fn new(admission: Admission) -> Self {
        Self {
            admission,
            allowlist: Arc::default(),
            seen_nonces: Arc::default(),
            request_windows: Arc::default(),
            gateway_window: Arc::default(),
        }
    }

    pub(crate) fn max_owners_per_kernel(&self) -> Option<usize> {
        self.admission.max_owners_per_kernel()
    }

    pub(crate) fn max_memberships_total(&self) -> Option<usize> {
        self.admission.max_memberships_total()
    }

    /// Verifies the signature, then the operator's admission and request budget. The nonce is
    /// remembered last, for a request that is going to be served: any key signs a request, and a
    /// refused one must not fill the replay memory of the admitted kernels. The caller still asks
    /// the store whether the kernel belongs to the returned owner id.
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
        self.ensure_unseen(&kernel_id, nonce)?;
        if owner_id.is_empty() {
            return Err(Refusal::OwnerRequired);
        }
        if owner_id.len() > MAX_OWNER_ID_BYTES
            || !owner_id.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(Refusal::OwnerInvalid);
        }
        if !self.admits(&kernel_id, now_ms).await {
            return Err(Refusal::NotAdmitted);
        }
        self.spend_request_budget(&kernel_id, now_ms)?;
        self.remember_nonce(&kernel_id, nonce, timestamp_ms, now_ms)?;
        Ok(AuthenticatedKernel {
            kernel_id,
            owner_id: owner_id.to_string(),
        })
    }

    async fn admits(&self, kernel_id: &str, now_ms: u64) -> bool {
        let Admission::Allowlist { file } = &self.admission else {
            return true;
        };
        let mut cache = self.allowlist.lock().await;
        let fresh = cache.read_at_ms.is_some_and(|read_at_ms| {
            (read_at_ms..read_at_ms.saturating_add(ALLOWLIST_CACHE_MS)).contains(&now_ms)
        });
        if !fresh {
            cache.listed = read_allowlist(file).await.unwrap_or_else(|error| {
                eprintln!("AEGS allowlist {} is unreadable: {error}", file.display());
                BTreeSet::new()
            });
            cache.read_at_ms = Some(now_ms);
        }
        cache.listed.contains(kernel_id)
    }

    /// A replay is refused before it can spend an admitted kernel's request budget.
    fn ensure_unseen(&self, kernel_id: &str, nonce: &str) -> Result<(), Refusal> {
        let seen = self
            .seen_nonces
            .lock()
            .map_err(|_| Refusal::NonceMemoryFull)?;
        if seen.contains_key(&(kernel_id.to_string(), nonce.to_string())) {
            return Err(Refusal::SignatureReplayed);
        }
        Ok(())
    }

    /// A served request's nonce is remembered until its timestamp leaves the window, so a
    /// recorded request cannot be sent again.
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
            max_requests_per_minute_total,
            ..
        } = &self.admission
        else {
            return Ok(());
        };
        {
            let mut window = self
                .gateway_window
                .lock()
                .map_err(|_| Refusal::GatewayRateLimited)?;
            if now_ms.saturating_sub(window.0) >= 60_000 {
                *window = (now_ms, 0);
            }
            window.1 += 1;
            if window.1 > *max_requests_per_minute_total {
                return Err(Refusal::GatewayRateLimited);
            }
        }
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
            max_requests_per_minute_total: u32::MAX,
            max_memberships_total: usize::MAX,
        })
    }
}

#[cfg(test)]
mod admission_order_tests {
    use super::*;
    use base64::Engine as _;
    use sha2::Digest as _;

    fn kernel(name: &str) -> (String, String) {
        let secret = p256::SecretKey::from_slice(&sha2::Sha256::digest(name.as_bytes())).unwrap();
        let public = p256::elliptic_curve::sec1::ToEncodedPoint::to_encoded_point(
            &secret.public_key(),
            false,
        );
        (
            base64::engine::general_purpose::STANDARD.encode(secret.to_bytes()),
            base64::engine::general_purpose::STANDARD.encode(public.as_bytes()),
        )
    }

    fn headers(name: &str, owner: &str, nonce: &str, now_ms: u64) -> HeaderMap {
        let (private, public) = kernel(name);
        let signature = chariox_event_protocol::kernel_identity::sign(
            &private,
            &ManagementRequest {
                method: "POST",
                path: "/v1/subscriptions/reconcile",
                generator_id: "dev.chariox.github",
                owner_id: owner,
                timestamp_ms: now_ms,
                nonce,
                body: b"{}",
            }
            .message(),
        )
        .unwrap();
        let mut map = HeaderMap::new();
        for (header, value) in [
            (KERNEL_KEY_HEADER, public),
            (KERNEL_TIMESTAMP_HEADER, now_ms.to_string()),
            (KERNEL_NONCE_HEADER, nonce.to_string()),
            (KERNEL_SIGNATURE_HEADER, signature),
            (OWNER_ID_HEADER, owner.to_string()),
        ] {
            map.insert(header, value.parse().unwrap());
        }
        map
    }

    async fn authenticate(
        auth: &ManagementAuth,
        name: &str,
        owner: &str,
        nonce: &str,
        now_ms: u64,
    ) -> Result<AuthenticatedKernel, Refusal> {
        auth.authenticate(
            &headers(name, owner, nonce, now_ms),
            "POST",
            "/v1/subscriptions/reconcile",
            "dev.chariox.github",
            b"{}",
            now_ms,
        )
        .await
    }

    fn allowlist_dir(tag: &str, listed: &[&str]) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("aegs-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("allowlist.json");
        let entries = listed
            .iter()
            .map(|name| {
                (
                    kernel_id_for_public_key(&kernel(name).1),
                    "listed".to_string(),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        std::fs::write(&file, serde_json::to_string(&entries).unwrap()).unwrap();
        (dir, file)
    }

    // Any key signs a request, so a request from a kernel the operator did not admit (or one
    // that names no owner) must not use up the memory that protects the admitted kernels
    // from replay: the nonce is remembered only for a request that is going to be served.
    #[tokio::test]
    async fn a_refused_request_does_not_take_a_nonce_slot() {
        let (dir, file) = allowlist_dir("admission-order", &[]);
        let auth = ManagementAuth::new(Admission::Allowlist { file });
        let now = 1_700_000_000_000;
        for index in 0..3 {
            let refused = authenticate(
                &auth,
                &format!("stranger-{index}"),
                "owner",
                &format!("n{index}"),
                now,
            )
            .await;
            assert_eq!(refused, Err(Refusal::NotAdmitted));
        }
        let too_long = "x".repeat(MAX_OWNER_ID_BYTES + 1);
        for (index, owner) in ["", too_long.as_str()].into_iter().enumerate() {
            let refused = authenticate(&auth, "stranger-0", owner, &format!("o{index}"), now).await;
            assert!(refused.is_err());
        }
        let held = auth.seen_nonces.lock().unwrap().len();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            held == 0,
            "{held} nonces of refused requests are held in the replay memory"
        );
    }

    #[tokio::test]
    async fn an_admitted_request_is_served_once_and_a_replay_is_refused() {
        let (dir, file) = allowlist_dir("admitted-replay", &["listed"]);
        let auth = ManagementAuth::new(Admission::Allowlist { file });
        let now = 1_700_000_000_000;
        let first = authenticate(&auth, "listed", "owner", "n1", now).await;
        let second = authenticate(&auth, "listed", "owner", "n1", now).await;
        let _ = std::fs::remove_dir_all(&dir);
        assert!(first.is_ok(), "{first:?}");
        assert_eq!(second, Err(Refusal::SignatureReplayed));
    }

    // A replay of a request the budget refused must not be served later either, and a replay
    // of an admitted request must not spend the kernel's budget again.
    #[tokio::test]
    async fn a_replayed_request_does_not_spend_the_budget() {
        let auth = ManagementAuth::new(Admission::Open {
            max_owners_per_kernel: 8,
            max_requests_per_minute: 2,
            max_requests_per_minute_total: 1000,
            max_memberships_total: 1000,
        });
        let now = 1_700_000_000_000;
        assert!(authenticate(&auth, "busy", "owner", "n1", now)
            .await
            .is_ok());
        for _ in 0..5 {
            assert_eq!(
                authenticate(&auth, "busy", "owner", "n1", now).await,
                Err(Refusal::SignatureReplayed)
            );
        }
        assert!(authenticate(&auth, "busy", "owner", "n2", now)
            .await
            .is_ok());
        assert_eq!(
            authenticate(&auth, "busy", "owner", "n3", now).await,
            Err(Refusal::RateLimited)
        );
    }

    // Anonymous keys must not make the AEGS read and parse the allowlist file per request:
    // within a short interval the file is read once, and a change is still picked up later.
    #[tokio::test]
    async fn unadmitted_requests_do_not_reread_the_allowlist_each_time() {
        let (dir, file) = allowlist_dir("allowlist-cache", &[]);
        let auth = ManagementAuth::new(Admission::Allowlist { file: file.clone() });
        let now = 1_700_000_000_000;
        assert_eq!(
            authenticate(&auth, "late", "owner", "n0", now).await,
            Err(Refusal::NotAdmitted)
        );
        // The operator lists the kernel; requests inside the interval still see the cached list.
        let listed = serde_json::json!({ kernel_id_for_public_key(&kernel("late").1): "late" });
        std::fs::write(&file, listed.to_string()).unwrap();
        assert_eq!(
            authenticate(&auth, "late", "owner", "n1", now + 1_000).await,
            Err(Refusal::NotAdmitted),
            "the allowlist was read again inside the cache interval"
        );
        // After the interval the change takes effect without a restart.
        let after = authenticate(&auth, "late", "owner", "n2", now + 60_000).await;
        let _ = std::fs::remove_dir_all(&dir);
        assert!(after.is_ok(), "{after:?}");
    }

    // A read that fails while the operator rewrites the file must not turn the last good list
    // into an empty one: the kernels it admitted stay admitted until a read succeeds.
    #[tokio::test]
    async fn a_failed_reread_keeps_the_last_good_allowlist() {
        let (dir, file) = allowlist_dir("allowlist-keep", &["listed"]);
        let auth = ManagementAuth::new(Admission::Allowlist { file: file.clone() });
        let now = 1_700_000_000_000;
        assert!(authenticate(&auth, "listed", "owner", "n0", now)
            .await
            .is_ok());

        std::fs::write(&file, "{ not json").unwrap();
        let garbage = authenticate(&auth, "listed", "owner", "n1", now + 60_000).await;

        std::fs::remove_file(&file).unwrap();
        let missing = authenticate(&auth, "listed", "owner", "n2", now + 120_000).await;
        let stranger = authenticate(&auth, "stranger", "owner", "n3", now + 120_000).await;

        let _ = std::fs::remove_dir_all(&dir);
        assert!(garbage.is_ok(), "garbage file: {garbage:?}");
        assert!(missing.is_ok(), "missing file: {missing:?}");
        assert_eq!(stranger, Err(Refusal::NotAdmitted));
    }

    // With no successful read ever, nobody is admitted; the first good read admits.
    #[tokio::test]
    async fn a_never_readable_allowlist_admits_nobody_until_it_reads() {
        let (dir, file) = allowlist_dir("allowlist-never", &["listed"]);
        std::fs::write(&file, "{ not json").unwrap();
        let auth = ManagementAuth::new(Admission::Allowlist { file: file.clone() });
        let now = 1_700_000_000_000;
        let refused = authenticate(&auth, "listed", "owner", "n0", now).await;

        let listed = serde_json::json!({ kernel_id_for_public_key(&kernel("listed").1): "listed" });
        std::fs::write(&file, listed.to_string()).unwrap();
        let admitted = authenticate(&auth, "listed", "owner", "n1", now + 60_000).await;

        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(refused, Err(Refusal::NotAdmitted));
        assert!(admitted.is_ok(), "{admitted:?}");
    }

    // A successful read replaces the list, and a parsed empty list means nobody is admitted.
    #[tokio::test]
    async fn a_successful_reread_replaces_the_list_even_when_it_is_empty() {
        let (dir, file) = allowlist_dir("allowlist-replace", &["first"]);
        let auth = ManagementAuth::new(Admission::Allowlist { file: file.clone() });
        let now = 1_700_000_000_000;
        assert!(authenticate(&auth, "first", "owner", "n0", now)
            .await
            .is_ok());

        let second = serde_json::json!({ kernel_id_for_public_key(&kernel("second").1): "second" });
        std::fs::write(&file, second.to_string()).unwrap();
        let swapped_out = authenticate(&auth, "first", "owner", "n1", now + 60_000).await;
        let swapped_in = authenticate(&auth, "second", "owner", "n2", now + 60_000).await;

        std::fs::write(&file, "{}").unwrap();
        let emptied = authenticate(&auth, "second", "owner", "n3", now + 120_000).await;

        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(swapped_out, Err(Refusal::NotAdmitted));
        assert!(swapped_in.is_ok(), "{swapped_in:?}");
        assert_eq!(emptied, Err(Refusal::NotAdmitted));
    }
}
