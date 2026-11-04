//! How much disk a folder takes, looked at without interpreting what is in it.
//! Every scan is bounded: a tree bigger than the budget reads as what was seen
//! so far.
use std::path::{Path, PathBuf};

/// Entries one scan may look at.
pub(crate) const BUDGET: usize = 200_000;

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

/// The `count` biggest entries directly inside `dir`, biggest first.
pub(crate) fn biggest_children(dir: &Path, count: usize) -> Vec<(PathBuf, u64)> {
    let mut budget = BUDGET;
    let mut children: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|child| {
            let path = child.path();
            let bytes = size_of(&path, &mut budget);
            (path, bytes)
        })
        .collect();
    children.sort_by(|a, b| b.1.cmp(&a.1));
    children.truncate(count);
    children
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_biggest_folders_come_first_and_a_budget_stops_a_huge_tree() {
        let root = std::env::temp_dir().join(format!("disk-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("big")).unwrap();
        std::fs::create_dir_all(root.join("small")).unwrap();
        std::fs::write(root.join("big/blob"), vec![1u8; 1024 * 1024]).unwrap();
        std::fs::write(root.join("small/note"), b"x").unwrap();
        let biggest = biggest_children(&root, 1);
        assert_eq!(biggest.len(), 1);
        assert!(biggest[0].0.ends_with("big"));
        assert!(biggest[0].1 >= 1024 * 1024);
        let mut none = 0;
        assert_eq!(size_of(&root, &mut none), 0);
        let _ = std::fs::remove_dir_all(&root);
    }
}
