//! MP-08 / MP-10 / MP-11: only installed manifests and contained pnpm store links prove packages.
use super::detect_index::EvidenceRoot;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use zeroize::Zeroizing;

fn read(root: &EvidenceRoot, path: &str) -> Result<Zeroizing<String>, ()> {
    let file = root.file(path).map_err(|_| ())?;
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(());
    }
    Ok(Zeroizing::new(
        std::str::from_utf8(&bytes).map_err(|_| ())?.into(),
    ))
}
fn contained_target(parent: &Path, target: &Path, name: &str) -> Result<String, ()> {
    if target.is_absolute() {
        return Err(());
    }
    let mut resolved = parent.to_path_buf();
    for component in target.components() {
        match component {
            Component::Normal(part) => resolved.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                if !resolved.pop() {
                    return Err(());
                }
            }
            _ => return Err(()),
        }
    }
    if !resolved.starts_with("node_modules/.pnpm")
        || !resolved.ends_with(PathBuf::from("node_modules").join(name))
    {
        return Err(());
    }
    resolved.push("package.json");
    resolved.to_str().map(str::to_owned).ok_or(())
}
#[cfg(unix)]
fn same_link(before: &std::fs::Metadata, after: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    before.file_type().is_symlink()
        && after.file_type().is_symlink()
        && before.dev() == after.dev()
        && before.ino() == after.ino()
        && before.mtime() == after.mtime()
        && before.mtime_nsec() == after.mtime_nsec()
        && before.ctime() == after.ctime()
        && before.ctime_nsec() == after.ctime_nsec()
}
pub(super) fn installed_manifest(root: &Path, name: &str) -> Result<Zeroizing<String>, ()> {
    #[cfg(test)]
    if let Some(marker) = std::env::var_os("CHARIOX_TYPED_CHECK_STALL_MARKER") {
        std::fs::write(marker, std::process::id().to_string()).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    // Keep the workspace anchor pinned across the link resolution and read.
    let root = EvidenceRoot::open(root).map_err(|_| ())?;
    let manifest = format!("node_modules/{name}/package.json");
    if let Ok(text) = read(&root, &manifest) {
        return Ok(text);
    }
    #[cfg(unix)]
    {
        let package = PathBuf::from("node_modules").join(name);
        let parent = package.parent().ok_or(())?;
        let (_anchor, directory) = root.directory(parent.to_str().ok_or(())?).map_err(|_| ())?;
        let link = directory.join(package.file_name().ok_or(())?);
        let before = std::fs::symlink_metadata(&link).map_err(|_| ())?;
        if !before.file_type().is_symlink() {
            return Err(());
        }
        let target = std::fs::read_link(&link).map_err(|_| ())?;
        let resolved = contained_target(parent, &target, name)?;
        // The single package-manager link is interpreted explicitly. Every
        // store component and the manifest still uses the no-follow reader.
        let text = read(&root, &resolved)?;
        let after = std::fs::symlink_metadata(&link).map_err(|_| ())?;
        if !same_link(&before, &after) || std::fs::read_link(&link).map_err(|_| ())? != target {
            return Err(());
        }
        Ok(text)
    }
    #[cfg(not(unix))]
    {
        Err(())
    }
}
