use std::collections::{BTreeMap, HashMap};
use std::ffi::OsStr;
use std::io::Read;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use sha2::{Digest, Sha256};

use crate::error::DaemonError;
use crate::local::{
    LocalDaemonResponse, WorkspaceFileContent, WorkspaceRepoFileEntry, WorkspaceRepoFileListing,
};
use crate::runtime::workspace_git_changes::workspace_git_file_changes;
use crate::runtime::workspace_git_common::{
    detect_git_branch, resolve_repo_root, workspace_default_compare_ref,
};

pub(crate) fn list_workspace_repo_files(
    workspace_id: &str,
    worktree_id: &str,
    path_prefix: Option<&str>,
    requested_compare_ref: Option<&str>,
    limit: Option<u32>,
) -> Result<WorkspaceRepoFileListing, DaemonError> {
    let worktree_path = worktree_id.trim();
    if worktree_path.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "list workspace files",
            message: "worktree_id is required".to_string(),
        });
    }
    let repo_root = resolve_repo_root(worktree_path)?;
    let repo_root_string = repo_root.display().to_string();
    let prefix = normalize_repo_file_prefix(path_prefix.unwrap_or_default());
    let branch = detect_git_branch(worktree_path).ok();
    let compare_ref = requested_compare_ref
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| workspace_default_compare_ref(&repo_root_string, branch.as_deref()));
    let change_map = workspace_git_file_changes(worktree_path, &compare_ref)?
        .into_iter()
        .map(|file| (file.path.clone(), file))
        .collect::<HashMap<_, _>>();
    let mut paths = workspace_git_tracked_files(worktree_path)?;
    paths.extend(change_map.keys().cloned());
    paths.sort();
    paths.dedup();

    let mut entries = BTreeMap::<String, WorkspaceRepoFileEntry>::new();
    for path in paths {
        let normalized_path = normalize_repo_file_prefix(&path);
        if normalized_path.is_empty() {
            continue;
        }
        let Some(remainder) = repo_file_remainder_for_prefix(&normalized_path, &prefix) else {
            continue;
        };
        if remainder.is_empty() {
            continue;
        }
        let mut parts = remainder.split('/');
        let Some(name) = parts.next().filter(|value| !value.is_empty()) else {
            continue;
        };
        let is_directory = parts.next().is_some();
        let entry_path = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };
        let change = change_map.get(&normalized_path);
        let entry = entries
            .entry(entry_path.clone())
            .or_insert_with(|| WorkspaceRepoFileEntry {
                path: entry_path.clone(),
                name: name.to_string(),
                kind: if is_directory { "directory" } else { "file" }.to_string(),
                changed: false,
                status: None,
                additions: 0,
                deletions: 0,
            });
        if is_directory {
            entry.kind = "directory".to_string();
        }
        if let Some(change) = change {
            entry.changed = true;
            entry.additions = entry.additions.saturating_add(change.additions);
            entry.deletions = entry.deletions.saturating_add(change.deletions);
            if !is_directory {
                entry.status = Some(change.status.clone());
            } else if entry.status.is_none() {
                entry.status = Some("changed".to_string());
            }
        }
    }
    let mut entries = entries.into_values().collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        let left_rank = if left.kind == "directory" { 0 } else { 1 };
        let right_rank = if right.kind == "directory" { 0 } else { 1 };
        left_rank
            .cmp(&right_rank)
            .then_with(|| left.name.cmp(&right.name))
    });
    let limit = limit.unwrap_or(400).clamp(1, 1000) as usize;
    let total_entries = entries.len();
    let truncated = total_entries > limit;
    entries.truncate(limit);
    Ok(WorkspaceRepoFileListing {
        workspace_id: workspace_id.to_string(),
        worktree_id: worktree_id.to_string(),
        path_prefix: prefix,
        compare_ref,
        total_entries: total_entries.min(u32::MAX as usize) as u32,
        truncated,
        entries,
        generated_at_ms: current_unix_ms(),
    })
}

