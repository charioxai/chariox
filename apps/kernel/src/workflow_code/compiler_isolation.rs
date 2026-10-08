//! MP-08 / MP-11: declarative compilation has no home-kernel ambient authority.
use super::*;
use std::io::Read;

fn isolation_error(message: impl Into<String>) -> crate::DaemonError {
    crate::DaemonError::LocalTransport {
        operation: "workflow_code.compile",
        message: message.into(),
    }
}

// Resolve only requested schema data beneath a held approved root. Never walk
// the workspace or send unrelated JSON files into the compiler process.
pub(super) struct SchemaImports {
    root: Option<PathBuf>,
    directory: Option<fs::File>,
    pub(super) files: Option<BTreeMap<String, String>>,
    pub(super) errors: BTreeMap<String, String>,
    bytes: u64,
}

impl SchemaImports {
    pub(super) fn new(root: Option<&Path>) -> Result<Self, crate::DaemonError> {
        let root = root
            .map(fs::canonicalize)
            .transpose()
            .map_err(io_error("workflow_code.compile"))?;
        let directory = root
            .as_ref()
            .map(|path| open_schema_root(path))
            .transpose()?;
        Ok(Self {
            files: root.as_ref().map(|_| BTreeMap::new()),
            root,
            directory,
            errors: BTreeMap::new(),
            bytes: 0,
        })
    }

    pub(super) fn load(
        &mut self,
        requests: Vec<String>,
        limits: &WorkflowCodeLimitsConfig,
    ) -> Result<(), crate::DaemonError> {
        let root = self.root.as_ref().ok_or_else(|| {
            isolation_error("schemaFromFile requires an approved schema import root")
        })?;
        let directory = self.directory.as_ref().unwrap();
        let files = self.files.as_mut().unwrap();
        if requests.len() > 1024 {
            return Err(isolation_error(
                "schema imports exceed configured file limit",
            ));
        }
        let mut changed = false;
        for key in requests {
            let path = Path::new(&key);
            if path.is_absolute()
                || !key.ends_with(".json")
                || key.contains('\\')
                || path
                    .components()
                    .any(|part| !matches!(part, std::path::Component::Normal(_)))
            {
                return Err(isolation_error("schemaFromFile path must stay inside the approved import root and end in .json"));
            }
            if files.contains_key(&key) || self.errors.contains_key(&key) {
                continue;
            }
            if files.len() + self.errors.len() >= 1024 {
                return Err(isolation_error(
                    "schema imports exceed configured file limit",
                ));
            }
            changed = true;
            let file = match open_schema_file(directory, root, &root.join(&key)) {
                Ok(file) => file,
                Err(_) => {
                    self.errors.insert(
                        key,
                        "schemaFromFile path is unavailable in the approved import root".into(),
                    );
                    continue;
                }
            };
            let mut data = String::new();
            if file
                .take(u64::from(limits.max_schema_bytes).saturating_sub(self.bytes) + 1)
                .read_to_string(&mut data)
                .is_err()
            {
                self.errors
                    .insert(key, "schemaFromFile data is unavailable".into());
                continue;
            }
            self.bytes = self.bytes.saturating_add(data.len() as u64);
            if self.bytes > u64::from(limits.max_schema_bytes) {
                return Err(isolation_error(
                    "schema imports exceed configured schema byte limit",
                ));
            }
            files.insert(key, data);
        }
        if !changed {
            return Err(isolation_error("schema import resolution made no progress"));
        }
        Ok(())
    }
}

