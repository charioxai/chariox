//! Signed UI files for an App view, taken from the active release after it is
//! re-verified against the owner's current publisher trust.
use super::{app_active_release::ActiveReleaseError, DurableKernelStateStore};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppViewAssetsError {
    NotFound,
    Inactive { data_kept: bool },
    Release(ActiveReleaseError),
    TooLarge,
}

impl AppViewAssetsError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::NotFound => "App installation not found for this user. Use /app list to check the installation id.",
            Self::Inactive { data_kept: true } => "This App is uninstalled; its data is kept. Reinstall with /app update <installation-id> <package> before opening it.",
            Self::Inactive { data_kept: false } => "This App has no active release or retained data. Check /app status <installation-id> for a pending installation; otherwise install the package again with /app install <package>.",
            Self::Release(ActiveReleaseError::Untrusted) => "The App publisher is no longer trusted. Review publisher trust before opening it.",
            Self::Release(ActiveReleaseError::Invalid) => "The App's stored release failed verification. Reinstall a valid signed package before opening it.",
            Self::Release(ActiveReleaseError::Unavailable) => "The App is installed, but its stored package is unavailable. Reinstall the package with /app update <installation-id> <package> before opening it.",
            Self::Release(ActiveReleaseError::NotActive) => "The App's active release has no usable trust binding. Review /app status <installation-id> and reinstall the signed package.",
            Self::Release(ActiveReleaseError::Storage) => "App storage is unavailable. Restore access to kernel storage and try again.",
            Self::TooLarge => "The App view exceeds the 2 MiB transfer limit. Ask the publisher for a smaller UI package.",
        }
    }
}

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
    ) -> Result<AppViewAssets, AppViewAssetsError> {
        let record =
            self.get_app_installation(owner, installation)
                .map_err(|error| match error {
                    super::apps::AppRegistryError::Registry(
                        chariox_app_runtime::installation::InstallationError::NotFound,
                    ) => AppViewAssetsError::NotFound,
                    _ => AppViewAssetsError::Release(ActiveReleaseError::Storage),
                })?;
        if record.active.is_none() {
            return Err(AppViewAssetsError::Inactive {
                data_kept: record.retained.is_some(),
            });
        }
        let release = self
            .active_app_release(owner, installation)
            .map_err(AppViewAssetsError::Release)?;
        let verified = release.verify().map_err(AppViewAssetsError::Release)?;
        let entry = verified
            .manifest()
            .ui
            .entry
            .strip_prefix("ui/")
            .ok_or(AppViewAssetsError::Release(ActiveReleaseError::Invalid))?
            .to_owned();
        let mut total = 0;
        let mut assets = Vec::new();
        for (path, bytes) in verified.files() {
            let Some(path) = path.strip_prefix("ui/") else {
                continue;
            };
            total += bytes.len();
            if total > MAX_VIEW_BYTES {
                return Err(AppViewAssetsError::TooLarge);
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
        assert_eq!(
            store.app_view_assets("alice", "installed").unwrap_err(),
            AppViewAssetsError::Release(ActiveReleaseError::Unavailable)
        );
        assert_ne!(
            store.app_view_assets("alice", "installed").unwrap_err(),
            store.app_view_assets("alice", "missing").unwrap_err(),
            "a listed installation with a missing archive must not be reported as missing"
        );
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
        assert_eq!(
            store.app_view_assets("bob", "installed").unwrap_err(),
            AppViewAssetsError::NotFound
        );
        store
            .mutate_app_installation(
                "alice",
                super::super::apps::AppRegistryMutation::Uninstall {
                    installation_id: "installed".into(),
                    expected_generation: view.generation,
                    now_ms: 99,
                },
            )
            .unwrap();
        assert!(store
            .list_app_installations("alice", None, 50)
            .unwrap()
            .installations
            .iter()
            .any(|i| i.installation_id == "installed"));
        let inactive = store.app_view_assets("alice", "installed").unwrap_err();
        assert_eq!(inactive, AppViewAssetsError::Inactive { data_kept: true });
        assert!(inactive.message().contains("uninstalled; its data is kept"));
        assert!(inactive.message().contains("/app update"));
        let record = store.get_app_installation("alice", "installed").unwrap();
        store
            .mutate_app_installation(
                "alice",
                super::super::apps::AppRegistryMutation::ForgetData {
                    installation_id: "installed".into(),
                    expected_generation: record.generation,
                },
            )
            .unwrap();
        let deleted = store.app_view_assets("alice", "installed").unwrap_err();
        assert_eq!(deleted, AppViewAssetsError::Inactive { data_kept: false });
        assert!(deleted
            .message()
            .contains("no active release or retained data"));
        assert!(deleted.message().contains("/app install"));
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }
}
