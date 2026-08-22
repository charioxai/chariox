use super::*;

pub(super) fn prepare_repository(
    repository: &DevelopmentRepositoryManifest,
    artifacts_root: &Path,
    destination: &Path,
    remaining_project_checkout_bytes: u64,
    remaining_project_materialized_entries: u64,
) -> Result<RepositoryMaterializationEstimate, DaemonError> {
    let bundle = artifacts_root.join(&repository.bundle_path);
    let bundle_text = utf8_path(&bundle, "Git bundle")?;
    let destination_text = utf8_path(destination, "repository destination")?;
    let staging_parent = destination
        .parent()
        .ok_or_else(|| context_error("repository destination has no parent"))?;
    git_output_isolated(
        staging_parent,
        &[
            "clone",
            "--quiet",
            "--no-hardlinks",
            "--no-checkout",
            bundle_text,
            destination_text,
        ],
    )?;
    verify_git_bundle_isolated(destination, &bundle, &repository.head_sha)?;
    let estimate = inspect_import_repository(
        destination,
        remaining_project_checkout_bytes,
        remaining_project_materialized_entries,
    )?;
    let estimate = charge_overlay_materialization(
        estimate,
        &repository.overlay,
        remaining_project_checkout_bytes.min(MAX_CHECKOUT_BYTES_PER_REPOSITORY),
        remaining_project_materialized_entries.min(MAX_MATERIALIZED_ENTRIES_PER_REPOSITORY),
    )?;
    if git_text_isolated(destination, &["rev-parse", "HEAD"])? != repository.head_sha {
        return Err(context_error(
            "materialized repository HEAD does not match the manifest",
        ));
    }
    Ok(estimate)
}

pub(super) fn materialize_prepared_repository(
    repository: &DevelopmentRepositoryManifest,
    artifacts_root: &Path,
    destination: &Path,
) -> Result<(), DaemonError> {
    if let Some(branch) = &repository.branch {
        git_output_isolated(destination, &["check-ref-format", "--branch", branch])?;
        git_output_isolated(
            destination,
            &["checkout", "--quiet", "-B", branch, &repository.head_sha],
        )?;
    } else {
        git_output_isolated(
            destination,
            &["checkout", "--quiet", "--detach", &repository.head_sha],
        )?;
    }
    for entry in &repository.overlay {
        git_output_isolated(
            destination,
            &[
                "rm",
                "-r",
                "--cached",
                "--quiet",
                "--force",
                "--ignore-unmatch",
                "--",
                &entry.path,
            ],
        )?;
    }
    for entry in &repository.overlay {
        if let DevelopmentFileState::File {
            object_path,
            executable,
            ..
        } = &entry.index
        {
            let object = artifacts_root.join(object_path);
            let object_text = utf8_path(&object, "overlay object")?;
            let object_id = git_text_isolated(destination, &["hash-object", "-w", object_text])?;
            validate_git_oid(&object_id)?;
            let cacheinfo = format!(
                "{},{object_id},{}",
                if *executable { "100755" } else { "100644" },
                entry.path
            );
            git_output_isolated(
                destination,
                &["update-index", "--add", "--cacheinfo", &cacheinfo],
            )?;
        }
    }
    let mut paths = repository.overlay.iter().collect::<Vec<_>>();
    paths.sort_by_key(|entry| std::cmp::Reverse(entry.path.matches('/').count()));
    for entry in &paths {
        remove_materialized_path(destination, &entry.path)?;
    }
    paths.sort_by_key(|entry| entry.path.matches('/').count());
    for entry in paths {
        if let DevelopmentFileState::File {
            object_path,
            executable,
            ..
        } = &entry.worktree
        {
            write_materialized_file(
                destination,
                &entry.path,
                &artifacts_root.join(object_path),
                *executable,
            )?;
        }
    }
    if let Some(origin) = &repository.origin_url {
        git_output_isolated(destination, &["remote", "set-url", "origin", origin])?;
    } else {
        git_output_isolated(destination, &["remote", "remove", "origin"])?;
    }
    restore_upstream(repository, destination)?;
    verify_materialized_repository(repository, artifacts_root, destination)?;
    Ok(())
}

fn restore_upstream(
    repository: &DevelopmentRepositoryManifest,
    destination: &Path,
) -> Result<(), DaemonError> {
    let Some(upstream) = &repository.upstream else {
        return Ok(());
    };
    let branch = repository
        .branch
        .as_deref()
        .ok_or_else(|| context_error("tracked upstream requires a local branch"))?;
    let remote_branch = upstream
        .strip_prefix("origin/")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| context_error("only origin upstream branches are portable"))?;
    git_output_isolated(
        destination,
        &["check-ref-format", "--branch", remote_branch],
    )?;
    let remote_key = format!("branch.{branch}.remote");
    let merge_key = format!("branch.{branch}.merge");
    let merge_ref = format!("refs/heads/{remote_branch}");
    git_output_isolated(destination, &["config", "--local", &remote_key, "origin"])?;
    git_output_isolated(destination, &["config", "--local", &merge_key, &merge_ref])?;
    Ok(())
}