// Open beneath a held approved root, without following any symlink, including
// parent directories. Canonical-path checks alone do not hold across renames.
#[cfg(target_os = "linux")]
fn open_schema_root(path: &Path) -> Result<fs::File, crate::DaemonError> {
    open_schema_path(libc::AT_FDCWD, path, libc::O_DIRECTORY, false)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn open_schema_root(_path: &Path) -> Result<fs::File, crate::DaemonError> {
    Err(isolation_error(
        "safe schema import is unavailable on this host",
    ))
}

#[cfg(target_os = "linux")]
fn open_schema_file(
    directory: &fs::File,
    root: &Path,
    path: &Path,
) -> Result<fs::File, crate::DaemonError> {
    use std::os::fd::AsRawFd;
    let relative = path
        .strip_prefix(root)
        .map_err(|_| isolation_error("schema outside approved import root"))?;
    let file = open_schema_path(directory.as_raw_fd(), relative, libc::O_NONBLOCK, true)?;
    if !file
        .metadata()
        .map_err(io_error("workflow_code.compile"))?
        .is_file()
    {
        return Err(isolation_error("schema import must be a regular file"));
    }
    Ok(file)
}

#[cfg(target_os = "linux")]
fn open_schema_path(
    directory: i32,
    path: &Path,
    flags: i32,
    beneath: bool,
) -> Result<fs::File, crate::DaemonError> {
    use std::os::fd::FromRawFd;
    use std::os::unix::ffi::OsStrExt;
    #[repr(C)]
    struct OpenHow {
        flags: u64,
        mode: u64,
        resolve: u64,
    }
    let name = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| isolation_error("invalid schema path"))?;
    const RESOLVE_BENEATH: u64 = 0x08;
    const RESOLVE_NO_SYMLINKS: u64 = 0x04;
    let how = OpenHow {
        flags: (libc::O_RDONLY | libc::O_CLOEXEC | flags) as u64,
        mode: 0,
        resolve: RESOLVE_NO_SYMLINKS | if beneath { RESOLVE_BENEATH } else { 0 },
    };
    let fd = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            directory,
            name.as_ptr(),
            &how,
            std::mem::size_of::<OpenHow>(),
        )
    };
    if fd < 0 {
        return Err(io_error("workflow_code.compile")(
            std::io::Error::last_os_error(),
        ));
    }
    Ok(unsafe { fs::File::from_raw_fd(fd as i32) })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn open_schema_file(
    _directory: &fs::File,
    _root: &Path,
    _path: &Path,
) -> Result<fs::File, crate::DaemonError> {
    Err(isolation_error(
        "safe schema import is unavailable on this host",
    ))
}

#[cfg(any(test, target_os = "macos"))]
#[path = "compiler_macho.rs"]
mod macho;

#[cfg(target_os = "macos")]
#[path = "compiler_seatbelt.rs"]
mod seatbelt;
#[cfg(target_os = "macos")]
pub(super) use seatbelt::compiler_command;

#[cfg(target_os = "macos")]
fn open_schema_root(path: &Path) -> Result<fs::File, crate::DaemonError> {
    use std::os::unix::fs::OpenOptionsExt;
    let directory = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open("/")
        .map_err(io_error("workflow_code.compile"))?;
    open_schema_beneath(
        &directory,
        path.strip_prefix("/")
            .map_err(|_| isolation_error("schema root must be absolute"))?,
        true,
    )
}

#[cfg(target_os = "macos")]
fn open_schema_file(
    directory: &fs::File,
    root: &Path,
    path: &Path,
) -> Result<fs::File, crate::DaemonError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| isolation_error("schema outside approved import root"))?;
    let file = open_schema_beneath(directory, relative, false)?;
    if !file
        .metadata()
        .map_err(io_error("workflow_code.compile"))?
        .is_file()
    {
        return Err(isolation_error("schema import must be a regular file"));
    }
    Ok(file)
}

#[cfg(target_os = "macos")]
fn open_schema_beneath(
    directory: &fs::File,
    path: &Path,
    is_directory: bool,
) -> Result<fs::File, crate::DaemonError> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    let mut held = directory
        .try_clone()
        .map_err(io_error("workflow_code.compile"))?;
    let mut parts = path.components().peekable();
    while let Some(part) = parts.next() {
        let std::path::Component::Normal(name) = part else {
            return Err(isolation_error(
                "schema path must stay beneath approved root",
            ));
        };
        let name = std::ffi::CString::new(name.as_bytes())
            .map_err(|_| isolation_error("invalid schema path"))?;
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | if parts.peek().is_some() || is_directory {
                libc::O_DIRECTORY
            } else {
                0
            };
        let fd = unsafe { libc::openat(held.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(io_error("workflow_code.compile")(
                std::io::Error::last_os_error(),
            ));
        }
        held = unsafe { fs::File::from_raw_fd(fd) };
    }
    Ok(held)
}

