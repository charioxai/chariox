use super::*;

pub fn export_development_context(
    request: DevelopmentContextExportRequest,
) -> Result<DevelopmentContextExportResult, DaemonError> {
    validate_export_request(&request)?;
    let archive_parent = request.archive_path.parent().ok_or_else(|| {
        context_error("development context archive path must have a parent directory")
    })?;
    if request.archive_path.exists() {
        return Err(context_error(format!(
            "development context archive `{}` already exists",
            request.archive_path.display()
        )));
    }

    let resolved_archive_parent = resolve_future_path(archive_parent)?;
    let mut canonical_sources = BTreeSet::new();
    let mut selected_sources = Vec::with_capacity(request.repositories.len());
    for selection in &request.repositories {
        let canonical_worktree = fs::canonicalize(&selection.worktree_path)
            .map_err(|error| context_io_error("resolve source worktree", error))?;
        if !canonical_sources.insert(canonical_worktree.clone()) {
            return Err(context_error(format!(
                "source worktree `{}` is selected more than once",
                canonical_worktree.display()
            )));
        }
        if resolved_archive_parent.starts_with(&canonical_worktree) {
            return Err(context_error(format!(
                "development context archive cannot be created inside source worktree `{}`",
                canonical_worktree.display()
            )));
        }
        selected_sources.push((selection, canonical_worktree));
    }

    fs::create_dir_all(archive_parent)
        .map_err(|error| context_io_error("create archive parent", error))?;
    let canonical_archive_parent = fs::canonicalize(archive_parent)
        .map_err(|error| context_io_error("resolve archive parent", error))?;
    if canonical_archive_parent != resolved_archive_parent {
        return Err(context_error(
            "development context archive parent changed while preparing the export",
        ));
    }

    let staging_root =
        create_unique_private_directory(&canonical_archive_parent, ".tmp-chariox-managed-context")?;
    let staging_name = staging_root
        .file_name()
        .ok_or_else(|| context_error("development context staging path has no file name"))?
        .to_string_lossy();
    let temporary_archive_path = canonical_archive_parent.join(format!("{staging_name}.archive"));
    let mut cleanup = ExportCleanup::new(staging_root.clone(), temporary_archive_path.clone());
    let temporary_archive = private_create_new(&temporary_archive_path)?;
    cleanup.own_archive();

    let mut repositories = Vec::with_capacity(request.repositories.len());
    let mut source_repositories = Vec::with_capacity(request.repositories.len());
    let mut repository_ids = BTreeSet::new();
    let mut target_directories = BTreeSet::new();
    let mut uncompressed_artifact_bytes = 0_u64;
    let mut checkout_bytes = 0_u64;
    let mut materialized_entries = 0_u64;
    let mut manifest_budget = ManifestMemoryBudget::new();
    manifest_budget.consume(request.project_id.len().saturating_add(256))?;
    for (selection, canonical_worktree) in selected_sources {
        let (exported, estimate) = export_repository(
            selection,
            &canonical_worktree,
            &staging_root,
            &mut repository_ids,
            &mut target_directories,
            &mut manifest_budget,
        )?;
        checkout_bytes = checkout_bytes.saturating_add(estimate.checkout_bytes);
        materialized_entries = materialized_entries.saturating_add(estimate.materialized_entries);
        if checkout_bytes > MAX_CHECKOUT_BYTES_PER_PROJECT
            || materialized_entries > MAX_MATERIALIZED_ENTRIES_PER_PROJECT
        {
            return Err(context_error(format!(
                "development context materialization exceeds the project budget of {MAX_CHECKOUT_BYTES_PER_PROJECT} bytes or {MAX_MATERIALIZED_ENTRIES_PER_PROJECT} entries"
            )));
        }
        source_repositories.push(DevelopmentSourceRepositoryMapping {
            source_workspace_id: selection.workspace_id.clone(),
            repository_id: exported.repository_id.clone(),
        });
        uncompressed_artifact_bytes = uncompressed_artifact_bytes
            .saturating_add(exported.bundle_size_bytes)
            .saturating_add(exported.overlay_size_bytes);
        if uncompressed_artifact_bytes > MAX_PACKAGE_BYTES {
            return Err(context_error(format!(
                "development context artifacts exceed {MAX_PACKAGE_BYTES} bytes"
            )));
        }
        repositories.push(exported);
    }

    let manifest = DevelopmentContextManifest {
        schema_version: DEVELOPMENT_CONTEXT_SCHEMA_VERSION,
        project_id: request.project_id,
        repositories,
    };
    write_archive(
        &temporary_archive_path,
        temporary_archive,
        &staging_root,
        &manifest,
    )?;
    let archive_size_bytes = fs::metadata(&temporary_archive_path)
        .map_err(|error| context_io_error("inspect development context archive", error))?
        .len();
    if archive_size_bytes > MAX_PACKAGE_BYTES {
        return Err(context_error(format!(
            "development context archive is {archive_size_bytes} bytes; maximum is {MAX_PACKAGE_BYTES}"
        )));
    }
    let archive_sha256 = sha256_file(&temporary_archive_path)?;
    cleanup.remove_staging()?;
    publish_archive_no_clobber(
        &temporary_archive_path,
        &request.archive_path,
        &canonical_archive_parent,
    )?;
    Ok(DevelopmentContextExportResult {
        manifest,
        archive_path: request.archive_path,
        archive_sha256,
        archive_size_bytes,
        source_repositories,
    })
}

