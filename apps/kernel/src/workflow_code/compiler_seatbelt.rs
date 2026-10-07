//! macOS compiler boundary. No home, workspace, network, fork or host IPC access.
use super::*;
use std::io::{Read, Seek, SeekFrom};
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
        let (libraries, rpaths) = load_commands(&canonical)?;
        for dependency in libraries {
            let dependency = if let Some(relative) = dependency.strip_prefix("@rpath/") {
                rpaths.iter().filter_map(|rpath| resolve_library_path(rpath, &path, node).ok())
                    .map(|directory| normalize_path(&directory.join(relative)))
                    .find(|candidate| candidate.is_file())
                    .ok_or_else(|| isolation_error("compiler runtime has an unresolved library search path; use Node with self-contained runtime dependencies"))?
            } else {
                resolve_library_path(&dependency, &path, node)?
            };
            if !files.contains(&dependency) {
                pending.push(dependency);
            }
        }
    }
    Ok(files)
}

const LC_REQ_DYLD: u32 = 0x8000_0000;
const LC_RPATH: u32 = 0x1c | LC_REQ_DYLD;
// LC_LOAD_DYLIB, LC_LOAD_WEAK_DYLIB, LC_REEXPORT_DYLIB, LC_LAZY_LOAD_DYLIB, LC_LOAD_UPWARD_DYLIB.
const LC_LOAD_DYLIBS: [u32; 5] = [
    0xc,
    0x18 | LC_REQ_DYLD,
    0x1f | LC_REQ_DYLD,
    0x20,
    0x23 | LC_REQ_DYLD,
];

fn word(bytes: &[u8], at: usize, big_endian: bool) -> Option<u32> {
    let bytes = bytes.get(at..at.checked_add(4)?)?.try_into().ok()?;
    Some(if big_endian {
        u32::from_be_bytes(bytes)
    } else {
        u32::from_le_bytes(bytes)
    })
}

