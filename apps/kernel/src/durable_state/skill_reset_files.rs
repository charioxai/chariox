//! MP-08 / MP-10 / MP-11: bounded, exact legacy-copy cleanup; never delete roots.
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy)]
pub(crate) enum LegacySkillRootKind {
    Installed,
    Materialized,
}
#[derive(Debug, Clone)]
pub(crate) struct LegacySkillRoot {
    pub path: PathBuf,
    pub kind: LegacySkillRootKind,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct OwnedSkillCopy {
    root: PathBuf,
    relative: PathBuf,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}
fn refused() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "legacy skill cleanup ownership changed; startup remains closed",
    )
}
pub(super) fn collect(roots: &[LegacySkillRoot]) -> io::Result<Vec<OwnedSkillCopy>> {
    let mut copies = Vec::new();
    for source in roots {
        if !source.path.is_absolute() {
            return Err(refused());
        }
        match fs::symlink_metadata(&source.path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
            Ok(m) if m.file_type().is_symlink() || !m.is_dir() => return Err(refused()),
            _ => {}
        }
        let root = fs::canonicalize(&source.path)?;
        walk(
            &root,
            &root,
            match source.kind {
                LegacySkillRootKind::Installed => 1,
                LegacySkillRootKind::Materialized => 3,
            },
            &mut copies,
        )?;
    }
    Ok(copies)
}
fn walk(
    root: &Path,
    directory: &Path,
    remaining: usize,
    copies: &mut Vec<OwnedSkillCopy>,
) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        // Never follow symlinks, even to a package. No data outside these exact
        // product-owned roots can become a deletion target.
        if metadata.file_type().is_symlink() {
            continue;
        }
        if !metadata.is_dir() {
            continue;
        }
        let skill = path.join("SKILL.md");
        let has_skill = match fs::symlink_metadata(&skill) {
            Ok(m) => m.is_file() && !m.file_type().is_symlink(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => false,
            Err(e) => return Err(e),
        };
        if has_skill {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if metadata.uid() != unsafe { libc::geteuid() } {
                    return Err(refused());
                }
            }
            copies.push(OwnedSkillCopy {
                root: root.to_path_buf(),
                relative: path
                    .strip_prefix(root)
                    .map_err(|_| refused())?
                    .to_path_buf(),
                #[cfg(unix)]
                device: {
                    use std::os::unix::fs::MetadataExt;
                    metadata.dev()
                },
                #[cfg(unix)]
                inode: {
                    use std::os::unix::fs::MetadataExt;
                    metadata.ino()
                },
            });
        } else if remaining > 1 {
            walk(root, &path, remaining - 1, copies)?;
        }
    }
    Ok(())
}
/// The caller journals this exact plan before invoking cleanup. A crash after
/// SKILL.md was removed still retries the recorded directory, not discovery.
pub(super) fn remove(copies: &[OwnedSkillCopy]) -> io::Result<()> {
    for copy in copies {
        if !copy.root.is_absolute() || copy.relative.as_os_str().is_empty() {
            return Err(refused());
        }
        let root_metadata = fs::symlink_metadata(&copy.root)?;
        if root_metadata.file_type().is_symlink()
            || !root_metadata.is_dir()
            || fs::canonicalize(&copy.root)? != copy.root
        {
            return Err(refused());
        }
        let mut path = copy.root.clone();
        // Recheck every component at deletion, rejecting a swapped ancestor.
        for part in copy.relative.components() {
            let std::path::Component::Normal(part) = part else {
                return Err(refused());
            };
            path.push(part);
            match fs::symlink_metadata(&path) {
                Ok(m) if !m.is_dir() || m.file_type().is_symlink() => return Err(refused()),
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => break,
                Err(e) => return Err(e),
            }
        }
        let metadata = match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
            Ok(m) => m,
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.dev() != copy.device
                || metadata.ino() != copy.inode
                || metadata.uid() != unsafe { libc::geteuid() }
            {
                return Err(refused());
            }
        }
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(refused());
        }
        fs::remove_dir_all(&path)?;
        if let Some(parent) = path.parent() {
            fs::File::open(parent)?.sync_all()?;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Root(PathBuf);
    impl Root {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "skillsk-reset-files-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn package(&self, relative: &str) -> PathBuf {
            let p = self.0.join(relative);
            fs::create_dir_all(&p).unwrap();
            fs::write(p.join("SKILL.md"), "instructions").unwrap();
            p
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn skill_reset_files_removes_exact_legacy_copies_preserves_v2_and_siblings() {
        let root = Root::new();
        let old = root.package("skills/review");
        let new = root.package("skills/packages/skill-1/digest");
        fs::write(root.0.join("skills/unrelated"), "keep").unwrap();
        let plan = collect(&[LegacySkillRoot {
            path: root.0.join("skills"),
            kind: LegacySkillRootKind::Installed,
        }])
        .unwrap();
        assert_eq!(plan.len(), 1);
        remove(&plan).unwrap();
        remove(&plan).unwrap();
        assert!(!old.exists());
        assert!(new.join("SKILL.md").exists());
        assert!(root.0.join("skills/unrelated").exists());
    }
    #[test]
    fn skill_reset_files_materializations_keep_new_lease_layout() {
        let root = Root::new();
        let old = root.package("cache/home/review/digest");
        let new = root.package("cache/home/lease/skill-1/digest");
        let plan = collect(&[LegacySkillRoot {
            path: root.0.join("cache"),
            kind: LegacySkillRootKind::Materialized,
        }])
        .unwrap();
        assert_eq!(plan.len(), 1);
        remove(&plan).unwrap();
        assert!(!old.exists());
        assert!(new.exists());
    }
    #[test]
    #[cfg(unix)]
    fn skill_reset_files_refuses_swapped_target_and_parent_symlinks() {
        let root = Root::new();
        let old = root.package("skills/review");
        let unrelated = root.package("elsewhere/keep");
        let plan = collect(&[LegacySkillRoot {
            path: root.0.join("skills"),
            kind: LegacySkillRootKind::Installed,
        }])
        .unwrap();
        fs::rename(&old, root.0.join("original")).unwrap();
        std::os::unix::fs::symlink(&unrelated, &old).unwrap();
        assert!(remove(&plan).is_err());
        assert!(unrelated.join("SKILL.md").exists());
        fs::remove_file(&old).unwrap();
        fs::remove_dir(old.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(root.0.join("elsewhere"), root.0.join("skills")).unwrap();
        assert!(remove(&plan).is_err());
        assert!(unrelated.exists());
    }
    #[test]
    fn skill_reset_files_journal_plan_survives_partial_package_deletion() {
        let root = Root::new();
        let old = root.package("skills/review");
        fs::write(old.join("helper"), "remaining").unwrap();
        let plan = collect(&[LegacySkillRoot {
            path: root.0.join("skills"),
            kind: LegacySkillRootKind::Installed,
        }])
        .unwrap();
        let journal = serde_json::to_string(&plan).unwrap();
        fs::remove_file(old.join("SKILL.md")).unwrap();
        let recovered: Vec<OwnedSkillCopy> = serde_json::from_str(&journal).unwrap();
        remove(&recovered).unwrap();
        assert!(!old.exists());
    }
}
