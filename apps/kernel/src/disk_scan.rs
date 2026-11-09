//! How much disk a folder takes, looked at without interpreting what is in it.
//! Every scan is bounded: a tree bigger than the budget reads as what was seen
//! so far.
use std::path::Path;

/// Bytes allocated by `path` and everything below it, links not followed.
pub(crate) fn size_of(path: &Path, budget: &mut usize) -> u64 {
    if *budget == 0 {
        return 0;
    }
    *budget -= 1;
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if !meta.is_dir() {
        return allocated(&meta);
    }
    let mut total = 0;
    if let Ok(children) = std::fs::read_dir(path) {
        for child in children.flatten() {
            total += size_of(&child.path(), budget);
        }
    }
    total
}

#[cfg(unix)]
fn allocated(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.blocks() * 512
}

#[cfg(not(unix))]
fn allocated(meta: &std::fs::Metadata) -> u64 {
    meta.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_budget_stops_a_huge_tree() {
        let root = std::env::temp_dir().join(format!("disk-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("big")).unwrap();
        std::fs::create_dir_all(root.join("small")).unwrap();
        std::fs::write(root.join("big/blob"), vec![1u8; 1024 * 1024]).unwrap();
        std::fs::write(root.join("small/note"), b"x").unwrap();
        let mut budget = 100;
        assert!(size_of(&root.join("big"), &mut budget) >= 1024 * 1024);
        assert!(size_of(&root.join("small"), &mut budget) < 1024 * 1024);
        let mut none = 0;
        assert_eq!(size_of(&root, &mut none), 0);
        let _ = std::fs::remove_dir_all(&root);
    }
}
