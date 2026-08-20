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

pub(super) fn reject_unsupported_repository_features(worktree: &Path) -> Result<(), DaemonError> {
    if git_text(worktree, &["rev-parse", "--is-shallow-repository"])? == "true" {
        return Err(context_error(format!(
            "repository `{}` is shallow; shallow repositories are not supported in development context version 1",
            worktree.display()
        )));
    }
    stream_git_nul_records(
        worktree,
        &["ls-tree", "-r", "-z", "HEAD"],
        MAX_REPOSITORY_TREE_ENTRIES,
        |record| {
            let Some((metadata, path)) = record.split_once('\t') else {
                return Err(context_error("Git tree returned a malformed entry"));
            };
            let fields = metadata.split_whitespace().collect::<Vec<_>>();
            if fields.len() != 3 {
                return Err(context_error("Git tree returned malformed metadata"));
            }
            let mode = fields[0];
            let object_id = fields[2];
            validate_relative_path(path)?;
            match mode {
                "160000" => {
                    return Err(context_error(format!(
                        "repository `{}` contains submodule `{path}`; submodules are not supported in development context version 1",
                        worktree.display()
                    )));
                }
                "120000" => validate_committed_symlink(worktree, path, object_id)?,
                _ => {}
            }
            if path == ".gitattributes" || path.ends_with("/.gitattributes") {
                let size = git_blob_size(worktree, object_id)?;
                if size > MAX_CONTEXT_IGNORE_BYTES {
                    return Err(context_error(format!(
                        "repository attributes `{path}` are {size} bytes; maximum is {MAX_CONTEXT_IGNORE_BYTES}"
                    )));
                }
                let contents = git_bytes(worktree, &["cat-file", "blob", object_id])?;
                reject_lfs_attributes(path, &contents)?;
            }
            Ok(())
        },
    )?;
    let lfs = git_output_allow_status(
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
    )?;
    if lfs.status.code() == Some(0) {
        return Err(context_error(format!(
            "repository `{}` contains Git LFS pointers; Git LFS is not supported in development context version 1",
            worktree.display()
        )));
    }
    Ok(())
}

fn validate_committed_symlink(
    worktree: &Path,
    path: &str,
    object_id: &str,
) -> Result<(), DaemonError> {
    let size = git_blob_size(worktree, object_id)?;
    if size > MAX_GIT_NUL_RECORD_BYTES as u64 {
        return Err(context_error(format!(
            "committed symlink `{path}` target exceeds {MAX_GIT_NUL_RECORD_BYTES} bytes"
        )));
    }
    let target = git_bytes(worktree, &["cat-file", "blob", object_id])?;
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
    let file = File::open(bundle).map_err(|error| context_io_error("open Git bundle", error))?;
    let mut lines = BufReader::new(file).lines();
    let signature = lines
        .next()
        .transpose()
        .map_err(|error| context_io_error("read Git bundle header", error))?
        .unwrap_or_default();
    if !signature.starts_with("# v") || !signature.ends_with(" git bundle") {
        return Err(context_error("Git bundle has an invalid signature"));
    }
    for line in lines {
        let line = line.map_err(|error| context_io_error("read Git bundle header", error))?;
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
    let heads = git_text(worktree, &["bundle", "list-heads", bundle_text, "HEAD"])?;
    if heads.split_whitespace().next() != Some(expected_head) {
        return Err(context_error(
            "Git bundle HEAD does not match the captured repository HEAD",
        ));
    }
    git_output(worktree, &["bundle", "verify", bundle_text])?;
    Ok(())
}

pub(super) fn git_blob_size(worktree: &Path, object_id: &str) -> Result<u64, DaemonError> {
    git_text(worktree, &["cat-file", "-s", object_id])?
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
    let output = run_git_output_bounded(worktree, args, MAX_GIT_TEXT_BYTES)?;
    let output = require_git_success(output, "run Git text command")?;
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|_| context_error(format!("git {} returned non-UTF-8 output", args.join(" "))))
}

pub(super) fn git_optional_text(
    worktree: &Path,
    args: &[&str],
) -> Result<Option<String>, DaemonError> {
    let output = run_git_output_bounded(worktree, args, MAX_GIT_TEXT_BYTES)?;
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
    Ok(git_output(worktree, args)?.stdout)
}

pub(super) fn git_output(worktree: &Path, args: &[&str]) -> Result<Output, DaemonError> {
    let output = run_git_output_bounded(worktree, args, MAX_GIT_COMMAND_OUTPUT_BYTES)?;
    require_git_success(output, "run Git command")
}

pub(super) fn git_output_allow_status(
    worktree: &Path,
    args: &[&str],
    allowed: &[i32],
) -> Result<Output, DaemonError> {
    let output = run_git_output_bounded(worktree, args, MAX_GIT_TEXT_BYTES)?;
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
) -> Result<Output, DaemonError> {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(worktree)
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
    mut on_record: F,
) -> Result<(), DaemonError>
where
    F: FnMut(String) -> Result<(), DaemonError>,
{
    let mut child = Command::new("git")
        .args(args)
        .current_dir(worktree)
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
