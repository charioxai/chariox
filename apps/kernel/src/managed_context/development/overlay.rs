use super::*;

pub(super) fn export_overlay(
    worktree: &Path,
    repository_id: &str,
    staging_root: &Path,
    manifest_budget: &mut ManifestMemoryBudget,
) -> Result<(Vec<DevelopmentOverlayEntry>, u64), DaemonError> {
    let mut paths = BTreeSet::new();
    for args in [
        &[
            "diff",
            "--name-only",
            "--no-renames",
            "-z",
            "--cached",
            "--",
        ][..],
        &["diff", "--name-only", "--no-renames", "-z", "--"][..],
        &["ls-files", "--others", "--exclude-standard", "-z", "--"][..],
    ] {
        stream_git_nul_records(worktree, args, MAX_OVERLAY_FILES_PER_REPOSITORY, |path| {
            if paths.insert(path.clone()) {
                manifest_budget.consume(path.len().saturating_add(256))?;
            }
            if paths.len() > MAX_OVERLAY_FILES_PER_REPOSITORY {
                return Err(context_error(format!(
                    "repository `{}` has more than {MAX_OVERLAY_FILES_PER_REPOSITORY} dirty paths",
                    worktree.display()
                )));
            }
            Ok(())
        })?;
    }
    if paths.len() > MAX_OVERLAY_FILES_PER_REPOSITORY {
        return Err(context_error(format!(
            "repository `{}` has {} dirty paths; maximum is {MAX_OVERLAY_FILES_PER_REPOSITORY}",
            worktree.display(),
            paths.len()
        )));
    }
    let ignore_patterns = read_context_ignore_patterns(worktree)?;
    let mut stored_objects = BTreeSet::new();
    let mut overlay_size_bytes = 0_u64;
    let mut entries = Vec::new();
    for path in paths {
        validate_relative_path(&path)?;
        if context_force_excluded_path(&path)
            || user_ignore_pattern_matches_any(&ignore_patterns, &path)
        {
            continue;
        }
        let index = index_file_state(
            worktree,
            repository_id,
            &path,
            staging_root,
            &mut stored_objects,
            &mut overlay_size_bytes,
        )?;
        let worktree_state = worktree_file_state(
            worktree,
            repository_id,
            &path,
            staging_root,
            &mut stored_objects,
            &mut overlay_size_bytes,
        )?;
        let entry = DevelopmentOverlayEntry {
            path,
            index,
            worktree: worktree_state,
        };
        manifest_budget.consume(file_state_manifest_bytes(&entry.index))?;
        manifest_budget.consume(file_state_manifest_bytes(&entry.worktree))?;
        entries.push(entry);
    }
    Ok((entries, overlay_size_bytes))
}

fn index_file_state(
    worktree: &Path,
    repository_id: &str,
    path: &str,
    staging_root: &Path,
    stored_objects: &mut BTreeSet<String>,
    overlay_size_bytes: &mut u64,
) -> Result<DevelopmentFileState, DaemonError> {
    let output = git_output(worktree, &["ls-files", "--stage", "-z", "--", path])?;
    let records = split_nul(&output.stdout)?;
    if records.is_empty() {
        return Ok(DevelopmentFileState::Absent);
    }
    if records.len() != 1 {
        return Err(context_error(format!(
            "dirty path `{path}` has unresolved Git index stages"
        )));
    }
    let Some((metadata, indexed_path)) = records[0].split_once('\t') else {
        return Err(context_error(format!(
            "dirty path `{path}` has malformed Git index metadata"
        )));
    };
    if indexed_path != path {
        return Err(context_error(format!(
            "Git index returned unexpected path `{indexed_path}` for `{path}`"
        )));
    }
    let fields = metadata.split_whitespace().collect::<Vec<_>>();
    if fields.len() != 3 || fields[2] != "0" {
        return Err(context_error(format!(
            "dirty path `{path}` has unsupported Git index metadata"
        )));
    }
    let mode = fields[0];
    if mode == "120000" || mode == "160000" {
        return Err(context_error(format!(
            "dirty path `{path}` is a symlink or submodule; dirty symlinks and submodules are not supported"
        )));
    }
    let blob_size = git_blob_size(worktree, fields[1])?;
    if blob_size > MAX_OVERLAY_FILE_BYTES {
        return Err(context_error(format!(
            "dirty path `{path}` is {blob_size} bytes; maximum is {MAX_OVERLAY_FILE_BYTES}"
        )));
    }
    let bytes = git_bytes(worktree, &["cat-file", "blob", fields[1]])?;
    validate_overlay_file_bytes(path, &bytes)?;
    store_overlay_object(
        repository_id,
        bytes,
        mode == "100755",
        staging_root,
        stored_objects,
        overlay_size_bytes,
    )
}

