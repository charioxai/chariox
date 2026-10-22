//! Reports host-space preparation refusals in App logs, at most once per
//! owner and installation every ten minutes.
use super::LifecycleError;
use crate::durable_state::DurableKernelStateStore;
use chariox_app_runtime::worker_process::{HostDiskSpace, WorkerError};
use std::{collections::BTreeMap, sync::Mutex};

const QUIET_MS: u64 = 10 * 60 * 1000;
static NOTIFIED: Mutex<BTreeMap<(String, String), u64>> = Mutex::new(BTreeMap::new());

/// A refused preparation keeps the stable preparation code, unless disk
/// space is what refused it.
#[cfg(test)]
pub(super) fn preparation_error(error: WorkerError) -> LifecycleError {
    match error {
        WorkerError::HostDiskSpace(space) => LifecycleError::DiskSpace(space),
        _ => LifecycleError::Preparation,
    }
}

/// Preserve platform diagnostics for ordinary refusals and retain numeric
/// headroom for the owner notice.
pub(super) fn preparation_error_with_diagnostic(
    error: WorkerError,
    diagnostic: impl FnOnce(WorkerError) -> LifecycleError,
) -> LifecycleError {
    match error {
        WorkerError::HostDiskSpace(space) => LifecycleError::DiskSpace(space),
        other => diagnostic(other),
    }
}

pub(super) fn notify(
    store: &DurableKernelStateStore,
    owner: &str,
    installation: &str,
    space: HostDiskSpace,
) {
    if !due(owner, installation, crate::session::unix_epoch_ms()) {
        return;
    }
    crate::logging::warn_with_fields(
        "app.worker",
        "App start refused: not enough free disk space",
        serde_json::json!({
            "installation_id": installation,
            "free_bytes": space.free,
            "needed_bytes": space.needed,
        }),
    );
    let mut fields = serde_json::Map::new();
    fields.insert(
        "disk_space".into(),
        serde_json::json!({"free_bytes": space.free, "needed_bytes": space.needed}),
    );
    let _ = store.append_app_kernel_notice(owner, installation, &space.message(), fields);
}

fn due(owner: &str, installation: &str, now_ms: u64) -> bool {
    let mut notified = NOTIFIED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    due_in(&mut notified, owner, installation, now_ms)
}

fn due_in(
    notified: &mut BTreeMap<(String, String), u64>,
    owner: &str,
    installation: &str,
    now_ms: u64,
) -> bool {
    notified.retain(|_, at| now_ms.saturating_sub(*at) < QUIET_MS);
    let key = (owner.to_owned(), installation.to_owned());
    if notified.contains_key(&key) {
        return false;
    }
    notified.insert(key, now_ms);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_disk_space_leaves_the_stable_preparation_code() {
        let space = HostDiskSpace { free: 1, needed: 2 };
        assert_eq!(
            preparation_error(WorkerError::HostDiskSpace(space)),
            LifecycleError::DiskSpace(space)
        );
        assert_eq!(
            LifecycleError::DiskSpace(space).to_string(),
            "app_lifecycle_disk_space"
        );
        for other in [WorkerError::Preparation, WorkerError::Identity] {
            assert_eq!(preparation_error(other), LifecycleError::Preparation);
        }
    }

    #[test]
    fn the_owner_is_told_once_per_installation_every_ten_minutes() {
        let owner = format!("owner-{:016x}", rand::random::<u64>());
        let mut notices = BTreeMap::new();
        assert!(due_in(&mut notices, &owner, "docs", 1_000));
        assert!(!due_in(&mut notices, &owner, "docs", 1_000 + QUIET_MS - 1));
        assert!(due_in(&mut notices, &owner, "todo", 1_000 + QUIET_MS - 1));
        assert!(due_in(
            &mut notices,
            "other-owner",
            "docs",
            1_000 + QUIET_MS - 1
        ));
        assert!(due_in(&mut notices, &owner, "docs", 1_000 + QUIET_MS));
    }

    #[test]
    fn ordinary_platform_preparation_refusals_keep_their_diagnostics() {
        let mut observed = Vec::new();
        for error in [
            WorkerError::Preparation,
            WorkerError::Storage("app_storage_identity"),
        ] {
            assert_eq!(
                preparation_error_with_diagnostic(error, |other| {
                    observed.push(other.to_string());
                    LifecycleError::Preparation
                }),
                LifecycleError::Preparation,
            );
        }
        assert_eq!(
            observed,
            [
                "app_worker_preparation",
                "app_worker_storage:app_storage_identity"
            ]
        );
    }

    #[test]
    fn macos_preparation_refusal_reaches_the_numeric_owner_notice() {
        let root =
            std::env::temp_dir().join(format!("chariox-disk-space-{:016x}", rand::random::<u64>()));
        std::fs::create_dir(&root).unwrap();
        let store = DurableKernelStateStore::open_owned(root.join("kernel.sqlite")).unwrap();
        let owner = format!("owner-{:016x}", rand::random::<u64>());
        let gib = 1024 * 1024 * 1024;
        let space = HostDiskSpace {
            free: 7 * gib + gib / 2,
            needed: 8 * gib,
        };
        let error = preparation_error_with_diagnostic(WorkerError::HostDiskSpace(space), |_| {
            LifecycleError::Preparation
        });
        assert_eq!(error, LifecycleError::DiskSpace(space));
        let LifecycleError::DiskSpace(retained) = error else {
            panic!("macOS preparation lost the numeric refusal");
        };
        notify(&store, &owner, "docs", retained);
        // A retried start within the quiet period adds nothing.
        notify(&store, &owner, "docs", space);
        let entries = store.app_logs(&owner, "docs", 0, 10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].level, "warn");
        assert_eq!(
            entries[0].message,
            "Not enough free disk space: Apps need 8 GiB free on the host (7.5 GiB free). \
             Free at least 0.5 GiB."
        );
        assert_eq!(
            entries[0].fields,
            serde_json::json!({
                "kernel": true,
                "disk_space": {"free_bytes": space.free, "needed_bytes": space.needed},
            })
        );
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }
}
