//! The gateways of other kernels that reached this kernel for their agents
//! (`gateway-origins.json`, this user only). A gateway is listed with the paired
//! clients so the owner can see it, and revoking it refuses its key from then on.
//! The key is the relay's authenticated client key; the origin kernel's name is
//! what the gateway says about itself and is only shown.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};

use crate::error::DaemonError;
use crate::runtime::terminal_pairings::public_key_thumbprint;

/// A listed gateway's client id is this prefix plus its key thumbprint.
pub(crate) const CLIENT_PREFIX: &str = "agent-gateway:";
/// How often a busy gateway rewrites its `last_seen_ms`.
const SEEN_REFRESH_MS: u64 = 60_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct GatewayOrigin {
    pub(crate) key_thumbprint: String,
    pub(crate) origin_kernel: String,
    /// How the origin names itself to its owner; absent in files written before it existed.
    #[serde(default)]
    pub(crate) origin_name: Option<String>,
    pub(crate) first_seen_ms: u64,
    pub(crate) last_seen_ms: u64,
    pub(crate) revoked: bool,
}

impl GatewayOrigin {
    pub(crate) fn client_id(&self) -> String {
        format!("{CLIENT_PREFIX}{}", self.key_thumbprint)
    }

    /// What the owner sees in the paired clients list.
    pub(crate) fn display_name(&self) -> &str {
        self.origin_name.as_deref().unwrap_or(&self.origin_kernel)
    }
}

/// The origin a gateway request names for itself.
#[derive(Clone, Copy)]
pub(crate) struct Named<'a> {
    pub(crate) kernel: &'a str,
    pub(crate) name: Option<&'a str>,
}

/// What the cache holds per file, so a revoke is seen by the next request.
static CACHE: LazyLock<Mutex<BTreeMap<PathBuf, Vec<GatewayOrigin>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

pub(crate) fn path_in(state_root: &Path) -> PathBuf {
    state_root.join("gateway-origins.json")
}

fn unavailable(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "gateway origins",
        message,
    }
}

fn load(path: &Path) -> Result<Vec<GatewayOrigin>, DaemonError> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| unavailable(format!("gateway-origins.json is unreadable: {error}"))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(unavailable(format!(
            "cannot read gateway-origins.json: {error}"
        ))),
    }
}

/// Runs `change` on the file's origins; it says whether they changed and must be stored.
fn with<R>(
    path: &Path,
    change: impl FnOnce(&mut Vec<GatewayOrigin>) -> (R, bool),
) -> Result<R, DaemonError> {
    let mut cache = CACHE.lock().expect("gateway origins poisoned");
    if !cache.contains_key(path) {
        cache.insert(path.to_path_buf(), load(path)?);
    }
    let origins = cache.get_mut(path).expect("just inserted");
    let (result, changed) = change(origins);
    if changed {
        let bytes = serde_json::to_vec_pretty(origins).expect("gateway origins serialize");
        crate::config::write_private_file(path, &bytes)
            .map_err(|error| unavailable(format!("cannot store gateway-origins.json: {error}")))?;
    }
    Ok(result)
}

/// A request arrived with this relay client key. A request that names its origin
/// kernel comes from a gateway and is recorded; any request from a revoked key is refused.
/// Returns false for a revoked key.
pub(crate) fn admit(
    path: &Path,
    client_public_key: &str,
    origin: Option<Named<'_>>,
    now_ms: u64,
) -> Result<bool, DaemonError> {
    let thumbprint = public_key_thumbprint(client_public_key);
    with(path, |origins| {
        match origins.iter_mut().find(|o| o.key_thumbprint == thumbprint) {
            Some(known) if known.revoked => (false, false),
            Some(known) => {
                let Some(origin) = origin else {
                    return (true, false);
                };
                // A renamed origin shows its new name at once; a busy one rewrites `last_seen_ms` rarely.
                let renamed = known.origin_name.as_deref() != origin.name;
                let stale = now_ms.saturating_sub(known.last_seen_ms) >= SEEN_REFRESH_MS;
                if renamed {
                    known.origin_name = origin.name.map(str::to_owned);
                }
                if renamed || stale {
                    known.last_seen_ms = now_ms;
                }
                (true, renamed || stale)
            }
            None => match origin {
                Some(origin) => {
                    origins.push(GatewayOrigin {
                        key_thumbprint: thumbprint,
                        origin_kernel: origin.kernel.to_string(),
                        origin_name: origin.name.map(str::to_owned),
                        first_seen_ms: now_ms,
                        last_seen_ms: now_ms,
                        revoked: false,
                    });
                    (true, true)
                }
                None => (true, false),
            },
        }
    })
}

