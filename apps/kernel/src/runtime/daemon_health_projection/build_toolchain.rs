//! Setup publishes a version only after validating the managed environment.

pub(super) fn installed_version() -> Option<String> {
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        return None;
    }
    read_version(&std::env::home_dir()?)
}

fn read_version(home: &std::path::Path) -> Option<String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;

    let root = home.join(".local/share/chariox/build-toolchain");
    if !root.join("env/bin/x86_64-conda-linux-gnu-cc").is_file() {
        return None;
    }
    let mut marker = String::new();
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(root.join("version"))
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > 32 {
        return None;
    }
    file.take(33).read_to_string(&mut marker).ok()?;
    let version = marker.trim();
    (!version.is_empty()
        && version.len() <= 10
        && version.bytes().all(|byte| byte.is_ascii_digit()))
    .then(|| version.to_owned())
}

#[cfg(test)]
mod tests {
    use super::read_version;

    #[test]
    fn build_toolchain_version_requires_the_published_marker_and_compiler() {
        let home = crate::test_support::TestWorktree::new("build-toolchain");
        let root = home.path().join(".local/share/chariox/build-toolchain");
        let bin = root.join("env/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let compiler = bin.join("x86_64-conda-linux-gnu-cc");
        std::fs::write(&compiler, "").unwrap();
        assert_eq!(read_version(home.path()), None);

        let marker = root.join("version");
        std::fs::write(&marker, "42\n").unwrap();
        assert_eq!(read_version(home.path()).as_deref(), Some("42"));
        std::fs::write(&marker, "bad\n").unwrap();
        assert_eq!(read_version(home.path()), None);

        std::fs::write(&marker, "42\n").unwrap();
        std::fs::remove_file(compiler).unwrap();
        assert_eq!(read_version(home.path()), None);
    }
}