fn remove_materialized_path(repository: &Path, relative: &str) -> Result<(), DaemonError> {
    let path = repository.join(relative);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            fs::remove_dir_all(&path)
                .map_err(|error| context_io_error("remove materialized directory", error))
        }
        Ok(_) => fs::remove_file(&path)
            .map_err(|error| context_io_error("remove materialized file", error)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(context_io_error("inspect materialized path", error)),
    }
}

fn write_materialized_file(
    repository: &Path,
    relative: &str,
    object: &Path,
    executable: bool,
) -> Result<(), DaemonError> {
    let destination = repository.join(relative);
    let parent = destination
        .parent()
        .ok_or_else(|| context_error("materialized file has no parent"))?;
    ensure_safe_materialized_parent(repository, parent)?;
    let mut source =
        File::open(object).map_err(|error| context_io_error("open overlay object", error))?;
    let mut target = private_create_new(&destination)?;
    io::copy(&mut source, &mut target)
        .map_err(|error| context_io_error("write materialized file", error))?;
    target
        .sync_all()
        .map_err(|error| context_io_error("sync materialized file", error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            &destination,
            fs::Permissions::from_mode(if executable { 0o700 } else { 0o600 }),
        )
        .map_err(|error| context_io_error("set materialized file mode", error))?;
    }
    Ok(())
}

fn ensure_safe_materialized_parent(repository: &Path, parent: &Path) -> Result<(), DaemonError> {
    let relative = parent
        .strip_prefix(repository)
        .map_err(|_| context_error("materialized path escaped repository"))?;
    let mut current = repository.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(context_error(
                "materialized path contains unsafe components",
            ));
        };
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(context_error(format!(
                    "materialized path ancestor `{}` is not a real directory",
                    current.display()
                )));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&current)
                    .map_err(|error| context_io_error("create materialized directory", error))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&current, fs::Permissions::from_mode(0o700)).map_err(
                        |error| context_io_error("secure materialized directory", error),
                    )?;
                }
            }
            Err(error) => return Err(context_io_error("inspect materialized directory", error)),
        }
    }
    Ok(())
}

fn verify_materialized_repository(
    repository: &DevelopmentRepositoryManifest,
    artifacts_root: &Path,
    destination: &Path,
) -> Result<(), DaemonError> {
    if git_text_isolated(destination, &["rev-parse", "HEAD"])? != repository.head_sha {
        return Err(context_error("materialized repository HEAD changed"));
    }
    for entry in &repository.overlay {
        verify_materialized_index_state(destination, &entry.path, &entry.index, artifacts_root)?;
        verify_materialized_worktree_state(
            destination,
            &entry.path,
            &entry.worktree,
            artifacts_root,
        )?;
    }
    Ok(())
}

fn verify_materialized_index_state(
    repository: &Path,
    path: &str,
    expected: &DevelopmentFileState,
    artifacts_root: &Path,
) -> Result<(), DaemonError> {
    let output = git_output_isolated(repository, &["ls-files", "--stage", "-z", "--", path])?;
    let records = split_nul(&output.stdout)?;
    match expected {
        DevelopmentFileState::Absent if records.is_empty() => Ok(()),
        DevelopmentFileState::File {
            sha256, executable, ..
        } if records.len() == 1 => {
            let (metadata, indexed_path) = records[0]
                .split_once('\t')
                .ok_or_else(|| context_error("materialized Git index metadata is malformed"))?;
            let fields = metadata.split_whitespace().collect::<Vec<_>>();
            if indexed_path != path
                || fields.len() != 3
                || fields[2] != "0"
                || (fields[0] == "100755") != *executable
            {
                return Err(context_error("materialized Git index state is incorrect"));
            }
            let bytes = git_bytes_isolated(repository, &["cat-file", "blob", fields[1]])?;
            if sha256_bytes(&bytes) != *sha256 {
                return Err(context_error(
                    "materialized Git index content failed verification",
                ));
            }
            let _ = artifacts_root;
            Ok(())
        }
        _ => Err(context_error("materialized Git index state is incorrect")),
    }
}

fn verify_materialized_worktree_state(
    repository: &Path,
    path: &str,
    expected: &DevelopmentFileState,
    _artifacts_root: &Path,
) -> Result<(), DaemonError> {
    let absolute = repository.join(path);
    match expected {
        DevelopmentFileState::Absent => match fs::symlink_metadata(&absolute) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Ok(_) => Err(context_error("materialized worktree path should be absent")),
            Err(error) => Err(context_io_error(
                "inspect materialized worktree path",
                error,
            )),
        },
        DevelopmentFileState::File {
            sha256,
            size_bytes,
            executable,
            ..
        } => {
            let (bytes, metadata) = super::overlay::read_regular_file_without_following_symlinks(
                &absolute,
                path,
                *size_bytes,
            )?;
            if bytes.len() as u64 != *size_bytes
                || sha256_bytes(&bytes) != *sha256
                || file_is_executable(&metadata) != *executable
            {
                return Err(context_error(
                    "materialized worktree file failed verification",
                ));
            }
            Ok(())
        }
    }
}

fn utf8_path<'a>(path: &'a Path, label: &str) -> Result<&'a str, DaemonError> {
    path.to_str()
        .ok_or_else(|| context_error(format!("{label} path is not valid UTF-8")))
}
