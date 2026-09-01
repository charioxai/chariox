use super::*;

pub(super) fn ensure_worktree_root(worktree: &Path) -> Result<(), DaemonError> {
    let metadata = fs::symlink_metadata(worktree)
        .map_err(|error| context_io_error("inspect source worktree", error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(context_error(format!(
            "source worktree `{}` must be a real directory",
            worktree.display()
        )));
    }
    let root = PathBuf::from(git_text(worktree, &["rev-parse", "--show-toplevel"])?);
    let root = fs::canonicalize(root)
        .map_err(|error| context_io_error("resolve Git worktree root", error))?;
    if root != worktree {
        return Err(context_error(format!(
            "source path `{}` is not the Git worktree root",
            worktree.display()
        )));
    }
    Ok(())
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(super) struct RepositoryMaterializationEstimate {
    pub checkout_bytes: u64,
    pub materialized_entries: u64,
}

pub(super) fn charge_overlay_materialization(
    mut estimate: RepositoryMaterializationEstimate,
    overlay: &[DevelopmentOverlayEntry],
    maximum_checkout_bytes: u64,
    maximum_materialized_entries: u64,
) -> Result<RepositoryMaterializationEstimate, DaemonError> {
    for entry in overlay {
        if let DevelopmentFileState::File { size_bytes, .. } = &entry.index {
            charge_materialized_state(&mut estimate, *size_bytes, 3);
        }
        if let DevelopmentFileState::File { size_bytes, .. } = &entry.worktree {
            let entries = 1_u64
                .saturating_add(entry.path.bytes().filter(|byte| *byte == b'/').count() as u64);
            charge_materialized_state(&mut estimate, *size_bytes, entries);
        }
        validate_materialization_estimate(
            estimate,
            maximum_checkout_bytes,
            maximum_materialized_entries,
            "repository overlay",
        )?;
    }
    Ok(estimate)
}

fn charge_materialized_state(
    estimate: &mut RepositoryMaterializationEstimate,
    content_bytes: u64,
    entries: u64,
) {
    estimate.checkout_bytes = estimate
        .checkout_bytes
        .saturating_add(content_bytes)
        .saturating_add(entries.saturating_mul(4096));
    estimate.materialized_entries = estimate.materialized_entries.saturating_add(entries);
}

fn validate_materialization_estimate(
    estimate: RepositoryMaterializationEstimate,
    maximum_checkout_bytes: u64,
    maximum_materialized_entries: u64,
    label: &str,
) -> Result<(), DaemonError> {
    if estimate.checkout_bytes > maximum_checkout_bytes {
        return Err(context_error(format!(
            "{label} exceeds the {maximum_checkout_bytes}-byte checkout budget"
        )));
    }
    if estimate.materialized_entries > maximum_materialized_entries {
        return Err(context_error(format!(
            "{label} exceeds the {maximum_materialized_entries}-entry materialization budget"
        )));
    }
    Ok(())
}

pub(super) fn inspect_export_repository(
    worktree: &Path,
) -> Result<RepositoryMaterializationEstimate, DaemonError> {
    inspect_repository_features(
        worktree,
        false,
        MAX_CHECKOUT_BYTES_PER_REPOSITORY,
        MAX_MATERIALIZED_ENTRIES_PER_REPOSITORY,
    )
}

pub(super) fn inspect_import_repository(
    worktree: &Path,
    maximum_checkout_bytes: u64,
    maximum_materialized_entries: u64,
) -> Result<RepositoryMaterializationEstimate, DaemonError> {
    inspect_repository_features(
        worktree,
        true,
        maximum_checkout_bytes.min(MAX_CHECKOUT_BYTES_PER_REPOSITORY),
        maximum_materialized_entries.min(MAX_MATERIALIZED_ENTRIES_PER_REPOSITORY),
    )
}

fn inspect_repository_features(
    worktree: &Path,
    isolated: bool,
    maximum_checkout_bytes: u64,
    maximum_materialized_entries: u64,
) -> Result<RepositoryMaterializationEstimate, DaemonError> {
    if git_text_with_environment(
        worktree,
        &["rev-parse", "--is-shallow-repository"],
        isolated,
    )? == "true"
    {
        return Err(context_error(format!(
            "repository `{}` is shallow; shallow repositories are not supported in development context version 1",
            worktree.display()
        )));
    }
    let mut checkout_bytes = 4096_u64;
    let mut materialized_entries = 1_u64;
    validate_materialization_estimate(
        RepositoryMaterializationEstimate {
            checkout_bytes,
            materialized_entries,
        },
        maximum_checkout_bytes,
        maximum_materialized_entries,
        "repository root",
    )?;
    stream_git_nul_records_with_environment(
        worktree,
        &["ls-tree", "-r", "-l", "-z", "HEAD"],
        MAX_REPOSITORY_TREE_ENTRIES,
        isolated,
        |record| {
            let Some((metadata, path)) = record.split_once('\t') else {
                return Err(context_error("Git tree returned a malformed entry"));
            };
            let fields = metadata.split_whitespace().collect::<Vec<_>>();
            if fields.len() != 4 {
                return Err(context_error("Git tree returned malformed metadata"));
            }
            let mode = fields[0];
            let object_id = fields[2];
            let size = if fields[3] == "-" {
                0
            } else {
                fields[3]
                    .parse::<u64>()
                    .map_err(|_| context_error("Git tree returned an invalid blob size"))?
            };
            validate_relative_path(path)?;
            materialized_entries = materialized_entries
                .saturating_add(1)
                .saturating_add(path.bytes().filter(|byte| *byte == b'/').count() as u64);
            if materialized_entries > maximum_materialized_entries {
                return Err(context_error(format!(
                    "repository `{}` current tree exceeds the {maximum_materialized_entries}-entry materialization budget",
                    worktree.display()
                )));
            }
            match mode {
                "160000" => {
                    return Err(context_error(format!(
                        "repository `{}` contains submodule `{path}`; submodules are not supported in development context version 1",
                        worktree.display()
                    )));
                }
                "120000" => validate_committed_symlink(worktree, path, object_id, isolated)?,
                _ => {}
            }
            let materialized_size = if mode == "100644" || mode == "100755" {
                size.saturating_mul(2)
            } else {
                size
            };
            let allocation_bytes = 4096_u64.saturating_mul(
                1_u64.saturating_add(path.bytes().filter(|byte| *byte == b'/').count() as u64),
            );
            checkout_bytes = checkout_bytes
                .saturating_add(materialized_size)
                .saturating_add(allocation_bytes);
            if checkout_bytes > maximum_checkout_bytes {
                return Err(context_error(format!(
                    "repository `{}` current tree exceeds the {maximum_checkout_bytes}-byte checkout budget",
                    worktree.display()
                )));
            }
            if path == ".gitattributes" || path.ends_with("/.gitattributes") {
                if size > MAX_CONTEXT_IGNORE_BYTES {
                    return Err(context_error(format!(
                        "repository attributes `{path}` are {size} bytes; maximum is {MAX_CONTEXT_IGNORE_BYTES}"
                    )));
                }
                let contents = git_bytes_with_environment(
                    worktree,
                    &["cat-file", "blob", object_id],
                    isolated,
                )?;
                reject_lfs_attributes(path, &contents)?;
                reject_checkout_transform_attributes(path, &contents)?;
            }
            Ok(())
        },
    )?;
    let lfs = git_output_allow_status_with_environment(
        worktree,
        &[
            "grep",
            "-I",
            "-q",
            "version https://git-lfs.github.com/spec/v1",
            "HEAD",
            "--",
            ".",
        ],
        &[0, 1],
        isolated,
    )?;
    if lfs.status.code() == Some(0) {
        return Err(context_error(format!(
            "repository `{}` contains Git LFS pointers; Git LFS is not supported in development context version 1",
            worktree.display()
        )));
    }
    Ok(RepositoryMaterializationEstimate {
        checkout_bytes,
        materialized_entries,
    })
}

fn reject_checkout_transform_attributes(path: &str, bytes: &[u8]) -> Result<(), DaemonError> {
    for line in String::from_utf8_lossy(bytes).lines() {
        let line = line.trim_start();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        for attribute in line.split_whitespace().skip(1) {
            let attribute = attribute.trim_start_matches(['-', '!']);
            if attribute == "ident"
                || attribute.starts_with("ident=")
                || attribute == "working-tree-encoding"
                || attribute.starts_with("working-tree-encoding=")
            {
                return Err(context_error(format!(
                    "repository attributes `{path}` enable checkout-transforming attribute `{attribute}`; ident and working-tree-encoding are not supported in development context version 1"
                )));
            }
        }
    }
    Ok(())
}

fn validate_committed_symlink(
    worktree: &Path,
    path: &str,
    object_id: &str,
    isolated: bool,
) -> Result<(), DaemonError> {
    let size = git_blob_size_with_environment(worktree, object_id, isolated)?;
    if size > MAX_GIT_NUL_RECORD_BYTES as u64 {
        return Err(context_error(format!(
            "committed symlink `{path}` target exceeds {MAX_GIT_NUL_RECORD_BYTES} bytes"
        )));
    }
    let target = git_bytes_with_environment(worktree, &["cat-file", "blob", object_id], isolated)?;
    let target = String::from_utf8(target)
        .map_err(|_| context_error(format!("committed symlink `{path}` has a non-UTF-8 target")))?;
    if !symlink_target_stays_within_repository(path, target.trim()) {
        return Err(context_error(format!(
            "committed symlink `{path}` points outside the repository"
        )));
    }
    Ok(())
}

pub(super) fn reject_lfs_attributes(path: &str, bytes: &[u8]) -> Result<(), DaemonError> {
    if String::from_utf8_lossy(bytes)
        .lines()
        .any(|line| line.contains("filter=lfs"))
    {
        return Err(context_error(format!(
            "repository attributes `{path}` enable Git LFS; Git LFS is not supported in development context version 1"
        )));
    }
    Ok(())
}

pub(super) fn reject_lfs_pointer(bytes: &[u8]) -> Result<(), DaemonError> {
    if bytes.starts_with(b"version https://git-lfs.github.com/spec/v1\n") {
        return Err(context_error(
            "dirty overlay contains a Git LFS pointer; Git LFS is not supported in development context version 1",
        ));
    }
    Ok(())
}

fn symlink_target_stays_within_repository(path: &str, target: &str) -> bool {
    let target = Path::new(target);
    if target.is_absolute() || target.as_os_str().is_empty() {
        return false;
    }
    let mut depth = Path::new(path)
        .parent()
        .map(|parent| parent.components().count())
        .unwrap_or(0);
    for component in target.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(_) => depth += 1,
            Component::ParentDir if depth > 0 => depth -= 1,
            Component::ParentDir => return false,
            Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    true
}

pub(super) fn create_git_bundle(
    worktree: &Path,
    destination: &Path,
    maximum_bytes: u64,
) -> Result<(), DaemonError> {
    let mut cleanup = BundleCleanup::new(destination.to_path_buf());
    let mut file = private_create_new(destination)?;
    let mut child = Command::new("git")
        .args([OsStr::new("bundle"), OsStr::new("create"), OsStr::new("-")])
        .arg("HEAD")
        .current_dir(worktree)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| context_io_error("create Git bundle", error))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| context_error("create Git bundle did not provide stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| context_error("create Git bundle did not provide stderr"))?;
    let stderr_reader = std::thread::spawn(move || read_capped_and_drain(stderr));
    let mut written = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = match stdout.read(&mut buffer) {
            Ok(read) => read,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stderr_reader.join();
                return Err(context_io_error("read Git bundle", error));
            }
        };
        if read == 0 {
            break;
        }
        written = written.saturating_add(read as u64);
        if written > maximum_bytes {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stderr_reader.join();
            return Err(context_error(format!(
                "repository Git bundle exceeds {maximum_bytes} bytes"
            )));
        }
        if let Err(error) = file.write_all(&buffer[..read]) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stderr_reader.join();
            return Err(context_io_error("write Git bundle", error));
        }
    }
    let status = child
        .wait()
        .map_err(|error| context_io_error("wait for Git bundle", error))?;
    let stderr = stderr_reader.join().unwrap_or_default();
    if !status.success() {
        return Err(context_error(format!(
            "create Git bundle failed: {}",
            String::from_utf8_lossy(&stderr).trim()
        )));
    }
    file.sync_all()
        .map_err(|error| context_io_error("sync Git bundle", error))?;
    cleanup.keep();
    Ok(())
}