#[cfg(target_os = "linux")]
#[path = "compiler_linux.rs"]
mod linux;

#[cfg(target_os = "linux")]
#[path = "compiler_seccomp.rs"]
mod seccomp;

#[cfg(target_os = "linux")]
fn root_compiler_host_id() -> libc::uid_t {
    // MP-11 F14: NPROC counts every task of a host UID, including unrelated
    // services using nobody. Choose a high, process-private identity once in the
    // parent, so the two compiler jobs share a budget without sharing service UIDs.
    static ID: std::sync::OnceLock<libc::uid_t> = std::sync::OnceLock::new();
    *ID.get_or_init(|| 1_000_000 + rand::random::<u32>() % 1_000_000_000)
}

#[cfg(target_os = "linux")]
pub(super) fn compiler_command(
    node: &Path,
    limits: &WorkflowCodeLimitsConfig,
) -> Result<Command, crate::DaemonError> {
    use std::os::unix::process::CommandExt;
    let node = fs::canonicalize(node).map_err(io_error("workflow_code.compile"))?;
    let mounts = linux::runtime_files(&node)?;
    if !Path::new("/usr/bin/bwrap").is_file() {
        return Err(isolation_error(
            "isolated workflow compilation requires /usr/bin/bwrap",
        ));
    }
    let mut command = Command::new("/usr/bin/bwrap");
    command
        .env_clear()
        .args([
            "--unshare-all",
            "--die-with-parent",
            "--new-session",
            "--uid",
            "65534",
            "--gid",
            "65534",
            "--cap-drop",
            "ALL",
            "--clearenv",
            "--perms",
            "0500",
            "--size",
        ])
        .arg(limits.script_memory_bytes.max(4096).to_string())
        .args(["--tmpfs", "/tmp", "--chdir", "/tmp"]);
    for path in mounts {
        command.arg("--ro-bind").arg(&path).arg(&path);
    }
    command
        .arg("--ro-bind")
        .arg(&node)
        .arg("/compiler/node")
        .args(["--remount-ro", "/", "--seccomp", "3", "/compiler/node"]);
    let filter = seccomp::compiler_filter()?;
    // V8's heap flag alone does not bound native allocations / ArrayBuffers.
    let address_limit = limits
        .script_memory_bytes
        .saturating_add(1024 * 1024 * 1024);
    let cpu_seconds = limits.script_timeout_ms.div_ceil(1000).max(1);
    let drop_root_privileges = unsafe { libc::getuid() == 0 || libc::geteuid() == 0 };
    let host_id = drop_root_privileges.then(root_compiler_host_id);
    unsafe {
        command.pre_exec(move || {
            // Mark every non-stdio descriptor close-on-exec, including a descriptor
            // opened by an embedding transport without its usual CLOEXEC flag.
            const CLOSE_RANGE_CLOEXEC: u32 = 4;
            if libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, CLOSE_RANGE_CLOEXEC) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            for (resource, value) in [
                (libc::RLIMIT_AS, address_limit),
                (libc::RLIMIT_CPU, cpu_seconds),
                (libc::RLIMIT_CORE, 0),
            ] {
                let limit = libc::rlimit {
                    rlim_cur: value as libc::rlim_t,
                    rlim_max: value as libc::rlim_t,
                };
                if libc::setrlimit(resource, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            if let Some(host_id) = host_id {
                // NPROC counts the host UID. A namespace UID change alone
                // retains host root's exemption. Use an unprivileged host UID.
                // Ordinary users retain their existing per-user process limit;
                // the syscall, address-space and time bounds still apply.
                let limit = libc::rlimit {
                    rlim_cur: 64,
                    rlim_max: 64,
                };
                if libc::setrlimit(libc::RLIMIT_NPROC, &limit) != 0
                    || libc::setgroups(0, std::ptr::null()) != 0
                    || libc::setgid(host_id) != 0
                    || libc::setuid(host_id) != 0
                {
                    return Err(std::io::Error::last_os_error());
                }
            }
            seccomp::install_filter_fd(&filter)
        });
    }
    Ok(command)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn compiler_command(
    _node: &Path,
    _limits: &WorkflowCodeLimitsConfig,
) -> Result<Command, crate::DaemonError> {
    Err(isolation_error("isolated workflow compilation is unavailable on this host; a supported compiler isolation backend is required"))
}

#[cfg(all(test, target_os = "macos"))]
#[path = "compiler_seatbelt_tests.rs"]
mod macos_tests;

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    // MP-11 F14: a realm escape cannot turn the compiler into a process launcher.
    #[test]
    fn security_f14_linux_compiler_cannot_spawn_its_own_node() {
        let node = discover_workflow_code_node_path().unwrap();
        let mut command = compiler_command(&node, &WorkflowCodeLimitsConfig::default()).unwrap();
        let output = command.args(["--disable-wasm-trap-handler", "-e",
            "const {spawnSync}=require('node:child_process'); const r=spawnSync(process.execPath,['--disable-wasm-trap-handler','-e','process.exit(0)']); if(!r.error || !['EPERM','EAGAIN'].includes(r.error.code)) process.exit(1);"
        ]).output().unwrap();
        assert!(
            output.status.success(),
            "MP-11 F14: sandbox allowed a child Node: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // MP-11 F14: NPROC applies to the host real UID, not the namespace UID.
    #[test]
    fn security_f14_root_compiler_uses_nonroot_host_uid() {
        if unsafe { libc::geteuid() } != 0 {
            return;
        }
        fn compiler_uid(pid: u32) -> Option<u32> {
            let root = std::path::PathBuf::from(format!("/proc/{pid}"));
            if std::fs::read_to_string(root.join("comm")).ok()?.trim() == "node" {
                let status = std::fs::read_to_string(root.join("status")).ok()?;
                return status
                    .lines()
                    .find(|line| line.starts_with("Uid:"))?
                    .split_whitespace()
                    .nth(1)?
                    .parse()
                    .ok();
            }
            let children =
                std::fs::read_to_string(root.join(format!("task/{pid}/children"))).ok()?;
            children
                .split_whitespace()
                .filter_map(|id| id.parse::<u32>().ok())
                .filter(|id| *id > 1)
                .find_map(compiler_uid)
        }
        let node = discover_workflow_code_node_path().unwrap();
        let mut command = compiler_command(&node, &WorkflowCodeLimitsConfig::default()).unwrap();
        let child = command
            .args([
                "--disable-wasm-trap-handler",
                "-e",
                "setTimeout(() => {}, 2000)",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        assert!(child.id() > 1);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut uid = None;
        while std::time::Instant::now() < deadline && uid.is_none() {
            uid = compiler_uid(child.id());
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "compiler UID diagnostic must run");
        assert!(
            uid.is_some_and(|uid| uid != 0),
            "MP-11 F14: host root UID exempts compiler NPROC limit"
        );
    }

    #[test]
    fn security_f14_root_compiler_avoids_shared_service_uid() {
        assert!(
            root_compiler_host_id() >= 1_000_000,
            "MP-11 F14: root compiler shares a normal host service UID and its task budget"
        );
    }

    #[test]
    fn compiler_isolation_denies_host_files_commands_environment_and_network() {
        let node = discover_workflow_code_node_path().expect("real Node required");
        let mut command = compiler_command(&node, &WorkflowCodeLimitsConfig::default()).unwrap();
        // Trusted diagnostics test the OS boundary directly. These imports are
        // never provided to workflow source or to an agent prompt.
        let diagnostic = r#"
import fs from 'node:fs';
import net from 'node:net';
import {spawnSync} from 'node:child_process';
// Bubblewrap creates this PWD after clearing the inherited environment.
if (Object.keys(process.env).some(key => key !== 'PWD') || process.env.PWD !== '/tmp') throw new Error('inherited environment');
if (process.getuid() === 0 || process.getgid() === 0) throw new Error('privileged compiler identity');
if (process.cwd() !== '/tmp' || fs.readdirSync('/tmp').length) throw new Error('scratch not empty');
let readonly = false;
try {fs.writeFileSync('/marker', 'x')} catch (error) {readonly = error.code === 'EROFS'}
if (!readonly) throw new Error('root is writable');
let scratchReadonly = false;
try {fs.writeFileSync('/tmp/marker', 'x')} catch (error) {scratchReadonly = ['EACCES', 'EPERM', 'EROFS'].includes(error.code)}
if (!scratchReadonly) throw new Error('scratch is writable');
for (const path of ['/root', '/etc/hostname', '/bin/sh', '/proc']) {
  if (fs.existsSync(path)) throw new Error('host file exposed');
}
const child = spawnSync('/bin/sh', ['-c', 'exit 0']);
if (!['ENOENT', 'EPERM', 'EAGAIN'].includes(child.error?.code)) throw new Error('host command available');
await new Promise((resolve, reject) => {
 const socket = net.connect({host: '1.1.1.1', port: 443});
 socket.on('connect', () => {socket.destroy(); reject(new Error('network available'))});
 socket.on('error', () => resolve());
 socket.setTimeout(1000, () => {socket.destroy(); resolve()});
});
process.stdout.write('isolated');
"#;
        let output = command
            .args([
                "--disable-wasm-trap-handler",
                "--input-type=module",
                "-e",
                diagnostic,
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"isolated");
    }

    #[test]
    fn compiler_isolation_does_not_inherit_parent_file_handles() {
        use std::os::fd::{AsRawFd, FromRawFd};
        let worktree = crate::test_support::TestWorktree::new("compiler-parent-handle");
        let path = worktree.path().join("public-fixture.txt");
        fs::write(&path, "MP-08 / MP-11 public fixture").unwrap();
        let file = fs::File::open(path).unwrap();
        let descriptor = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD, 128) };
        assert!(descriptor >= 128);
        let _owned_handle = unsafe { fs::File::from_raw_fd(descriptor) };
        let node = discover_workflow_code_node_path().unwrap();
        let mut command = compiler_command(&node, &WorkflowCodeLimitsConfig::default()).unwrap();
        let diagnostic = format!("const fs=require('node:fs');try{{fs.fstatSync({descriptor});process.exit(1)}}catch(e){{if(e.code!=='EBADF')process.exit(2)}}");
        let output = command
            .args(["--disable-wasm-trap-handler", "-e", &diagnostic])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn compiler_isolation_schema_open_rejects_parent_directory_links() {
        use std::os::unix::fs::symlink;
        let root = crate::test_support::TestWorktree::new("compiler-schema-root");
        let outside = crate::test_support::TestWorktree::new("compiler-schema-outside");
        fs::write(outside.path().join("sample.json"), r#"{"type":"string"}"#).unwrap();
        symlink(outside.path(), root.path().join("schemas")).unwrap();
        let directory = fs::File::open(root.path()).unwrap();
        assert!(open_schema_file(
            &directory,
            root.path(),
            &root.path().join("schemas/sample.json")
        )
        .is_err());
        assert!(open_schema_root(&root.path().join("schemas")).is_err());
    }

    #[test]
    fn compiler_isolation_schema_open_rejects_nonregular_files_without_waiting() {
        use std::os::unix::ffi::OsStrExt;
        let root = crate::test_support::TestWorktree::new("compiler-schema-pipe");
        let path = root.path().join("schema.json");
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let directory = fs::File::open(root.path()).unwrap();
        let error = open_schema_file(&directory, root.path(), &path).unwrap_err();
        assert!(error.to_string().contains("regular file"));
    }

    #[test]
    fn compiler_isolation_enforces_async_timeout() {
        let limits = WorkflowCodeLimitsConfig {
            script_timeout_ms: 500,
            ..Default::default()
        };
        let error =
            compile_workflow_code_javascript("ignored", "await new Promise(() => {})", &limits)
                .unwrap_err();
        // Node may settle an unresolved top-level await itself or the parent
        // timeout may reap it; neither can hold the kernel indefinitely.
        assert!(
            error.to_string().contains("compiler failed") || error.to_string().contains("timeout"),
            "{error}"
        );
    }
}