pub(crate) fn get_workspace_file_content(
    workspace_id: &str,
    worktree_id: &str,
    path: &str,
    requested_compare_ref: Option<&str>,
    known_fingerprint: Option<&str>,
    max_bytes: Option<u32>,
) -> Result<LocalDaemonResponse, DaemonError> {
    let worktree_path = worktree_id.trim();
    if worktree_path.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "read workspace file",
            message: "worktree_id is required".to_string(),
        });
    }
    let normalized_path = normalize_workspace_file_content_path(path)?;
    let repo_root = resolve_repo_root(worktree_path)?;
    let repo_root_canonical =
        std::fs::canonicalize(&repo_root).map_err(|error| DaemonError::LocalTransport {
            operation: "read workspace file",
            message: format!(
                "failed to resolve repo root `{}`: {error}",
                repo_root.display()
            ),
        })?;
    let full_path = repo_root_canonical.join(&normalized_path);
    let full_path_canonical =
        std::fs::canonicalize(&full_path).map_err(|error| DaemonError::LocalTransport {
            operation: "read workspace file",
            message: format!("file `{normalized_path}` is not readable: {error}"),
        })?;
    if !full_path_canonical.starts_with(&repo_root_canonical) {
        return Err(DaemonError::LocalTransport {
            operation: "read workspace file",
            message: "file path escapes the repository root".to_string(),
        });
    }
    if full_path_canonical.is_dir() {
        return Err(DaemonError::LocalTransport {
            operation: "read workspace file",
            message: format!("`{normalized_path}` is a directory"),
        });
    }
    let metadata =
        std::fs::metadata(&full_path_canonical).map_err(|error| DaemonError::LocalTransport {
            operation: "read workspace file",
            message: format!("file `{normalized_path}` is not readable: {error}"),
        })?;
    let size_bytes = metadata.len();
    let mtime_ms = metadata
        .modified()
        .ok()
        .and_then(|mtime| mtime.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0);
    let max_bytes = max_bytes.unwrap_or(250_000).clamp(1, 1_000_000) as usize;
    let mut file =
        std::fs::File::open(&full_path_canonical).map_err(|error| DaemonError::LocalTransport {
            operation: "read workspace file",
            message: format!("file `{normalized_path}` is not readable: {error}"),
        })?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "read workspace file",
            message: format!("file `{normalized_path}` is not readable: {error}"),
        })?;
    let truncated = bytes.len() > max_bytes;
    if truncated {
        bytes.truncate(max_bytes);
    }
    let sha256 = workspace_hex_digest(&Sha256::digest(&bytes));
    let fingerprint = workspace_file_fingerprint(&normalized_path, size_bytes, mtime_ms, &sha256);
    if known_fingerprint
        .map(str::trim)
        .filter(|value| !value.is_empty())
        == Some(fingerprint.as_str())
    {
        return Ok(LocalDaemonResponse::WorkspaceFileContentNotModified {
            workspace_id: workspace_id.to_string(),
            worktree_id: worktree_id.to_string(),
            path: normalized_path,
            fingerprint,
            generated_at_ms: current_unix_ms(),
        });
    }
    let binary = bytes.contains(&0);
    let (encoding, content_text, content_base64) = if binary {
        (
            "base64".to_string(),
            None,
            Some(STANDARD.encode(bytes.as_slice())),
        )
    } else {
        match String::from_utf8(bytes.clone()) {
            Ok(text) => ("utf-8".to_string(), Some(text), None),
            Err(_) => (
                "base64".to_string(),
                None,
                Some(STANDARD.encode(bytes.as_slice())),
            ),
        }
    };
    let branch = detect_git_branch(worktree_path).ok();
    let repo_root_string = repo_root.display().to_string();
    let compare_ref = requested_compare_ref
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| workspace_default_compare_ref(&repo_root_string, branch.as_deref()));
    let change = workspace_git_file_changes(worktree_path, &compare_ref)?
        .into_iter()
        .find(|candidate| candidate.path == normalized_path);
    let language = workspace_file_language(&normalized_path).to_string();
    let mime = workspace_file_mime(&language).to_string();
    let name = normalized_path
        .rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or(normalized_path.as_str())
        .to_string();
    Ok(LocalDaemonResponse::WorkspaceFileContent {
        content: WorkspaceFileContent {
            workspace_id: workspace_id.to_string(),
            worktree_id: worktree_id.to_string(),
            path: normalized_path,
            name,
            language,
            mime,
            encoding,
            content_text,
            content_base64,
            size_bytes,
            mtime_ms,
            fingerprint,
            sha256: Some(sha256),
            truncated,
            status: change.as_ref().map(|file| file.status.clone()),
            additions: change.as_ref().map(|file| file.additions).unwrap_or(0),
            deletions: change.as_ref().map(|file| file.deletions).unwrap_or(0),
            compare_ref,
            generated_at_ms: current_unix_ms(),
        },
    })
}

fn workspace_git_tracked_files(worktree_path: &str) -> Result<Vec<String>, DaemonError> {
    let output = std::process::Command::new("git")
        .args(["ls-files", "-z", "-c", "-o", "--exclude-standard"])
        .current_dir(worktree_path)
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "workspace repo files",
            message: error.to_string(),
        })?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter_map(|part| {
            let value = String::from_utf8_lossy(part).trim().to_string();
            (!value.is_empty()).then_some(value)
        })
        .collect())
}

