use std::path::Path;

pub(crate) const WORKSPACE_LIVE_SYNC_FORCE_EXCLUDE_PATTERNS: &[&str] = &[
    ".git/**",
    ".arroba/**",
    ".arrobaignore",
    ".env*",
    ".codex/**",
    ".opencode/**",
    ".claude/**",
    ".cursor/**",
    "*.sock",
    "*.socket",
    ".tmp-arroba/**",
    ".tmp-live-workspace-live-sync-drill/**",
    ".tmp-live-remote-workspace-live-sync-drill/**",
    "history/**",
    "session-history/**",
    "operational-history/**",
    "operational-history*",
    "node_modules/**",
    "target/**",
    ".cache/**",
    ".turbo/**",
    ".next/**",
    "dist/**",
    "build/**",
    ".venv/**",
    "venv/**",
    "__pycache__/**",
    ".pytest_cache/**",
    ".mypy_cache/**",
    ".ruff_cache/**",
    ".gradle/**",
    ".m2/**",
    ".pnpm-store/**",
];

const WORKSPACE_LIVE_SYNC_FORCE_EXCLUDE_DIRS: &[&str] = &[
    ".codex",
    ".opencode",
    ".claude",
    ".cursor",
    ".tmp-arroba",
    ".tmp-live-workspace-live-sync-drill",
    ".tmp-live-remote-workspace-live-sync-drill",
    "history",
    "session-history",
    "operational-history",
    "node_modules",
    "target",
    ".cache",
    ".turbo",
    ".next",
    "dist",
    "build",
    ".venv",
    "venv",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".gradle",
    ".m2",
    ".pnpm-store",
];

pub(crate) fn workspace_live_sync_force_exclude_patterns() -> Vec<String> {
    WORKSPACE_LIVE_SYNC_FORCE_EXCLUDE_PATTERNS
        .iter()
        .map(|pattern| (*pattern).to_string())
        .collect()
}

pub(crate) fn workspace_live_sync_user_ignore_patterns(worktree_path: &Path) -> Vec<String> {
    if !worktree_path.exists() {
        return Vec::new();
    }
    let ignore_path = worktree_path.join(".arrobaignore");
    if !ignore_path.exists() {
        let seed = match std::fs::read_to_string(worktree_path.join(".gitignore")) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(_) => String::new(),
        };
        let _ = std::fs::write(&ignore_path, seed);
    }
    std::fs::read_to_string(&ignore_path)
        .unwrap_or_default()
        .lines()
        .filter_map(workspace_live_sync_normalize_ignore_pattern)
        .collect()
}

pub(crate) fn workspace_live_sync_force_excluded_path(path: &str) -> bool {
    if path == ".arrobaignore"
        || path == ".git"
        || path.starts_with(".git/")
        || path == ".arroba"
        || path.starts_with(".arroba/")
    {
        return true;
    }
    if path.split('/').any(|part| part.starts_with(".env")) {
        return true;
    }
    if path.split('/').any(|part| {
        part.ends_with(".sock")
            || part.ends_with(".socket")
            || part.starts_with("operational-history")
    }) {
        return true;
    }
    path.split('/').any(|part| {
        WORKSPACE_LIVE_SYNC_FORCE_EXCLUDE_DIRS
            .iter()
            .any(|excluded| part == *excluded)
    })
}

fn workspace_live_sync_normalize_ignore_pattern(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('!') {
        return None;
    }
    let directory = trimmed.ends_with('/');
    let mut pattern = trimmed
        .trim_start_matches('/')
        .trim_end_matches('/')
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>()
        .join("/");
    if pattern.is_empty() {
        return None;
    }
    if directory {
        pattern.push_str("/**");
    }
    Some(pattern)
}