pub(super) fn publish_archive_no_clobber(
    temporary: &Path,
    destination: &Path,
    destination_parent: &Path,
) -> Result<(), DaemonError> {
    fs::hard_link(temporary, destination).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            context_error(format!(
                "development context archive `{}` already exists",
                destination.display()
            ))
        } else {
            context_io_error("publish development context archive", error)
        }
    })?;
    #[cfg(unix)]
    if let Err(error) = File::open(destination_parent).and_then(|directory| directory.sync_all()) {
        let _ = fs::remove_file(destination);
        return Err(context_io_error(
            "sync development context archive directory",
            error,
        ));
    }
    Ok(())
}

fn resolve_future_path(path: &Path) -> Result<PathBuf, DaemonError> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(context_error(
            "development context archive path cannot contain parent-directory components",
        ));
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| context_io_error("resolve current directory", error))?
            .join(path)
    };
    let mut existing = absolute.as_path();
    let mut suffix = Vec::new();
    while !existing.exists() {
        let component = existing.file_name().ok_or_else(|| {
            context_error("development context archive parent has no existing ancestor")
        })?;
        suffix.push(component.to_os_string());
        existing = existing.parent().ok_or_else(|| {
            context_error("development context archive parent has no existing ancestor")
        })?;
    }
    let mut resolved = fs::canonicalize(existing)
        .map_err(|error| context_io_error("resolve archive ancestor", error))?;
    for component in suffix.iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

fn validate_export_request(request: &DevelopmentContextExportRequest) -> Result<(), DaemonError> {
    if request.project_id.trim().is_empty()
        || request.project_id.len() > MAX_CONTEXT_IDENTIFIER_BYTES
    {
        return Err(context_error("development context project id is required"));
    }
    if request.repositories.is_empty() || request.repositories.len() > MAX_REPOSITORIES {
        return Err(context_error(format!(
            "development context must contain between 1 and {MAX_REPOSITORIES} repositories"
        )));
    }
    if request
        .repositories
        .iter()
        .filter(|repository| repository.role == DevelopmentRepositoryRole::Primary)
        .count()
        != 1
    {
        return Err(context_error(
            "development context must contain exactly one primary repository",
        ));
    }
    if request.repositories.iter().any(|repository| {
        repository.workspace_id.trim().is_empty()
            || repository.workspace_id.len() > MAX_CONTEXT_IDENTIFIER_BYTES
    }) {
        return Err(context_error(
            "development context source Workspace identifiers are required",
        ));
    }
    Ok(())
}

fn export_repository(
    selection: &DevelopmentRepositorySelection,
    worktree: &Path,
    staging_root: &Path,
    repository_ids: &mut BTreeSet<String>,
    target_directories: &mut BTreeSet<String>,
    manifest_budget: &mut ManifestMemoryBudget,
) -> Result<
    (
        DevelopmentRepositoryManifest,
        RepositoryMaterializationEstimate,
    ),
    DaemonError,
