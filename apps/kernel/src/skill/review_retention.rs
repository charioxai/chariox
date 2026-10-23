//! MP-08 / MP-10 / MP-11: bounded reclamation of expired reviewed instruction bytes.
//! Only a matching durable receipt can authorize deleting a generated review root.
use super::*;

const MAX_SWEEP_REMOVALS: usize = 128;

pub(super) fn sweep(
    service: &SkillCatalogService,
    now_ms: u64,
) -> Result<(), crate::error::DaemonError> {
    // Serialize sweeps with publication and other sweepers, without holding a
    // coordinator/App lock during filesystem work.
    let _lock = service.storage.lock()?;
    // An old handle cannot clean another home journal's current reviews.
    service.storage.snapshot()?;
    let relative = Path::new("state/skill-reviews");
    let path = service.storage.home().join(relative);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(failure("cannot inspect skill review retention state")),
        Ok(_) => {}
    }
    let root = super::super::paths::existing_owned_directory(service.storage.home(), relative)?;
    let mut removed = 0;
    for entry in
        fs::read_dir(&root).map_err(|_| failure("cannot list skill review retention state"))?
    {
        if removed >= MAX_SWEEP_REMOVALS {
            break;
        }
        let entry = entry.map_err(|_| failure("cannot inspect skill review retention entry"))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(suffix) = name.strip_prefix("review-") else {
            continue;
        };
        if suffix.len() != 32
            || !suffix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            continue;
        }
        let metadata = match fs::symlink_metadata(entry.path()) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(failure("cannot inspect skill review directory")),
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.uid() != unsafe { libc::geteuid() } {
                continue;
            }
        }
        let events = service
            .storage
            .journal()
            .load_subject_events_by_kind(name, REVIEW_KIND, 1)?;
        let Some(event) = events.last() else {
            continue;
        };
        let Ok(review) = serde_json::from_value::<StoredReview>(event.payload.clone()) else {
            continue;
        };
        if review.receipt.id != name || review.receipt.expires_at_ms > now_ms {
            continue;
        }
        // Resolve the complete owned chain again before deletion. Symlink roots
        // and unrelated files never become review cleanup targets.
        let owned = match super::super::paths::existing_owned_directory(
            service.storage.home(),
            &relative.join(name),
        ) {
            Ok(owned) => owned,
            Err(_)
                if fs::symlink_metadata(entry.path())
                    .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
            {
                continue
            }
            Err(error) => return Err(error),
        };
        match fs::remove_dir_all(&owned) {
            Ok(()) => removed += 1,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(failure("expired skill review cleanup failed")),
        }
    }
    if removed > 0 {
        fs::File::open(root)
            .and_then(|file| file.sync_all())
            .map_err(|_| failure("cannot persist skill review cleanup"))?;
    }
    Ok(())
}
