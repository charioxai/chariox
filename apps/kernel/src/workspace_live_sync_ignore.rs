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
