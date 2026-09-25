//! Signed UI files for an App view, taken from the active release after it is
//! re-verified against the owner's current publisher trust.
use super::DurableKernelStateStore;

/// UI bytes sent to the browser controller in one relayed request (about
/// 2.7 MiB as base64). Larger views need chunked delivery first.
const MAX_VIEW_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug)]
pub(crate) struct AppViewAsset {
    pub(crate) path: String,
    pub(crate) content_type: &'static str,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct AppViewAssets {
    /// The active generation these files belong to.
    pub(crate) generation: u64,
    /// Entry path relative to `ui/`.
    pub(crate) entry: String,
    pub(crate) assets: Vec<AppViewAsset>,
}

impl DurableKernelStateStore {
    pub(crate) fn app_view_assets(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<AppViewAssets, &'static str> {
        let release = self
            .active_app_release(owner, installation)
            .map_err(|_| "app_view_release_unavailable")?;
        let verified = release.verify().map_err(|_| "app_view_release_invalid")?;
        let entry = verified
            .manifest()
            .ui
            .entry
            .strip_prefix("ui/")
            .ok_or("app_view_release_invalid")?
            .to_owned();
        let mut total = 0;
        let mut assets = Vec::new();
        for (path, bytes) in verified.files() {
            let Some(path) = path.strip_prefix("ui/") else {
                continue;
            };
            total += bytes.len();
            if total > MAX_VIEW_BYTES {
                return Err("app_view_too_large");
            }
            assets.push(AppViewAsset {
                path: path.to_owned(),
                content_type: content_type(path),
                bytes: bytes.to_vec(),
            });
        }
        Ok(AppViewAssets {
            generation: release.generation(),
            entry,
            assets,
        })
    }
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("txt" | "md") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chariox_app_package::{verify, VerificationPolicy};
    use chariox_app_runtime::release_store::{ReleaseStore, StageBudget};

    #[test]
    fn serves_only_signed_ui_files_of_the_owners_active_release() {
        let root = std::env::temp_dir().join(format!(
            "chariox-view-assets-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let store = DurableKernelStateStore::open_owned(root.join("kernel.sqlite")).unwrap();
        crate::durable_state::app_state::fixture_event_catalog(&store);
        // Without its stored archive the release cannot be served.
        assert!(store.app_view_assets("alice", "installed").is_err());
        let (bytes, publisher) = crate::durable_state::app_state::fixture_event_package();
        let verified = verify(
            &bytes,
            &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
        )
        .unwrap();
        let budget = StageBudget {
            max_stage_bytes: 1024 * 1024,
            reserved_bytes: 1024 * 1024,
            host_reserve_bytes: 1024 * 1024,
        };
        ReleaseStore::open_or_create(store.path())
            .unwrap()
            .stage(&verified, &bytes, budget)
            .unwrap();
        let view = store.app_view_assets("alice", "installed").unwrap();
        assert_eq!(view.entry, "index.html");
        let entry = view.assets.iter().find(|a| a.path == "index.html").unwrap();
        assert_eq!(entry.content_type, "text/html; charset=utf-8");
        assert!(view
            .assets
            .iter()
            .all(|a| !a.path.starts_with("runtime/") && !a.path.contains("..")));
        assert!(store.app_view_assets("bob", "installed").is_err());
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }
}