> {
    ensure_worktree_root(worktree)?;
    let source_before = repository_source_state(worktree)?;
    let source_estimate = inspect_export_repository(worktree)?;
    let head_sha = source_before.head_sha.clone();
    let branch = source_before.branch.clone();
    let origin_url = source_before.origin_url.clone();
    let upstream = source_before.upstream.clone().filter(|upstream| {
        origin_url.is_some()
            && upstream
                .strip_prefix("origin/")
                .is_some_and(|branch| !branch.is_empty())
    });
    let logical_name = repository_logical_name(worktree, origin_url.as_deref());
    let repository_id = unique_repository_id(
        origin_url.as_deref(),
        &head_sha,
        &logical_name,
        repository_ids,
    );
    let target_directory =
        unique_target_directory(&logical_name, &repository_id, target_directories);
    manifest_budget.consume(
        head_sha
            .len()
            .saturating_add(branch.as_ref().map_or(0, String::len))
            .saturating_add(upstream.as_ref().map_or(0, String::len))
            .saturating_add(origin_url.as_ref().map_or(0, String::len))
            .saturating_add(logical_name.len())
            .saturating_add(repository_id.len())
            .saturating_add(target_directory.len())
            .saturating_add(2048),
    )?;
    let repository_root = staging_root.join("repositories").join(&repository_id);
    create_private_directory(&repository_root)?;
    let bundle_path = repository_root.join("repository.bundle");
    create_git_bundle(worktree, &bundle_path, MAX_BUNDLE_BYTES_PER_REPOSITORY)?;
    let bundle_size_bytes = fs::metadata(&bundle_path)
        .map_err(|error| context_io_error("inspect Git bundle", error))?
        .len();
    verify_git_bundle(worktree, &bundle_path, &head_sha)?;
    let bundle_sha256 = sha256_file(&bundle_path)?;
    let (overlay, overlay_size_bytes) =
        export_overlay(worktree, &repository_id, staging_root, manifest_budget)?;
    let source_estimate = charge_overlay_materialization(
        source_estimate,
        &overlay,
        MAX_CHECKOUT_BYTES_PER_REPOSITORY,
        MAX_MATERIALIZED_ENTRIES_PER_REPOSITORY,
    )?;
    let verification_root = repository_root.join("snapshot-verification");
    create_private_directory(&verification_root)?;
    let mut verification_budget = ManifestMemoryBudget::new();
    let (verified_overlay, verified_overlay_size_bytes) = export_overlay(
        worktree,
        &repository_id,
        &verification_root,
        &mut verification_budget,
    )?;
    fs::remove_dir_all(&verification_root)
        .map_err(|error| context_io_error("remove snapshot verification", error))?;
    let verified_estimate = charge_overlay_materialization(
        inspect_export_repository(worktree)?,
        &verified_overlay,
        MAX_CHECKOUT_BYTES_PER_REPOSITORY,
        MAX_MATERIALIZED_ENTRIES_PER_REPOSITORY,
    )?;
    let source_after = repository_source_state(worktree)?;
    if source_before != source_after
        || overlay != verified_overlay
        || overlay_size_bytes != verified_overlay_size_bytes
        || source_estimate != verified_estimate
    {
        return Err(context_error(format!(
            "source repository `{}` changed while exporting; retry the development context export",
            worktree.display()
        )));
    }

    Ok((
        DevelopmentRepositoryManifest {
            repository_id: repository_id.clone(),
            logical_name,
            role: selection.role,
            target_directory,
            head_sha,
            branch,
            upstream,
            origin_url,
            bundle_path: format!("repositories/{repository_id}/repository.bundle"),
            bundle_sha256,
            bundle_size_bytes,
            overlay,
            overlay_size_bytes,
        },
        source_estimate,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RepositorySourceState {
    head_sha: String,
    branch: Option<String>,
    upstream: Option<String>,
    origin_url: Option<String>,
}

fn repository_source_state(worktree: &Path) -> Result<RepositorySourceState, DaemonError> {
    Ok(RepositorySourceState {
        head_sha: git_text(worktree, &["rev-parse", "--verify", "HEAD"])?,
        branch: git_optional_text(worktree, &["symbolic-ref", "--short", "-q", "HEAD"])?,
        upstream: git_optional_text(
            worktree,
            &[
                "rev-parse",
                "--abbrev-ref",
                "--symbolic-full-name",
                "@{upstream}",
            ],
        )?,
        origin_url: git_optional_text(worktree, &["remote", "get-url", "origin"])?
            .and_then(|value| sanitize_origin_url(&value)),
    })
}

fn repository_logical_name(worktree: &Path, origin_url: Option<&str>) -> String {
    let candidate = origin_url
        .and_then(|origin| origin.trim_end_matches('/').rsplit('/').next())
        .map(|name| name.trim_end_matches(".git"))
        .filter(|name| !name.is_empty())
        .or_else(|| worktree.file_name().and_then(OsStr::to_str))
        .unwrap_or("repository");
    sanitize_directory_name(candidate)
}

fn unique_repository_id(
    origin_url: Option<&str>,
    head_sha: &str,
    logical_name: &str,
    occupied: &mut BTreeSet<String>,
) -> String {
    let base = sha256_bytes(
        format!(
            "{}\0{head_sha}\0{logical_name}",
            origin_url.unwrap_or_default()
        )
        .as_bytes(),
    );
    for suffix in 0_u32.. {
        let candidate = if suffix == 0 {
            format!("repo-{}", &base[..16])
        } else {
            format!("repo-{}-{suffix}", &base[..16])
        };
        if occupied.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("repository id suffix space is unbounded")
}

fn unique_target_directory(
    logical_name: &str,
    repository_id: &str,
    occupied: &mut BTreeSet<String>,
) -> String {
    let base = sanitize_directory_name(logical_name);
    if occupied.insert(base.to_ascii_lowercase()) {
        return base;
    }
    let hashed = format!("{base}-{}", &repository_id[5..13]);
    if occupied.insert(hashed.to_ascii_lowercase()) {
        return hashed;
    }
    for suffix in 2_u32.. {
        let candidate = format!("{hashed}-{suffix}");
        if occupied.insert(candidate.to_ascii_lowercase()) {
            return candidate;
        }
    }
    unreachable!("target directory suffix space is unbounded")
}

fn sanitize_directory_name(value: &str) -> String {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let normalized = normalized.trim_matches(['-', '.']).to_string();
    let normalized = if normalized.is_empty() || normalized == "." || normalized == ".." {
        "repository".to_string()
    } else {
        normalized
    };
    if normalized.len() <= MAX_TARGET_DIRECTORY_BASE_BYTES {
        normalized
    } else {
        let digest = sha256_bytes(normalized.as_bytes());
        let prefix_bytes = MAX_TARGET_DIRECTORY_BASE_BYTES - 17;
        format!("{}-{}", &normalized[..prefix_bytes], &digest[..16])
    }
}

pub(super) fn sanitize_origin_url(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.starts_with('/')
        || value.starts_with("./")
        || value.starts_with("../")
        || value.starts_with("file:")
    {
        return None;
    }
    if let Ok(mut url) = url::Url::parse(value) {
        if !matches!(url.scheme(), "http" | "https" | "ssh" | "git") {
            return None;
        }
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.set_query(None);
        url.set_fragment(None);
        return Some(url.to_string());
    }
    if let Some((user_host, path)) = value.split_once(':') {
        let host = user_host
            .rsplit_once('@')
            .map(|(_, host)| host)
            .unwrap_or(user_host);
        if !host.is_empty() && !path.is_empty() && !host.contains('/') {
            return Some(format!("{host}:{path}"));
        }
    }
    None
}

struct ExportCleanup {
    staging_root: PathBuf,
    archive_path: PathBuf,
    remove_archive: bool,
}

impl ExportCleanup {
    fn new(staging_root: PathBuf, archive_path: PathBuf) -> Self {
        Self {
            staging_root,
            archive_path,
            remove_archive: false,
        }
    }

    fn own_archive(&mut self) {
        self.remove_archive = true;
    }

    fn remove_staging(&mut self) -> Result<(), DaemonError> {
        fs::remove_dir_all(&self.staging_root)
            .map_err(|error| context_io_error("remove development context staging", error))?;
        self.staging_root = PathBuf::new();
        Ok(())
    }
}

impl Drop for ExportCleanup {
    fn drop(&mut self) {
        if !self.staging_root.as_os_str().is_empty() {
            let _ = fs::remove_dir_all(&self.staging_root);
        }
        if self.remove_archive {
            let _ = fs::remove_file(&self.archive_path);
        }
    }
}