pub(super) fn verify_git_bundle(
    worktree: &Path,
    bundle: &Path,
    expected_head: &str,
) -> Result<(), DaemonError> {
    verify_git_bundle_with_environment(worktree, bundle, expected_head, false)
}

pub(super) fn verify_git_bundle_isolated(
    worktree: &Path,
    bundle: &Path,
    expected_head: &str,
) -> Result<(), DaemonError> {
    verify_git_bundle_with_environment(worktree, bundle, expected_head, true)
}

fn verify_git_bundle_with_environment(
    worktree: &Path,
    bundle: &Path,
    expected_head: &str,
    isolated: bool,
) -> Result<(), DaemonError> {
    let file = File::open(bundle).map_err(|error| context_io_error("open Git bundle", error))?;
    let mut reader = BufReader::new(file);
    let mut header_bytes = 0_usize;
    let mut header_records = 0_usize;
    let signature =
        read_bounded_bundle_header_line(&mut reader, &mut header_bytes, &mut header_records)?
            .unwrap_or_default();
    if !signature.starts_with("# v") || !signature.ends_with(" git bundle") {
        return Err(context_error("Git bundle has an invalid signature"));
    }
    while let Some(line) =
        read_bounded_bundle_header_line(&mut reader, &mut header_bytes, &mut header_records)?
    {
        if line.is_empty() {
            break;
        }
        if line.starts_with('-') {
            return Err(context_error(
                "Git bundle has prerequisites and is not self-contained",
            ));
        }
    }
    let bundle_text = bundle
        .to_str()
        .ok_or_else(|| context_error("Git bundle path is not valid UTF-8"))?;
    let heads = if isolated {
        git_text_isolated(worktree, &["bundle", "list-heads", bundle_text, "HEAD"])?
    } else {
        git_text(worktree, &["bundle", "list-heads", bundle_text, "HEAD"])?
    };
    if heads.split_whitespace().next() != Some(expected_head) {
        return Err(context_error(
            "Git bundle HEAD does not match the captured repository HEAD",
        ));
    }
    if isolated {
        git_output_isolated(worktree, &["bundle", "verify", bundle_text])?;
    } else {
        git_output(worktree, &["bundle", "verify", bundle_text])?;
    }
    Ok(())
}

