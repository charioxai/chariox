use super::*;

pub(super) fn deduplicate_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    paths
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

pub(super) fn discover_codex_external_sessions(root: &Path) -> Vec<ExternalProviderSessionRecord> {
    codex_candidate_paths(root)
        .into_iter()
        .filter_map(|path| parse_codex_transcript(&path))
        .collect()
}

pub(super) fn discover_claude_external_sessions(root: &Path) -> Vec<ExternalProviderSessionRecord> {
    claude_candidate_paths(root)
        .into_iter()
        .filter_map(|path| parse_claude_transcript(&path))
        .collect()
}

pub(super) fn discover_opencode_external_sessions(
    root: &Path,
) -> Vec<ExternalProviderSessionRecord> {
    let mut sessions = discover_opencode_sqlite_sessions(root);
    sessions.extend(
        opencode_candidate_paths(root)
            .into_iter()
            .filter_map(|path| parse_opencode_session_file(&path)),
    );
    sessions
}

pub(super) fn provider_session_candidate_paths(
    provider_filter: Option<&str>,
) -> Vec<(String, PathBuf)> {
    let mut paths = Vec::new();
    if provider_matches(provider_filter, "codex") {
        for root in codex_roots() {
            paths.extend(
                codex_candidate_paths(&root)
                    .into_iter()
                    .map(|path| ("codex".to_string(), path)),
            );
        }
    }
    if provider_matches(provider_filter, "claude") {
        for root in claude_roots() {
            paths.extend(
                claude_candidate_paths(&root)
                    .into_iter()
                    .map(|path| ("claude".to_string(), path)),
            );
        }
    }
    if provider_matches(provider_filter, "opencode") {
        for root in opencode_roots() {
            paths.extend(
                opencode_candidate_paths(&root)
                    .into_iter()
                    .map(|path| ("opencode".to_string(), path)),
            );
        }
    }
    paths
}

pub(super) fn codex_candidate_paths(root: &Path) -> Vec<PathBuf> {
    let mut candidates = jsonl_candidates(&root.join("archived_sessions"), 4);
    candidates.extend(jsonl_candidates(&root.join("sessions"), 4));
    sort_file_candidates_by_recent_modified(&mut candidates);
    candidates.truncate(MAX_PROVIDER_FILES);
    candidates
}

pub(super) fn claude_candidate_paths(root: &Path) -> Vec<PathBuf> {
    let mut candidates = jsonl_candidates(&root.join("projects"), 3);
    candidates.truncate(MAX_PROVIDER_FILES);
    candidates
}

pub(super) fn opencode_candidate_paths(root: &Path) -> Vec<PathBuf> {
    let mut candidates = session_json_candidates(root, 5);
    candidates.extend(opencode_sqlite_signature_paths(root));
    sort_file_candidates_by_recent_modified(&mut candidates);
    candidates.truncate(MAX_PROVIDER_FILES);
    candidates
}

pub(super) fn opencode_sqlite_db_path(root: &Path) -> PathBuf {
    root.join("opencode.db")
}

pub(super) fn opencode_sqlite_signature_paths(root: &Path) -> Vec<PathBuf> {
    let db = opencode_sqlite_db_path(root);
    let wal = root.join("opencode.db-wal");
    [db, wal]
        .into_iter()
        .filter(|path| path.is_file())
        .collect()
}

pub(super) fn jsonl_candidates(root: &Path, max_depth: usize) -> Vec<PathBuf> {
    file_candidates(root, max_depth, &["jsonl"])
}

pub(super) fn session_json_candidates(root: &Path, max_depth: usize) -> Vec<PathBuf> {
    file_candidates(root, max_depth, &["json", "jsonl"])
        .into_iter()
        .filter(|path| {
            let lower = path.display().to_string().to_ascii_lowercase();
            lower.contains("session")
                || lower.contains("conversation")
                || lower.contains("message")
                || lower.ends_with(".jsonl")
        })
        .collect()
}

pub(super) fn file_candidates(root: &Path, max_depth: usize, extensions: &[&str]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_file_candidates(root, max_depth, extensions, &mut files);
    sort_file_candidates_by_recent_modified(&mut files);
    files
}

pub(super) fn sort_file_candidates_by_recent_modified(files: &mut [PathBuf]) {
    let modified_by_path = files
        .iter()
        .map(|path| (path.clone(), file_modified_ms(path)))
        .collect::<BTreeMap<_, _>>();
    files.sort_by(|left, right| {
        modified_by_path
            .get(right)
            .copied()
            .unwrap_or(0)
            .cmp(&modified_by_path.get(left).copied().unwrap_or(0))
            .then_with(|| left.cmp(right))
    });
}

pub(super) fn collect_file_candidates(
    root: &Path,
    depth_remaining: usize,
    extensions: &[&str],
    files: &mut Vec<PathBuf>,
) {
    if depth_remaining == 0 {
        return;
    }
    #[cfg(test)]
    increment_file_candidate_scan_count();
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .components()
            .any(|component| component.as_os_str() == "node_modules")
        {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            collect_file_candidates(&path, depth_remaining - 1, extensions, files);
        } else if file_type.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extensions.iter().any(|allowed| *allowed == extension))
        {
            files.push(path);
        }
    }
}
