//! MP-08 / MP-10 / MP-11: Bounded, no-execution evidence; protected bytes never become a digest oracle.
use super::*;
use crate::error::DaemonError;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use zeroize::Zeroizing;

pub(super) const MAX_FILE: u64 = 16 * 1024 * 1024;
const MAX_TOTAL: u64 = 256 * 1024 * 1024;
const MAX_FILES: usize = 20_000;

pub(super) struct EvidenceFile {
    pub folder_id: String,
    pub path: String,
    pub text: Option<Zeroizing<String>>,
    pub digest: Option<String>,
    pub bytes: Option<u64>,
    pub protected: bool,
}
pub(super) struct EvidenceIndex {
    pub files: Vec<EvidenceFile>,
    pub skips: Vec<EnvironmentItemResult>,
}
pub(super) fn skip(folder: &str, path: &str, reason: &str) -> EnvironmentItemResult {
    EnvironmentItemResult {
        requirement_id: project_environment_item_id(folder, &format!("skip:{path}")),
        status: EnvironmentObservationStatus::NeedsYourInput,
        reason_code: reason.into(),
        safe_summary: format!("Skipped {path} · {}", reason.replace('_', " ")),
        receipt_ids: vec![],
    }
}
fn legacy_key_literal(value: &str) -> bool {
    value
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
        .any(|token| {
            token
                .strip_prefix("sk-")
                .is_some_and(|body| body.len() >= 20)
        })
}
/// Allow metadata, never terminal controls, URL credentials or common key literals.
pub(super) fn safe_metadata(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1024
        && !value.chars().any(char::is_control)
        && !legacy_key_literal(value)
        && ![
            "sk-proj-",
            "sk-ant-",
            "ghp_",
            "github_pat_",
            "AKIA",
            "-----BEGIN",
            "Bearer ",
            "://",
            "?token=",
            "?key=",
        ]
        .iter()
        .any(|marker| value.contains(marker))
}
pub(super) fn credential_content(text: &str) -> bool {
    super::index::contains_secret_configuration(text)
        || legacy_key_literal(text)
        // Names are metadata. Only assignment keys and literal credential patterns are hits.
        || text.split_inclusive([':', '=']).take(100_000).filter_map(|part| {
            part.strip_suffix(':').or_else(|| part.strip_suffix('='))
        }).any(|prefix| {
            let key = prefix
                .rsplit(['{', ',', '\n'])
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches(['\'', '"', ' '])
                .to_ascii_lowercase();
            [
                "password",
                "secret",
                "token",
                "private_key",
                "api_key",
                "apikey",
                "credential",
            ]
            .iter()
            .any(|name| key.contains(name))
        })
        || [
            "sk-proj-",
            "sk-ant-",
            "ghp_",
            "github_pat_",
            "AKIA",
            "-----BEGIN",
            "Bearer ",
        ]
        .iter()
        .any(|marker| text.contains(marker))
        || text.lines().any(|line| {
            // URL userinfo and nonempty dotenv assignments are private by default.
            line.contains("://")
                && line.split("://").skip(1).any(|tail| {
                    tail.split('/')
                        .next()
                        .is_some_and(|authority| authority.contains('@'))
                })
        })
}
pub(super) fn credential_configuration_name(name: &str) -> bool {
    matches!(
        name,
        ".mcp.json"
            | "devcontainer.json"
            | "compose.yaml"
            | "compose.yml"
            | "docker-compose.yaml"
            | "docker-compose.yml"
    )
}
fn value_file(path: &str) -> bool {
    let name = Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    secret_looking_project_path(path)
        && !(name.starts_with(".env")
            && [".example", ".sample", ".template"]
                .iter()
                .any(|s| name.ends_with(s)))
        || name == "auth.json"
        || name == "config.json" && path.split('/').any(|p| p == ".codex" || p == ".claude")
}
pub(super) fn collect(folders: &[EnvironmentFolder]) -> Result<EvidenceIndex, DaemonError> {
    let mut index = EvidenceIndex {
        files: vec![],
        skips: vec![],
    };
    let mut budget = Budget {
        visited: 0,
        bytes: 0,
    };
    for folder in folders {
        let root = Path::new(&folder.local_workspace_binding);
        let metadata = std::fs::symlink_metadata(root)
            .map_err(|_| environment_error("detection folder unavailable"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            index.skips.push(skip(
                &folder.folder_id,
                ".",
                "symlink_or_traversal_excluded",
            ));
            continue;
        }
        let root = match EvidenceRoot::open(root) {
            Ok(root) => root,
            Err(_) => {
                index.skips.push(skip(
                    &folder.folder_id,
                    ".",
                    "symlink_or_traversal_excluded",
                ));
                continue;
            }
        };
        walk(&root, "", &folder.folder_id, 0, &mut budget, &mut index)?;
    }
    Ok(index)
}
struct Budget {
    visited: usize,
    bytes: u64,
}
fn walk(
    root: &EvidenceRoot,
    directory: &str,
    folder: &str,
    depth: usize,
    budget: &mut Budget,
    index: &mut EvidenceIndex,
) -> Result<(), DaemonError> {
    if depth > 32 {
        index
            .skips
            .push(skip(folder, directory, "traversal_depth_excluded"));
        return Ok(());
    }
    if budget.visited >= MAX_FILES {
        index.skips.push(skip(
            folder,
            if directory.is_empty() { "." } else { directory },
            "file_count_limit",
        ));
        return Ok(());
    }
    let (directory_guard, directory_path) = match root.directory(directory) {
        Ok(view) => view,
        Err(_) => {
            index
                .skips
                .push(skip(folder, directory, "symlink_or_traversal_excluded"));
            return Ok(());
        }
    };
    let _directory_guard = directory_guard;
    let entries = match std::fs::read_dir(directory_path) {
        Ok(entries) => entries,
        Err(_) => {
            index
                .skips
                .push(skip(folder, directory, "directory_unreadable"));
            return Ok(());
        }
    };
    // Bound directory enumeration too, including directories, symlinks and rejected names.
    let mut names = BTreeMap::new();
    let remaining = MAX_FILES.saturating_sub(budget.visited);
    let mut enumerated = 0;
    for entry in entries.take(MAX_FILES.saturating_sub(budget.visited) + 1) {
        enumerated += 1;
        if enumerated > remaining {
            // Never import an arbitrary filesystem-order prefix of an oversized directory.
            budget.visited = MAX_FILES;
            index.skips.push(skip(
                folder,
                if directory.is_empty() { "." } else { directory },
                "file_count_limit",
            ));
            return Ok(());
        }
        let Ok(entry) = entry else {
            index
                .skips
                .push(skip(folder, directory, "entry_unreadable"));
            continue;
        };
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            index
                .skips
                .push(skip(folder, "[non-UTF8 path]", "unsupported_path_encoding"));
            continue;
        };
        names.insert(name, entry);
    }
    // Rejected names and read errors consume the enumeration budget too.
    budget.visited += enumerated.saturating_sub(names.len());
    // Inspect local manifests before descending into potentially large source trees.
    let mut entries: Vec<_> = names.into_iter().collect();
    entries.sort_by_key(|(name, entry)| {
        (
            entry.file_type().is_ok_and(|kind| kind.is_dir()),
            name.clone(),
        )
    });
    for (name, entry) in entries {
        if budget.visited >= MAX_FILES {
            index
                .skips
                .push(skip(folder, directory, "file_count_limit"));
            break;
        }
        budget.visited += 1;
        if !safe_metadata(&name) {
            index
                .skips
                .push(skip(folder, "[protected path]", "protected_metadata"));
            continue;
        }
        let path = if directory.is_empty() {
            name.clone()
        } else {
            format!("{directory}/{name}")
        };
        if relative_environment_path(&path).is_err() || path.len() > 1024 {
            index
                .skips
                .push(skip(folder, "[excluded path]", "traversal_excluded"));
            continue;
        }
        let metadata = match entry.path().symlink_metadata() {
            Ok(m) => m,
            Err(_) => {
                index.skips.push(skip(folder, &path, "entry_unreadable"));
                continue;
            }
        };
        if metadata.file_type().is_symlink() {
            index.skips.push(skip(folder, &path, "symlink_excluded"));
            continue;
        }
        if metadata.is_dir() {
            if [
                ".git",
                "node_modules",
                "target",
                ".next",
                ".cache",
                ".ssh",
                ".codex",
                ".claude",
                ".chariox",
            ]
            .contains(&name.as_str())
            {
                index
                    .skips
                    .push(skip(folder, &path, "generated_or_private_directory"));
            } else {
                walk(root, &path, folder, depth + 1, budget, index)?
            }
            continue;
        }
        if !metadata.is_file() {
            index.skips.push(skip(folder, &path, "non_regular_file"));
            continue;
        }
        if value_file(&path) {
            index.skips.push(skip(folder, &path, "protected_file"));
            continue;
        }
        if metadata.len() > MAX_FILE {
            index.skips.push(skip(folder, &path, "file_too_large"));
            index.files.push(EvidenceFile {
                folder_id: folder.into(),
                path,
                text: None,
                digest: None,
                bytes: None,
                protected: false,
            });
            continue;
        }
        if budget.bytes.saturating_add(metadata.len()) > MAX_TOTAL {
            index.skips.push(skip(folder, &path, "total_byte_limit"));
            continue;
        }
        let file = match root.file(&path) {
            Ok(file) => file,
            Err(_) => {
                index
                    .skips
                    .push(skip(folder, &path, "symlink_or_traversal_excluded"));
                continue;
            }
        };
        let remaining = (MAX_TOTAL - budget.bytes).min(MAX_FILE);
        let mut bytes = Zeroizing::new(Vec::new());
        if file.take(remaining + 1).read_to_end(&mut bytes).is_err() {
            index.skips.push(skip(folder, &path, "file_unreadable"));
            continue;
        }
        budget.bytes += bytes.len() as u64;
        if bytes.len() as u64 > remaining {
            index.skips.push(skip(folder, &path, "read_byte_limit"));
            continue;
        }
        let text = std::str::from_utf8(&bytes)
            .ok()
            .filter(|s| !s.contains('\0'));
        let Some(text) = text else {
            index
                .skips
                .push(skip(folder, &path, "unsupported_encoding"));
            index.files.push(EvidenceFile {
                folder_id: folder.into(),
                path,
                text: None,
                digest: None,
                bytes: None,
                protected: false,
            });
            continue;
        };
        // Any environment value is ambiguous, even without a credential-shaped key/literal.
        let protected = credential_content(text)
            || name.starts_with(".env")
            || credential_configuration_name(&name);
        let digest = (!protected).then(|| format!("{:x}", Sha256::digest(&bytes[..])));
        index.files.push(EvidenceFile {
            folder_id: folder.into(),
            path,
            text: Some(Zeroizing::new(text.into())),
            digest,
            bytes: (!protected).then_some(bytes.len() as u64),
            protected,
        });
    }
    Ok(())
}