pub(super) fn git_blob_size(worktree: &Path, object_id: &str) -> Result<u64, DaemonError> {
    git_blob_size_with_environment(worktree, object_id, false)
}

fn git_blob_size_with_environment(
    worktree: &Path,
    object_id: &str,
    isolated: bool,
) -> Result<u64, DaemonError> {
    git_text_with_environment(worktree, &["cat-file", "-s", object_id], isolated)?
        .parse::<u64>()
        .map_err(|_| context_error("Git returned an invalid blob size"))
}

struct BundleCleanup {
    path: PathBuf,
    remove: bool,
}

impl BundleCleanup {
    fn new(path: PathBuf) -> Self {
        Self { path, remove: true }
    }

    fn keep(&mut self) {
        self.remove = false;
    }
}

impl Drop for BundleCleanup {
    fn drop(&mut self) {
        if self.remove {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub(super) fn git_text(worktree: &Path, args: &[&str]) -> Result<String, DaemonError> {
    git_text_with_environment(worktree, args, false)
}

fn git_text_with_environment(
    worktree: &Path,
    args: &[&str],
    isolated: bool,
) -> Result<String, DaemonError> {
    let output = run_git_output_bounded(worktree, args, MAX_GIT_TEXT_BYTES, isolated)?;
    let output = require_git_success(output, "run Git text command")?;
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|_| context_error(format!("git {} returned non-UTF-8 output", args.join(" "))))
}

pub(super) fn git_optional_text(
    worktree: &Path,
    args: &[&str],
) -> Result<Option<String>, DaemonError> {
    let output = run_git_output_bounded(worktree, args, MAX_GIT_TEXT_BYTES, false)?;
    let output = require_allowed_git_status(output, &[0, 1, 2, 128])?;
    if !output.status.success() {
        return Ok(None);
    }
    let value = String::from_utf8(output.stdout)
        .map_err(|_| context_error(format!("git {} returned non-UTF-8 output", args.join(" "))))?
        .trim()
        .to_string();
    Ok((!value.is_empty()).then_some(value))
}

pub(super) fn git_bytes(worktree: &Path, args: &[&str]) -> Result<Vec<u8>, DaemonError> {
    git_bytes_with_environment(worktree, args, false)
}

fn git_bytes_with_environment(
    worktree: &Path,
    args: &[&str],
    isolated: bool,
) -> Result<Vec<u8>, DaemonError> {
    let output = run_git_output_bounded(worktree, args, MAX_GIT_COMMAND_OUTPUT_BYTES, isolated)?;
    Ok(require_git_success(output, "run Git bytes command")?.stdout)
}

pub(super) fn git_output(worktree: &Path, args: &[&str]) -> Result<Output, DaemonError> {
    let output = run_git_output_bounded(worktree, args, MAX_GIT_COMMAND_OUTPUT_BYTES, false)?;
    require_git_success(output, "run Git command")
}

pub(super) fn git_text_isolated(worktree: &Path, args: &[&str]) -> Result<String, DaemonError> {
    let output = run_git_output_bounded(worktree, args, MAX_GIT_TEXT_BYTES, true)?;
    let output = require_git_success(output, "run isolated Git text command")?;
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|_| context_error(format!("git {} returned non-UTF-8 output", args.join(" "))))
}

pub(super) fn git_output_isolated(worktree: &Path, args: &[&str]) -> Result<Output, DaemonError> {
    let output = run_git_output_bounded(worktree, args, MAX_GIT_COMMAND_OUTPUT_BYTES, true)?;
    require_git_success(output, "run isolated Git command")
}

pub(super) fn git_bytes_isolated(worktree: &Path, args: &[&str]) -> Result<Vec<u8>, DaemonError> {
    Ok(git_output_isolated(worktree, args)?.stdout)
}

fn git_output_allow_status_with_environment(
    worktree: &Path,
    args: &[&str],
    allowed: &[i32],
    isolated: bool,
) -> Result<Output, DaemonError> {
    let output = run_git_output_bounded(worktree, args, MAX_GIT_TEXT_BYTES, isolated)?;
    require_allowed_git_status(output, allowed)
}

fn require_allowed_git_status(output: Output, allowed: &[i32]) -> Result<Output, DaemonError> {
    if output
        .status
        .code()
        .is_some_and(|code| allowed.contains(&code))
    {
        Ok(output)
    } else {
        require_git_success(output, "run Git command")
    }
}

fn run_git_output_bounded(
    worktree: &Path,
    args: &[&str],
    maximum_stdout_bytes: usize,
    isolated: bool,
) -> Result<Output, DaemonError> {
    let mut command = Command::new("git");
    command.args(args).current_dir(worktree);
    if isolated {
        configure_isolated_git_environment(&mut command);
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| context_io_error("run bounded Git command", error))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| context_error("bounded Git command did not provide stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| context_error("bounded Git command did not provide stderr"))?;
    let stderr_reader = std::thread::spawn(move || read_capped_and_drain(stderr));
    let mut stdout_bytes = Vec::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = match stdout.read(&mut buffer) {
            Ok(read) => read,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stderr_reader.join();
                return Err(context_io_error("read bounded Git output", error));
            }
        };
        if read == 0 {
            break;
        }
        if stdout_bytes.len().saturating_add(read) > maximum_stdout_bytes {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stderr_reader.join();
            return Err(context_error(format!(
                "Git command output exceeds {maximum_stdout_bytes} bytes"
            )));
        }
        stdout_bytes.extend_from_slice(&buffer[..read]);
    }
    let status = child
        .wait()
        .map_err(|error| context_io_error("wait for bounded Git command", error))?;
    let stderr = stderr_reader.join().unwrap_or_default();
    Ok(Output {
        status,
        stdout: stdout_bytes,
        stderr,
    })
}