fn normalize_workspace_file_content_path(path: &str) -> Result<String, DaemonError> {
    let raw_path = path.trim();
    if raw_path.is_empty() || Path::new(raw_path).is_absolute() {
        return Err(DaemonError::LocalTransport {
            operation: "read workspace file",
            message: "workspace file path must be repo-relative".to_string(),
        });
    }
    let parts = raw_path
        .trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>();
    if parts.is_empty() || parts.iter().any(|part| *part == "..") {
        return Err(DaemonError::LocalTransport {
            operation: "read workspace file",
            message: "workspace file path must stay inside the repository".to_string(),
        });
    }
    Ok(parts.join("/"))
}

fn normalize_repo_file_prefix(path: &str) -> String {
    path.trim()
        .trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>()
        .join("/")
}

fn repo_file_remainder_for_prefix<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_empty() {
        return Some(path);
    }
    if path == prefix {
        return Some("");
    }
    path.strip_prefix(prefix)
        .and_then(|value| value.strip_prefix('/'))
}

fn workspace_file_fingerprint(path: &str, size_bytes: u64, mtime_ms: u64, sha256: &str) -> String {
    let payload = format!("{path}:{size_bytes}:{mtime_ms}:{sha256}");
    workspace_hex_digest(&Sha256::digest(payload.as_bytes()))
}

fn workspace_hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn workspace_file_language(path: &str) -> &'static str {
    let extension = Path::new(path)
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "rs" => "rust",
        "ts" => "typescript",
        "tsx" => "typescriptreact",
        "js" => "javascript",
        "jsx" => "javascriptreact",
        "py" => "python",
        "json" => "json",
        "md" | "markdown" => "markdown",
        "swift" => "swift",
        "go" => "go",
        "java" => "java",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => "cpp",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" | "sass" => "scss",
        "yml" | "yaml" => "yaml",
        "toml" => "toml",
        "sh" | "bash" | "zsh" => "shell",
        "sql" => "sql",
        "xml" => "xml",
        "txt" | "text" => "text",
        _ => "text",
    }
}

fn workspace_file_mime(language: &str) -> &'static str {
    match language {
        "rust" => "text/x-rust",
        "typescript" | "typescriptreact" => "text/typescript",
        "javascript" | "javascriptreact" => "text/javascript",
        "python" => "text/x-python",
        "json" => "application/json",
        "markdown" => "text/markdown",
        "html" => "text/html",
        "css" | "scss" => "text/css",
        "yaml" => "application/x-yaml",
        "toml" => "application/toml",
        "xml" => "application/xml",
        _ => "text/plain",
    }
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_repo_file_prefix, normalize_workspace_file_content_path,
        repo_file_remainder_for_prefix, workspace_file_fingerprint, workspace_file_language,
        workspace_file_mime,
    };

    #[test]
    fn workspace_file_content_paths_are_repo_relative() {
        assert_eq!(
            normalize_workspace_file_content_path(" src/./main.rs/ ")
                .expect("path should normalize"),
            "src/main.rs"
        );
        assert!(normalize_workspace_file_content_path("").is_err());
        assert!(normalize_workspace_file_content_path("/tmp/file").is_err());
        assert!(normalize_workspace_file_content_path("../secrets").is_err());
        assert!(normalize_workspace_file_content_path("src/../secrets").is_err());
    }

    #[test]
    fn repo_file_prefixes_normalize_for_listing_projection() {
        assert_eq!(normalize_repo_file_prefix(" /src//./app/ "), "src/app");
        assert_eq!(normalize_repo_file_prefix("./"), "");
        assert_eq!(
            repo_file_remainder_for_prefix("src/app/main.rs", "src"),
            Some("app/main.rs")
        );
        assert_eq!(repo_file_remainder_for_prefix("src", "src"), Some(""));
        assert_eq!(
            repo_file_remainder_for_prefix("src-other/main.rs", "src"),
            None
        );
    }

    #[test]
    fn file_language_and_mime_follow_workspace_display_policy() {
        assert_eq!(workspace_file_language("src/app.tsx"), "typescriptreact");
        assert_eq!(workspace_file_mime("typescriptreact"), "text/typescript");
        assert_eq!(workspace_file_language("README.md"), "markdown");
        assert_eq!(workspace_file_mime("markdown"), "text/markdown");
        assert_eq!(workspace_file_language("unknown.bin"), "text");
        assert_eq!(workspace_file_mime("text"), "text/plain");
    }

    #[test]
    fn workspace_file_fingerprint_depends_on_metadata_and_digest() {
        let first = workspace_file_fingerprint("src/lib.rs", 10, 20, "abc");
        let second = workspace_file_fingerprint("src/lib.rs", 10, 21, "abc");
        assert_ne!(first, second);
        assert_eq!(first.len(), 64);
    }
}
