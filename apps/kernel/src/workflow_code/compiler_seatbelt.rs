//! macOS compiler boundary. No home, workspace, network, fork or host IPC access.
use super::*;
use std::os::unix::{fs::DirBuilderExt, process::CommandExt};

const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Self, crate::DaemonError> {
        // Atomic private directory creation; never reuse an existing path.
        let path =
            env::temp_dir().join(format!("chariox-compiler-{:032x}", rand::random::<u128>()));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(io_error("workflow_code.compile"))?;
        let mut scratch = Self(path);
        scratch.0 = fs::canonicalize(&scratch.0).map_err(io_error("workflow_code.compile"))?;
        Ok(scratch)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // This exact directory was created by us and contains only compiler state.
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn literal(path: &Path) -> Result<String, crate::DaemonError> {
    let path = path
        .to_str()
        .ok_or_else(|| isolation_error("compiler runtime path must be UTF-8"))?;
    // SBPL quoted strings use the same escaping for quotes and backslashes.
    serde_json::to_string(path).map_err(|_| isolation_error("invalid compiler runtime path"))
}

fn runtime_files(node: &Path) -> Result<BTreeSet<PathBuf>, crate::DaemonError> {
    let mut files = BTreeSet::new();
    let mut pending = vec![node.to_path_buf()];
    let mut inspected = BTreeSet::new();
    while let Some(path) = pending.pop() {
        files.insert(path.clone());
        if path.starts_with("/usr/lib") || path.starts_with("/System/Library") {
            // These are supplied by Apple's dyld shared cache, often with no file.
            continue;
        }
        let canonical = fs::canonicalize(&path).map_err(io_error("workflow_code.compile"))?;
        files.insert(canonical.clone());
        // dyld resolves directory symlinks before library-file symlinks. Allow
        // the intermediate names of this same runtime asset, not its directory.
        for ancestor in path.ancestors().skip(1) {
            if let Ok(directory) = fs::canonicalize(ancestor) {
                files.insert(directory.join(path.strip_prefix(ancestor).unwrap()));
            }
        }

        if !inspected.insert(canonical.clone()) {
            continue;
        }
        if inspected.len() > 256 {
            return Err(isolation_error(
                "compiler runtime dependency limit exceeded",
            ));
        }
        let output = Command::new("/usr/bin/otool")
            .env_clear()
            .args(["-L"])
            .arg(&canonical)
            .output()
            .map_err(io_error("workflow_code.compile"))?;
        if !output.status.success() {
            return Err(isolation_error(
                "cannot identify compiler runtime libraries with /usr/bin/otool",
            ));
        }
        let libraries = String::from_utf8_lossy(&output.stdout);
        let rpaths = if libraries.contains("@rpath/") {
            let output = Command::new("/usr/bin/otool")
                .env_clear()
                .arg("-l")
                .arg(&canonical)
                .output()
                .map_err(io_error("workflow_code.compile"))?;
            if !output.status.success() {
                return Err(isolation_error(
                    "cannot identify compiler runtime search paths",
                ));
            }
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| {
                    line.trim()
                        .strip_prefix("path ")
                        .and_then(|line| line.split_once(" (offset "))
                        .map(|(name, _)| name.to_string())
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        for (index, line) in libraries.lines().skip(1).enumerate() {
            let dependency = line
                .trim()
                .split(" (compatibility version")
                .next()
                .unwrap_or_default();
            // otool lists a dylib's own install name before its dependencies.
            if index == 0 && canonical.extension().is_some_and(|ext| ext == "dylib") {
                continue;
            }
            let dependency = if let Some(relative) = dependency.strip_prefix("@rpath/") {
                rpaths.iter().filter_map(|rpath| resolve_library_path(rpath, &path, node).ok())
                    .map(|directory| normalize_path(&directory.join(relative)))
                    .find(|candidate| candidate.is_file())
                    .ok_or_else(|| isolation_error("compiler runtime has an unresolved library search path; use Node with self-contained runtime dependencies"))?
            } else {
                resolve_library_path(dependency, &path, node)?
            };
            if !files.contains(&dependency) {
                pending.push(dependency);
            }
        }
    }
    Ok(files)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                result.pop();
            }
            std::path::Component::CurDir => {}
            part => result.push(part.as_os_str()),
        }
    }
    result
}

fn resolve_library_path(
    name: &str,
    library: &Path,
    node: &Path,
) -> Result<PathBuf, crate::DaemonError> {
    let path = if let Some(relative) = name.strip_prefix("@loader_path/") {
        library
            .parent()
            .ok_or_else(|| isolation_error("runtime library has no parent"))?
            .join(relative)
    } else if let Some(relative) = name.strip_prefix("@executable_path/") {
        node.parent()
            .ok_or_else(|| isolation_error("Node runtime has no parent"))?
            .join(relative)
    } else if name.starts_with('/') {
        PathBuf::from(name)
    } else {
        return Err(isolation_error("compiler runtime has an unresolved library path; use Node with self-contained runtime dependencies"));
    };
    Ok(normalize_path(&path))
}