pub(super) fn configure_isolated_git_environment(command: &mut Command) {
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0");
    for name in [
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_OBJECT_DIRECTORY",
        "GIT_INDEX_FILE",
        "GIT_WORK_TREE",
        "GIT_DIR",
        "GIT_CEILING_DIRECTORIES",
        "GIT_SSH_COMMAND",
        "GIT_ASKPASS",
        "SSH_ASKPASS",
        "GIT_TEMPLATE_DIR",
    ] {
        command.env_remove(name);
    }
}

fn read_capped_and_drain(mut reader: impl Read) -> Vec<u8> {
    let mut retained = Vec::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let Ok(read) = reader.read(&mut buffer) else {
            break;
        };
        if read == 0 {
            break;
        }
        let remaining = MAX_GIT_ERROR_BYTES.saturating_sub(retained.len());
        retained.extend_from_slice(&buffer[..read.min(remaining)]);
    }
    retained
}

pub(super) fn stream_git_nul_records<F>(
    worktree: &Path,
    args: &[&str],
    maximum_records: usize,
    on_record: F,
) -> Result<(), DaemonError>
where
    F: FnMut(String) -> Result<(), DaemonError>,
{
    stream_git_nul_records_with_environment(worktree, args, maximum_records, false, on_record)
}

fn stream_git_nul_records_with_environment<F>(
    worktree: &Path,
    args: &[&str],
    maximum_records: usize,
    isolated: bool,
    mut on_record: F,
) -> Result<(), DaemonError>
where
    F: FnMut(String) -> Result<(), DaemonError>,
{
    let mut command = Command::new("git");
    command.args(args).current_dir(worktree);
    if isolated {
        configure_isolated_git_environment(&mut command);
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| context_io_error("run streaming Git command", error))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| context_error("streaming Git command did not provide stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| context_error("streaming Git command did not provide stderr"))?;
    let stderr_reader = std::thread::spawn(move || read_capped_and_drain(stderr));
    let mut records = 0_usize;
    let mut record = Vec::new();
    let mut buffer = [0_u8; 64 * 1024];
    let result = 'stream: loop {
        let read = match stdout.read(&mut buffer) {
            Ok(read) => read,
            Err(error) => break Err(context_io_error("read streaming Git output", error)),
        };
        if read == 0 {
            break if record.is_empty() {
                Ok(())
            } else {
                Err(context_error(
                    "streaming Git command returned an unterminated record",
                ))
            };
        }
        for byte in &buffer[..read] {
            if *byte == 0 {
                records = records.saturating_add(1);
                if records > maximum_records {
                    break 'stream Err(context_error(format!(
                        "Git command returned more than {maximum_records} records"
                    )));
                }
                let value = match String::from_utf8(std::mem::take(&mut record)) {
                    Ok(value) => value,
                    Err(_) => {
                        break 'stream Err(context_error(
                            "Git returned a non-UTF-8 repository path",
                        ));
                    }
                };
                if let Err(error) = on_record(value) {
                    break 'stream Err(error);
                }
            } else {
                record.push(*byte);
                if record.len() > MAX_GIT_NUL_RECORD_BYTES {
                    break 'stream Err(context_error(format!(
                        "Git command returned a record larger than {MAX_GIT_NUL_RECORD_BYTES} bytes"
                    )));
                }
            }
        }
    };
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child
        .wait()
        .map_err(|error| context_io_error("wait for streaming Git command", error))?;
    let stderr = stderr_reader.join().unwrap_or_default();
    result?;
    if !status.success() {
        return Err(context_error(format!(
            "run streaming Git command failed: {}",
            String::from_utf8_lossy(&stderr).trim()
        )));
    }
    Ok(())
}