// MP-11: Retain the source directory FD while enumerating. A rename/symlink race cannot
// change the directory behind the view or redirect a subsequent content read.
pub(super) struct EvidenceRoot {
    #[cfg(not(unix))]
    path: std::path::PathBuf,
    #[cfg(unix)]
    anchor: std::fs::File,
}
impl EvidenceRoot {
    pub(super) fn open(path: &Path) -> Result<Self, DaemonError> {
        #[cfg(unix)]
        {
            use std::os::fd::{AsRawFd, FromRawFd};
            use std::os::unix::ffi::OsStrExt;
            use std::os::unix::fs::OpenOptionsExt;
            if !path.is_absolute() {
                return Err(environment_error("evidence root must be absolute"));
            }
            let mut anchor = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open("/")
                .map_err(|_| environment_error("evidence root unavailable"))?;
            for component in path.components() {
                let std::path::Component::Normal(component) = component else {
                    continue;
                };
                let component = std::ffi::CString::new(component.as_bytes())
                    .map_err(|_| environment_error("invalid evidence root"))?;
                let fd = unsafe {
                    libc::openat(
                        anchor.as_raw_fd(),
                        component.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                };
                if fd < 0 {
                    return Err(environment_error("evidence root symlink or unavailable"));
                }
                anchor = unsafe { std::fs::File::from_raw_fd(fd) };
            }
            Ok(Self { anchor })
        }
        #[cfg(not(unix))]
        {
            Ok(Self { path: path.into() })
        }
    }
    #[cfg(unix)]
    fn open_relative(&self, relative: &str, directory: bool) -> Result<std::fs::File, DaemonError> {
        use std::os::fd::{AsRawFd, FromRawFd};
        if !relative.is_empty() {
            relative_environment_path(relative).map_err(environment_error)?;
        }
        let mut anchor = self
            .anchor
            .try_clone()
            .map_err(|_| environment_error("evidence anchor unavailable"))?;
        let parts: Vec<_> = relative.split('/').filter(|s| !s.is_empty()).collect();
        for (n, component) in parts.iter().enumerate() {
            let component = std::ffi::CString::new(*component)
                .map_err(|_| environment_error("invalid evidence path"))?;
            let flags = libc::O_RDONLY
                | libc::O_NOFOLLOW
                | libc::O_CLOEXEC
                | libc::O_NONBLOCK
                | if directory || n + 1 < parts.len() {
                    libc::O_DIRECTORY
                } else {
                    0
                };
            let fd = unsafe { libc::openat(anchor.as_raw_fd(), component.as_ptr(), flags) };
            if fd < 0 {
                return Err(environment_error("evidence symlink or unavailable path"));
            }
            anchor = unsafe { std::fs::File::from_raw_fd(fd) };
        }
        Ok(anchor)
    }
    pub(super) fn file(&self, relative: &str) -> Result<std::fs::File, DaemonError> {
        #[cfg(unix)]
        let file = self.open_relative(relative, false)?;
        #[cfg(not(unix))]
        let file = open_workspace_file(&self.path, relative)?;
        if !file
            .metadata()
            .map_err(|_| environment_error("evidence metadata unavailable"))?
            .is_file()
        {
            return Err(environment_error("evidence file must be regular"));
        }
        Ok(file)
    }
    pub(super) fn directory(
        &self,
        relative: &str,
    ) -> Result<(Option<std::fs::File>, std::path::PathBuf), DaemonError> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let file = self.open_relative(relative, true)?;
            #[cfg(target_os = "linux")]
            let prefix = "/proc/self/fd";
            #[cfg(not(target_os = "linux"))]
            let prefix = "/dev/fd";
            let path = std::path::Path::new(prefix).join(file.as_raw_fd().to_string());
            Ok((Some(file), path))
        }
        #[cfg(not(unix))]
        {
            let mut path = self.path.clone();
            if !relative.is_empty() {
                relative_environment_path(relative).map_err(environment_error)?;
                for part in relative.split('/') {
                    path.push(part);
                    if std::fs::symlink_metadata(&path)
                        .map_err(|_| environment_error("evidence directory unavailable"))?
                        .file_type()
                        .is_symlink()
                    {
                        return Err(environment_error("evidence symlink rejected"));
                    }
                }
            }
            Ok((None, path))
        }
    }
}