/// Linked library and search-path names of every architecture slice, read from
/// the Mach-O load commands so compilation needs no developer tools.
fn load_commands(path: &Path) -> Result<(Vec<String>, Vec<String>), crate::DaemonError> {
    let invalid = || isolation_error("compiler runtime is not a supported Mach-O binary");
    let mut file = fs::File::open(path).map_err(io_error("workflow_code.compile"))?;
    let mut read = |offset: u64, length: usize| {
        let mut bytes = vec![0; length];
        file.seek(SeekFrom::Start(offset))
            .and_then(|_| file.read_exact(&mut bytes))
            .map(|_| bytes)
            .map_err(|_| invalid())
    };
    let header = read(0, 8)?;
    let slices = match word(&header, 0, true) {
        // Universal binaries: FAT_MAGIC and FAT_MAGIC_64 headers are big-endian.
        Some(magic @ (0xcafe_babe | 0xcafe_babf)) => {
            let count = word(&header, 4, true).ok_or_else(invalid)? as usize;
            let size = if magic == 0xcafe_babf { 32 } else { 20 };
            if count == 0 || count > 16 {
                return Err(invalid());
            }
            let table = read(8, count * size)?;
            table
                .chunks(size)
                .map(|entry| match size {
                    32 => Some(
                        u64::from(word(entry, 8, true)?) << 32 | u64::from(word(entry, 12, true)?),
                    ),
                    _ => word(entry, 8, true).map(u64::from),
                })
                .collect::<Option<Vec<_>>>()
                .ok_or_else(invalid)?
        }
        _ => vec![0],
    };
    let (mut libraries, mut rpaths) = (Vec::new(), Vec::new());
    for offset in slices {
        let header = read(offset, 32)?;
        // MH_MAGIC_64; arm64 and x86_64 runtimes are little-endian.
        if word(&header, 0, false) != Some(0xfeed_facf) {
            return Err(invalid());
        }
        let count = word(&header, 16, false).ok_or_else(invalid)?;
        let size = word(&header, 20, false).ok_or_else(invalid)? as usize;
        if size > 1024 * 1024 {
            return Err(invalid());
        }
        let commands = read(offset + 32, size)?;
        let mut at = 0;
        for _ in 0..count {
            let command = word(&commands, at, false).ok_or_else(invalid)?;
            let length = word(&commands, at + 4, false).ok_or_else(invalid)? as usize;
            let body = commands
                .get(at..at + length)
                .filter(|_| length >= 8)
                .ok_or_else(invalid)?;
            let names = if LC_LOAD_DYLIBS.contains(&command) {
                &mut libraries
            } else if command == LC_RPATH {
                &mut rpaths
            } else {
                at += length;
                continue;
            };
            // Both commands store their name as an lc_str offset after the header.
            let name = word(body, 8, false)
                .and_then(|start| body.get(start as usize..))
                .and_then(|name| name.split(|byte| *byte == 0).next())
                .and_then(|name| std::str::from_utf8(name).ok())
                .ok_or_else(invalid)?;
            names.push(name.to_string());
            at += length;
        }
    }
    Ok((libraries, rpaths))
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
    fn macos_compiler_memory_monitor_allows_node_baseline_above_heap_budget() {
        // Node's resident baseline alone exceeds a minimal V8 heap budget.
        let limits = WorkflowCodeLimitsConfig {
            script_memory_bytes: 16 * 1024 * 1024,
            ..WorkflowCodeLimitsConfig::default()
        };
        compile_workflow_code_javascript(
            "/ignored",
            "workflow.define({alias: 'memory-headroom'});",
            &limits,
        )
        .unwrap();
    }

    #[test]
    fn macos_runtime_libraries_are_read_from_mach_o_load_commands() {
        fn command(kind: u32, name: &str) -> Vec<u8> {
            let offset = if kind == LC_RPATH { 12 } else { 24 };
            let mut bytes = [kind, 0, offset].map(u32::to_le_bytes).concat();
            bytes.resize(offset as usize, 0);
            bytes.extend(name.as_bytes());
            bytes.resize((bytes.len() + 8) & !7, 0);
            let length = bytes.len() as u32;
            bytes[4..8].copy_from_slice(&length.to_le_bytes());
            bytes
        }
        let commands = [
            command(0xd, "@rpath/libself.dylib"),
            command(0xc, "/usr/lib/libSystem.B.dylib"),
            command(0x18 | LC_REQ_DYLD, "@loader_path/libweak.dylib"),
            command(LC_RPATH, "@loader_path/../lib"),
        ]
        .concat();
        let mut thin = [
            0xfeed_facf,
            0x0100_000c,
            0,
            6,
            4,
            commands.len() as u32,
            0,
            0,
        ]
        .map(u32::to_le_bytes)
        .concat();
        thin.extend(&commands);
        let mut fat = [0xcafe_babe, 1, 0x0100_000c, 0, 4096, thin.len() as u32, 12]
            .map(u32::to_be_bytes)
            .concat();
        fat.resize(4096, 0);
        fat.extend(&thin);

        let directory = Scratch::new().unwrap();
        for (name, bytes) in [("thin", &thin), ("fat", &fat)] {
            let path = directory.0.join(name);
            fs::write(&path, bytes).unwrap();
            let (libraries, rpaths) = load_commands(&path).unwrap();
            assert_eq!(
                libraries,
                ["/usr/lib/libSystem.B.dylib", "@loader_path/libweak.dylib"]
            );
            assert_eq!(rpaths, ["@loader_path/../lib"]);
        }
        let truncated = directory.0.join("truncated");
        fs::write(&truncated, &thin[..thin.len() - 8]).unwrap();
        assert!(load_commands(&truncated).is_err());

        let node = fs::canonicalize(discover_workflow_code_node_path().unwrap()).unwrap();
        let files = runtime_files(&node).unwrap();
        assert!(files.contains(Path::new("/usr/lib/libSystem.B.dylib")));
    }
}