pub(in crate::workflow_code) fn compiler_command(
    node: &Path,
    limits: &WorkflowCodeLimitsConfig,
) -> Result<Command, crate::DaemonError> {
    command_with_backend(node, limits, Path::new(SANDBOX_EXEC))
}

fn command_with_backend(
    node: &Path,
    limits: &WorkflowCodeLimitsConfig,
    backend: &Path,
) -> Result<Command, crate::DaemonError> {
    if !backend.is_file() {
        return Err(isolation_error("isolated workflow compilation requires Apple's /usr/bin/sandbox-exec; restore Seatbelt availability or use a supported Linux host with Bubblewrap"));
    }
    let node = fs::canonicalize(node).map_err(io_error("workflow_code.compile"))?;
    let files = runtime_files(&node)?;
    let scratch = Scratch::new()?;
    let mut profile = format!(
        "(version 1)\n(deny default)\n(allow process-exec (literal {}))\n\
         (allow file-read* file-write* (subpath {}))\n\
         (allow file-read* (literal \"/dev/null\") (literal \"/dev/random\") (literal \"/dev/urandom\"))\n\
         (allow file-read-data (require-all (literal \"/\") (vnode-type DIRECTORY)))\n\
         (allow sysctl-read (sysctl-name-prefix \"hw.\") (sysctl-name \"kern.osrelease\") (sysctl-name \"kern.osversion\") (sysctl-name \"kern.ostype\") (sysctl-name \"kern.version\") (sysctl-name \"kern.hostname\"))\n\
         (allow process-info* (target same-sandbox))\n",
        literal(&node)?, literal(&scratch.0)?
    );
    let mut directories = BTreeSet::new();
    for file in files {
        directories.extend(file.ancestors().skip(1).map(Path::to_path_buf));
        profile.push_str(&format!(
            "(allow file-read* (literal {}))\n",
            literal(&file)?
        ));
    }
    // Path traversal and dyld's symlink resolution need ancestor metadata only.
    for directory in directories {
        profile.push_str(&format!(
            "(allow file-read-metadata (literal {}))\n",
            literal(&directory)?
        ));
    }
    let mut command = Command::new(backend);
    command
        .env_clear()
        .current_dir(&scratch.0)
        .args(["-p", &profile])
        .arg(&node)
        // Homebrew OpenSSL otherwise reads operator configuration outside runtime assets.
        .arg("--openssl-config=/dev/null");
    let cpu_seconds = limits.script_timeout_ms.div_ceil(1000).max(1);
    let scratch_bytes = limits.script_memory_bytes.max(4096);
    unsafe {
        command.pre_exec(move || {
            // Keep the private directory alive until the command and child are done.
            let _ = &scratch;
            // Mark all inherited handles CLOEXEC, including the spawn error pipe.
            // fcntl is async-signal-safe; do not allocate or inspect directories here.
            let maximum = libc::getdtablesize();
            if maximum < 0 {
                return Err(std::io::Error::last_os_error());
            }
            for fd in 3..maximum {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags >= 0 {
                    if libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                } else if std::io::Error::last_os_error().raw_os_error() != Some(libc::EBADF) {
                    return Err(std::io::Error::last_os_error());
                }
            }
            for (resource, value) in [
                (libc::RLIMIT_CPU, cpu_seconds),
                (libc::RLIMIT_CORE, 0),
                (libc::RLIMIT_FSIZE, scratch_bytes),
            ] {
                let limit = libc::rlimit {
                    rlim_cur: value,
                    rlim_max: value,
                };
                if libc::setrlimit(resource, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_compiler_fails_closed_without_seatbelt() {
        let result = command_with_backend(
            Path::new("/unused-node"),
            &WorkflowCodeLimitsConfig::default(),
            Path::new("/unavailable-seatbelt"),
        );
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("/usr/bin/sandbox-exec"));
    }

    #[test]
    fn macos_compiler_memory_monitor_refuses_low_budget() {
        let limits = WorkflowCodeLimitsConfig {
            script_memory_bytes: 4096,
            ..WorkflowCodeLimitsConfig::default()
        };
        let result = compile_workflow_code_javascript(
            "/ignored",
            "workflow.define({alias: 'memory-limit'});",
            &limits,
        );
        assert!(result.unwrap_err().to_string().contains("memory limit"));
    }
}