pub(crate) fn entries(path: &Path) -> Result<Vec<GatewayOrigin>, DaemonError> {
    with(path, |origins| (origins.clone(), false))
}

/// Revokes the gateway with this client id; `None` when it is not one of the listed gateways.
pub(crate) fn revoke(path: &Path, client_id: &str) -> Result<Option<GatewayOrigin>, DaemonError> {
    with(path, |origins| {
        match origins.iter_mut().find(|o| o.client_id() == client_id) {
            Some(found) => {
                found.revoked = true;
                (Some(found.clone()), true)
            }
            None => (None, false),
        }
    })
}

/// The owner paired this key again with a link they just made: whatever revoked it earlier is over.
pub(crate) fn reinstate(
    path: &Path,
    key_thumbprint: &str,
) -> Result<Option<GatewayOrigin>, DaemonError> {
    with(path, |origins| {
        match origins
            .iter_mut()
            .find(|o| o.key_thumbprint == key_thumbprint && o.revoked)
        {
            Some(found) => {
                found.revoked = false;
                (Some(found.clone()), true)
            }
            None => (None, false),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mac(name: Option<&'static str>) -> Option<Named<'static>> {
        Some(Named {
            kernel: "mac-kernel",
            name,
        })
    }

    fn file() -> PathBuf {
        let dir = PathBuf::from(std::env::var("TMPDIR").unwrap_or_else(|_| "/var/tmp".into()))
            .join(format!(
                "gateway-origins-{}-{}",
                std::process::id(),
                crate::session::unix_epoch_ms()
            ));
        std::fs::create_dir_all(&dir).unwrap();
        path_in(&dir)
    }

    #[test]
    fn a_gateway_is_listed_once_and_revoking_it_refuses_its_key() {
        let path = file();
        // An ordinary terminal names no origin: it is not recorded.
        assert!(admit(&path, "terminal-key", None, 1).unwrap());
        assert!(entries(&path).unwrap().is_empty());
        for at in [10, 20, 100_000] {
            assert!(admit(&path, "gateway-key", mac(None), at).unwrap());
        }
        let listed = entries(&path).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            (listed[0].first_seen_ms, listed[0].last_seen_ms),
            (10, 100_000)
        );
        let id = listed[0].client_id();
        assert!(revoke(&path, "agent-gateway:unknown").unwrap().is_none());
        assert!(revoke(&path, &id).unwrap().unwrap().revoked);
        // The key is refused with or without a name, and the other key still works.
        assert!(!admit(&path, "gateway-key", mac(None), 200_000).unwrap());
        assert!(!admit(&path, "gateway-key", None, 200_001).unwrap());
        assert!(admit(&path, "terminal-key", None, 3).unwrap());
        // A restart reads the same answer from the file.
        CACHE.lock().unwrap().remove(&path);
        assert!(!admit(&path, "gateway-key", None, 300_000).unwrap());
        // A new pairing link made by the owner brings the key back; nothing else does.
        assert!(reinstate(&path, "no-such-thumbprint").unwrap().is_none());
        assert!(reinstate(&path, &public_key_thumbprint("gateway-key"))
            .unwrap()
            .is_some());
        assert!(admit(&path, "gateway-key", mac(None), 400_000).unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_gateway_is_listed_by_the_name_its_kernel_gives_and_a_rename_shows_at_once() {
        let path = file();
        assert!(admit(&path, "gateway-key", mac(Some("mac/dev")), 10).unwrap());
        assert_eq!(entries(&path).unwrap()[0].display_name(), "mac/dev");
        // Soon after, so no `last_seen_ms` refresh is due: the new name is still stored.
        assert!(admit(&path, "gateway-key", mac(Some("mac/main")), 20).unwrap());
        assert_eq!(entries(&path).unwrap()[0].display_name(), "mac/main");
        // A kernel without a name is listed by its id, as a file written before names was.
        assert!(admit(&path, "other-key", mac(None), 30).unwrap());
        let listed = entries(&path).unwrap();
        assert_eq!(listed[1].display_name(), "mac-kernel");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
