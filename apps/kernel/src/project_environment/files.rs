//! Files outside Git: the paths a Project lists travel with its checkouts. Moves and slices pick
//! them up through the ignore rules; a new worktree gets them copied.
use super::{environment_error, relative_project_path};
use crate::error::DaemonError;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 128 * 1024 * 1024;

/// Checkout folder -> the listed paths, so path-only callers (ignore rules) need no kernel state.
static LISTED: RwLock<BTreeMap<PathBuf, Vec<String>>> = RwLock::new(BTreeMap::new());

pub(crate) fn register_environment_files(checkouts: &[PathBuf], files: &[String]) {
    let mut listed = LISTED
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for checkout in checkouts {
        if files.is_empty() {
            listed.remove(checkout);
        } else {
            listed.insert(checkout.clone(), files.to_vec());
        }
    }
}

/// Ignore rules that let the listed files through a `.gitignore`.
pub(crate) fn environment_file_rules(checkout: &Path) -> Vec<String> {
    LISTED
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(checkout)
        .map(|files| files.iter().map(|path| format!("!{path}")).collect())
        .unwrap_or_default()
}

/// Copies the listed files that `source` has and `target` lacks, with owner-only permissions.
pub(crate) fn copy_environment_files(
    source: &Path,
    target: &Path,
    files: &[String],
) -> Result<usize, DaemonError> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut copied = 0;
    let mut total = 0_u64;
    for path in files {
        relative_project_path(path).map_err(environment_error)?;
        let from = source.join(path);
        let to = target.join(path);
        let metadata = match fs::symlink_metadata(&from) {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => continue,
        };
        if to.exists() {
            continue;
        }
        total = total.saturating_add(metadata.len());
        if metadata.len() > MAX_FILE_BYTES || total > MAX_TOTAL_BYTES {
            return Err(environment_error(format!(
                "{path} is too large to copy into a new checkout"
            )));
        }
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)
                .map_err(|_| environment_error("creating a folder in the new checkout failed"))?;
        }
        let mut input = fs::File::open(&from)
            .map_err(|_| environment_error(format!("{path} could not be read")))?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&to)
            .map_err(|_| environment_error(format!("{path} could not be written")))?;
        std::io::copy(&mut input, &mut output)
            .map_err(|_| environment_error(format!("{path} could not be copied")))?;
        fs::set_permissions(&to, fs::Permissions::from_mode(0o600))
            .map_err(|_| environment_error(format!("{path} could not be secured")))?;
        copied += 1;
    }
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("chariox-env-files-{:032x}", rand::random::<u128>()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn listed_files_are_copied_once_with_owner_only_mode_and_missing_ones_are_skipped() {
        use std::os::unix::fs::PermissionsExt;
        let root = scratch();
        let (source, target) = (root.join("main"), root.join("worktree"));
        fs::create_dir_all(source.join("config")).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(source.join("config/secrets.json"), b"{}").unwrap();
        fs::write(source.join(".env"), b"A=1").unwrap();
        fs::write(target.join(".env"), b"kept").unwrap();
        let files = vec!["config/secrets.json".into(), ".env".into(), "absent".into()];
        assert_eq!(copy_environment_files(&source, &target, &files).unwrap(), 1);
        let copied = target.join("config/secrets.json");
        assert_eq!(fs::read(&copied).unwrap(), b"{}");
        assert_eq!(
            fs::metadata(&copied).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(fs::read(target.join(".env")).unwrap(), b"kept");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn registered_files_become_include_rules_for_their_checkout_only() {
        let (a, b) = (
            PathBuf::from("/chariox/test/a"),
            PathBuf::from("/chariox/test/b"),
        );
        register_environment_files(std::slice::from_ref(&a), &["config/secrets.json".into()]);
        assert_eq!(
            environment_file_rules(&a),
            vec!["!config/secrets.json".to_string()]
        );
        assert!(environment_file_rules(&b).is_empty());
        register_environment_files(std::slice::from_ref(&a), &[]);
        assert!(environment_file_rules(&a).is_empty());
    }
}
