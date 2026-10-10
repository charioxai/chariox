use std::path::Path;

pub(crate) const WORKSPACE_LIVE_SYNC_FORCE_EXCLUDE_PATTERNS: &[&str] = &[
    ".git/**",
    ".chariox/**",
    "!.chariox/project.json",
    ".charioxignore",
    ".env*",
    ".codex/**",
    ".opencode/**",
    ".claude/**",
    ".cursor/**",
    "*.sock",
    "*.socket",
    ".tmp-chariox/**",
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
    ".tmp-chariox",
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
    let mut rules = Vec::new();
    for name in [".gitignore", ".charioxignore"] {
        rules.extend(
            std::fs::read_to_string(worktree_path.join(name))
                .unwrap_or_default()
                .lines()
                .filter_map(workspace_live_sync_normalize_ignore_pattern),
        );
        if name == ".gitignore" {
            rules.extend(crate::project_environment::project_private_file_rules(
                worktree_path,
            ));
        }
    }
    for name in [".worktreeinclude", ".chariox/worktreeinclude"] {
        for line in std::fs::read_to_string(worktree_path.join(name))
            .unwrap_or_default()
            .lines()
        {
            if let Some(rule) = workspace_live_sync_normalize_ignore_pattern(line) {
                rules.push(format!("!{}", rule.trim_start_matches('!')));
            }
        }
    }
    rules.extend(crate::project_environment::project_private_secret_files(
        worktree_path,
    ));
    rules
}

// MP-08 / MP-10 / MP-11: Explicit workspace source is distinct from runtime state.
pub(crate) fn workspace_source_capability_path(path: &str) -> bool {
    path == ".chariox/project.json"
}

pub(crate) fn workspace_live_sync_force_excluded_path(path: &str) -> bool {
    if path == ".charioxignore"
        || path == ".git"
        || path.starts_with(".git/")
        || path == ".chariox"
        || (path.starts_with(".chariox/") && !workspace_source_capability_path(path))
    {
        return true;
    }
    if path.split('/').any(|part| {
        part.starts_with(".env")
            || part.ends_with(".pem")
            || part.ends_with(".key")
            || part.ends_with(".tfvars")
            || part.starts_with("id_rsa")
            || part.starts_with("id_ed25519")
            || part.to_ascii_lowercase().contains("credential")
            || part == ".ssh"
    }) {
        return true;
    }
    if path.split('/').any(|part| {
        part.ends_with(".sock")
            || part.ends_with(".socket")
            || part.starts_with("operational-history")
    }) {
        return true;
    }
    path.split('/')
        .any(|part| WORKSPACE_LIVE_SYNC_FORCE_EXCLUDE_DIRS.contains(&part))
}

pub(crate) fn workspace_live_sync_ignored_path(path: &str, ignore_patterns: &[String]) -> bool {
    workspace_live_sync_force_excluded_path(path) || user_rules_exclude_path(path, ignore_patterns)
}

pub(crate) fn user_rules_exclude_path(path: &str, rules: &[String]) -> bool {
    let mut excluded = false;
    for rule in rules {
        let include = rule.starts_with('!');
        if workspace_live_sync_ignore_pattern_matches(rule.trim_start_matches('!'), path) {
            excluded = !include;
        }
    }
    excluded
}

fn workspace_live_sync_normalize_ignore_pattern(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let include = trimmed.starts_with('!');
    let trimmed = trimmed.trim_start_matches('!');
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
    if include {
        pattern.insert(0, '!');
    }
    Some(pattern)
}

fn workspace_live_sync_ignore_pattern_matches(pattern: &str, path: &str) -> bool {
    let directory_pattern = pattern.ends_with('/');
    let pattern = pattern.trim_end_matches('/');
    if pattern.is_empty() {
        return false;
    }
    if pattern.contains('/') {
        return workspace_live_sync_wildcard_match(pattern, path)
            || path
                .strip_prefix(pattern)
                .is_some_and(|suffix| suffix.starts_with('/'))
            || (directory_pattern && path == pattern);
    }
    path.split('/')
        .any(|part| workspace_live_sync_wildcard_match(pattern, part))
}

fn workspace_live_sync_wildcard_match(pattern: &str, value: &str) -> bool {
    if pattern == value {
        return true;
    }
    let Some((head, tail)) = pattern.split_once('*') else {
        return false;
    };
    if !value.starts_with(head) {
        return false;
    }
    let remainder = &value[head.len()..];
    if !tail.contains('*') {
        return tail.is_empty() || remainder.ends_with(tail);
    }
    remainder
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(remainder.len()))
        .any(|index| workspace_live_sync_wildcard_match(tail, &remainder[index..]))
}

#[cfg(test)]
mod mp08_private_overlay_tests {
    use super::*;
    #[test]
    fn mp08_optional_rules_layer_without_creating_a_repository_file() {
        let root = std::env::temp_dir().join(format!(
            "chariox-envlayer3-ignore-{}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join(".gitignore"), "private/\n*.local.md\n").unwrap();
        let rules = workspace_live_sync_user_ignore_patterns(&root);
        assert!(!root.join(".charioxignore").exists());
        assert!(workspace_live_sync_ignored_path(
            "private/notes.txt",
            &rules
        ));
        std::fs::write(
            root.join(".charioxignore"),
            "!CLAUDE.local.md\n!.env.local\n",
        )
        .unwrap();
        std::fs::write(root.join(".worktreeinclude"), "private/needed.json\n").unwrap();
        let rules = workspace_live_sync_user_ignore_patterns(&root);
        assert!(!workspace_live_sync_ignored_path("CLAUDE.local.md", &rules));
        assert!(!workspace_live_sync_ignored_path(
            "private/needed.json",
            &rules
        ));
        assert!(workspace_live_sync_ignored_path(".env.local", &rules));
        assert!(!workspace_live_sync_force_excluded_path(
            ".chariox/project.json"
        ));
        assert!(workspace_live_sync_force_excluded_path(
            ".chariox/runtime-mailbox.json"
        ));
        assert!(workspace_live_sync_force_excluded_path(
            "secrets/client.pem"
        ));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
