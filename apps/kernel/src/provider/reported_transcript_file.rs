//! MP-11: keep provider transcript admission and consumption on one descriptor.
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

pub(crate) struct ProviderReportedTranscript {
    pub(crate) path: PathBuf,
    file: File,
}

impl ProviderReportedTranscript {
    pub(super) fn open(path: PathBuf, confined: bool) -> Option<Self> {
        let file = if confined {
            open_without_symlinks(&path)?
        } else {
            File::open(&path).ok()?
        };
        file.metadata()
            .ok()?
            .is_file()
            .then_some(Self { path, file })
    }

    pub(crate) fn read(mut self) -> std::io::Result<String> {
        let mut raw = String::new();
        self.file.read_to_string(&mut raw)?;
        Ok(raw)
    }
}

#[cfg(unix)]
fn open_without_symlinks(path: &std::path::Path) -> Option<File> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Component;

    if !path.is_absolute() {
        return None;
    }
    let components = path
        .components()
        .filter_map(|component| match component {
            Component::RootDir => None,
            Component::Normal(name) => Some(Some(name)),
            _ => Some(None),
        })
        .collect::<Option<Vec<_>>>()?;
    if components.is_empty() {
        return None;
    }
    // Open every component relative to its held parent. No mutable path is
    // reopened after admission, and no ancestor/leaf link can escape the bind.
    let mut opened = File::open("/").ok()?;
    for (index, name) in components.iter().enumerate() {
        let name = CString::new(name.as_bytes()).ok()?;
        let last = index + 1 == components.len();
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | if last {
                libc::O_NONBLOCK
            } else {
                libc::O_DIRECTORY
            };
        // SAFETY: the parent is held open and name is a valid NUL-terminated component.
        let fd = unsafe { libc::openat(opened.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return None;
        }
        // SAFETY: openat returned an owned descriptor, consumed exactly once.
        opened = unsafe { File::from_raw_fd(fd) };
    }
    Some(opened)
}

#[cfg(not(unix))]
fn open_without_symlinks(_path: &std::path::Path) -> Option<File> {
    // Managed account mounts are a Unix contract; fail closed elsewhere.
    None
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    #[test]
    fn confined_reported_transcript_rejects_leaf_ancestor_and_root_links() {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "chariox-mp11-transcript-{:032x}",
            rand::random::<u128>()
        ));
        fs::create_dir(&root).unwrap();
        let account = root.join("account");
        fs::create_dir(&account).unwrap();
        let file = account.join("session.jsonl");
        fs::write(&file, "{}\n").unwrap();
        let link = account.join("link.jsonl");
        symlink(&file, &link).unwrap();
        let alias = root.join("alias");
        symlink(&account, &alias).unwrap();
        let leaf_rejected = ProviderReportedTranscript::open(link, true).is_none();
        let ancestor_rejected =
            ProviderReportedTranscript::open(alias.join("session.jsonl"), true).is_none();
        let directory_rejected = ProviderReportedTranscript::open(account.clone(), true).is_none();
        let normal = ProviderReportedTranscript::open(file, true)
            .unwrap()
            .read()
            .unwrap();
        // Ordinary providers retain their existing symlink behavior.
        let ordinary = ProviderReportedTranscript::open(alias.join("session.jsonl"), false)
            .unwrap()
            .read()
            .unwrap();
        fs::remove_dir_all(root).unwrap();
        assert!(leaf_rejected && ancestor_rejected && directory_rejected);
        assert_eq!(normal, "{}\n");
        assert_eq!(ordinary, normal);
    }
}