fn read_bounded_bundle_header_line(
    reader: &mut impl BufRead,
    total_bytes: &mut usize,
    records: &mut usize,
) -> Result<Option<String>, DaemonError> {
    let mut line = Vec::new();
    loop {
        let available = reader
            .fill_buf()
            .map_err(|error| context_io_error("read Git bundle header", error))?;
        if available.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            return Err(context_error("Git bundle header has an unterminated line"));
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        *total_bytes = total_bytes.saturating_add(take);
        if *total_bytes > MAX_GIT_BUNDLE_HEADER_BYTES {
            return Err(context_error(format!(
                "Git bundle header exceeds {MAX_GIT_BUNDLE_HEADER_BYTES} bytes"
            )));
        }
        line.extend_from_slice(&available[..take]);
        let found_newline = line.last() == Some(&b'\n');
        reader.consume(take);
        if found_newline {
            *records = records.saturating_add(1);
            if *records > MAX_GIT_BUNDLE_HEADER_RECORDS {
                return Err(context_error(format!(
                    "Git bundle header exceeds {MAX_GIT_BUNDLE_HEADER_RECORDS} records"
                )));
            }
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return String::from_utf8(line)
                .map(Some)
                .map_err(|_| context_error("Git bundle header is not valid UTF-8"));
        }
    }
}

fn require_git_success(output: Output, operation: &'static str) -> Result<Output, DaemonError> {
    if output.status.success() {
        return Ok(output);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Err(context_error(format!(
        "{operation} failed: {}",
        if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            output.status.to_string()
        }
    )))
}

pub(super) fn split_nul(bytes: &[u8]) -> Result<Vec<String>, DaemonError> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .map(|record| {
            String::from_utf8(record.to_vec())
                .map_err(|_| context_error("Git returned a non-UTF-8 repository path"))
        })
        .collect()
}