fn worktree_file_state(
    worktree: &Path,
    repository_id: &str,
    path: &str,
    staging_root: &Path,
    stored_objects: &mut BTreeSet<String>,
    overlay_size_bytes: &mut u64,
) -> Result<DevelopmentFileState, DaemonError> {
    reject_symlink_ancestors(worktree, path)?;
    let absolute = worktree.join(path);
    let metadata = match fs::symlink_metadata(&absolute) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(DevelopmentFileState::Absent)
        }
        Err(error) => return Err(context_io_error("inspect dirty worktree path", error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(context_error(format!(
            "dirty path `{path}` must be a regular file; symlinks and special files are not supported"
        )));
    }
    if metadata.len() > MAX_OVERLAY_FILE_BYTES {
        return Err(context_error(format!(
            "dirty path `{path}` is {} bytes; maximum is {MAX_OVERLAY_FILE_BYTES}",
            metadata.len()
        )));
    }
    let canonical = fs::canonicalize(&absolute)
        .map_err(|error| context_io_error("resolve dirty worktree path", error))?;
    if !canonical.starts_with(worktree) {
        return Err(context_error(format!(
            "dirty path `{path}` resolves outside the source worktree"
        )));
    }
    let (bytes, opened_metadata) =
        read_regular_file_without_following_symlinks(&absolute, path, MAX_OVERLAY_FILE_BYTES)?;
    validate_overlay_file_bytes(path, &bytes)?;
    let executable = file_is_executable(&opened_metadata);
    store_overlay_object(
        repository_id,
        bytes,
        executable,
        staging_root,
        stored_objects,
        overlay_size_bytes,
    )
}

fn reject_symlink_ancestors(worktree: &Path, path: &str) -> Result<(), DaemonError> {
    let mut current = worktree.to_path_buf();
    let components = Path::new(path).components().collect::<Vec<_>>();
    for component in components.iter().take(components.len().saturating_sub(1)) {
        let Component::Normal(component) = component else {
            return Err(context_error(format!("unsafe repository path `{path}`")));
        };
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(context_error(format!(
                    "dirty path `{path}` has symlink ancestor `{}`; dirty paths through symlinks are not supported",
                    current.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => break,
            Err(error) => return Err(context_io_error("inspect dirty path ancestor", error)),
        }
    }
    Ok(())
}

fn store_overlay_object(
    repository_id: &str,
    bytes: Vec<u8>,
    executable: bool,
    staging_root: &Path,
    stored_objects: &mut BTreeSet<String>,
    overlay_size_bytes: &mut u64,
) -> Result<DevelopmentFileState, DaemonError> {
    let size_bytes = bytes.len() as u64;
    if size_bytes > MAX_OVERLAY_FILE_BYTES {
        return Err(context_error(format!(
            "dirty file is {size_bytes} bytes; maximum is {MAX_OVERLAY_FILE_BYTES}"
        )));
    }
    let sha256 = sha256_bytes(&bytes);
    let object_path = format!("repositories/{repository_id}/objects/{sha256}");
    if stored_objects.insert(sha256.clone()) {
        *overlay_size_bytes = overlay_size_bytes.saturating_add(size_bytes);
        if *overlay_size_bytes > MAX_OVERLAY_BYTES_PER_REPOSITORY {
            return Err(context_error(format!(
                "repository dirty overlay exceeds {MAX_OVERLAY_BYTES_PER_REPOSITORY} bytes"
            )));
        }
        let destination = staging_root.join(&object_path);
        if let Some(parent) = destination.parent() {
            create_private_directory(parent)?;
        }
        write_private_file(&destination, &bytes)?;
    }
    Ok(DevelopmentFileState::File {
        object_path,
        sha256,
        size_bytes,
        executable,
    })
}

fn validate_overlay_file_bytes(path: &str, bytes: &[u8]) -> Result<(), DaemonError> {
    reject_lfs_pointer(bytes)?;
    if path == ".gitattributes" || path.ends_with("/.gitattributes") {
        reject_lfs_attributes(path, bytes)?;
    }
    Ok(())
}

pub(super) fn read_regular_file_without_following_symlinks(
    path: &Path,
    display_path: &str,
    maximum_bytes: u64,
) -> Result<(Vec<u8>, fs::Metadata), DaemonError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(path)
        .map_err(|error| context_io_error("open dirty worktree path", error))?;
    let metadata = file
        .metadata()
        .map_err(|error| context_io_error("inspect opened dirty worktree path", error))?;
    if !metadata.is_file() || metadata.len() > maximum_bytes {
        return Err(context_error(format!(
            "dirty path `{display_path}` must remain a bounded regular file while exporting"
        )));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    (&mut file)
        .take(maximum_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| context_io_error("read dirty worktree path", error))?;
    if bytes.len() as u64 > maximum_bytes {
        return Err(context_error(format!(
            "dirty path `{display_path}` grew beyond {maximum_bytes} bytes while exporting"
        )));
    }
    Ok((bytes, metadata))
}

pub(super) fn validate_relative_path(path: &str) -> Result<(), DaemonError> {
    if !crate::managed_context::portable_path::is_portable_relative_path(path) {
        return Err(context_error(format!("unsafe repository path `{path}`")));
    }
    Ok(())
}

pub(super) fn context_force_excluded_path(path: &str) -> bool {
    if path.split('/').any(is_portable_git_admin_component) {
        return true;
    }
    if path.split('/').any(|part| part.starts_with(".env")) {
        return true;
    }
    path.split('/').any(|part| {
        matches!(
            part,
            ".codex"
                | ".opencode"
                | ".claude"
                | ".cursor"
                | "node_modules"
                | "target"
                | ".cache"
                | ".turbo"
                | ".next"
                | "dist"
                | "build"
                | ".venv"
                | "venv"
                | "__pycache__"
                | ".pytest_cache"
                | ".mypy_cache"
                | ".ruff_cache"
                | ".gradle"
                | ".m2"
                | ".pnpm-store"
        ) || part.ends_with(".sock")
            || part.ends_with(".socket")
            || part.starts_with("operational-history")
    })
}

fn is_portable_git_admin_component(component: &str) -> bool {
    let folded = component
        .trim_end_matches([' ', '.'])
        .chars()
        .filter(|character| {
            !matches!(
                *character,
                '\u{200b}'..='\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2060}'..='\u{206f}'
                    | '\u{feff}'
            )
        })
        .collect::<String>()
        .to_ascii_lowercase();
    if folded == ".git" {
        return true;
    }
    let short = folded.strip_prefix('.').unwrap_or(&folded);
    short.strip_prefix("git~").is_some_and(|suffix| {
        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn read_context_ignore_patterns(worktree: &Path) -> Result<Vec<String>, DaemonError> {
    let path = worktree.join(".charioxignore");
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(context_io_error("inspect .charioxignore", error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(context_error(
            ".charioxignore must be a regular file and cannot be a symlink",
        ));
    }
    if metadata.len() > MAX_CONTEXT_IGNORE_BYTES {
        return Err(context_error(format!(
            ".charioxignore is {} bytes; maximum is {MAX_CONTEXT_IGNORE_BYTES}",
            metadata.len()
        )));
    }
    let (bytes, _) = read_regular_file_without_following_symlinks(
        &path,
        ".charioxignore",
        MAX_CONTEXT_IGNORE_BYTES,
    )?;
    if bytes.len() as u64 > MAX_CONTEXT_IGNORE_BYTES {
        return Err(context_error(format!(
            ".charioxignore grew beyond {MAX_CONTEXT_IGNORE_BYTES} bytes while exporting"
        )));
    }
    let contents = String::from_utf8(bytes)
        .map_err(|_| context_error(".charioxignore must contain valid UTF-8"))?;
    let patterns = contents
        .lines()
        .filter_map(normalize_ignore_pattern)
        .collect::<Vec<_>>();
    if patterns.len() > MAX_CONTEXT_IGNORE_PATTERNS {
        return Err(context_error(format!(
            ".charioxignore contains more than {MAX_CONTEXT_IGNORE_PATTERNS} patterns"
        )));
    }
    if let Some(pattern) = patterns
        .iter()
        .find(|pattern| pattern.len() > MAX_CONTEXT_IGNORE_PATTERN_BYTES)
    {
        return Err(context_error(format!(
            ".charioxignore pattern is {} bytes; maximum is {MAX_CONTEXT_IGNORE_PATTERN_BYTES}",
            pattern.len()
        )));
    }
    Ok(patterns)
}

fn file_state_manifest_bytes(state: &DevelopmentFileState) -> usize {
    match state {
        DevelopmentFileState::Absent => 32,
        DevelopmentFileState::File {
            object_path,
            sha256,
            ..
        } => object_path
            .len()
            .saturating_add(sha256.len())
            .saturating_add(192),
    }
}

fn normalize_ignore_pattern(line: &str) -> Option<String> {
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

fn user_ignore_pattern_matches_any(patterns: &[String], path: &str) -> bool {
    patterns.iter().any(|pattern| {
        if pattern.contains('/') {
            wildcard_match(pattern, path)
                || path
                    .strip_prefix(pattern.trim_end_matches("/**"))
                    .is_some_and(|suffix| suffix.starts_with('/'))
        } else {
            path.split('/').any(|part| wildcard_match(pattern, part))
        }
    })
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    let value = value.chars().collect::<Vec<_>>();
    let mut matches = vec![false; value.len() + 1];
    matches[0] = true;
    for character in pattern.chars() {
        if character == '*' {
            for index in 1..=value.len() {
                matches[index] = matches[index] || matches[index - 1];
            }
        } else {
            for index in (1..=value.len()).rev() {
                matches[index] = matches[index - 1] && value[index - 1] == character;
            }
            matches[0] = false;
        }
    }
    matches[value.len()]
}
