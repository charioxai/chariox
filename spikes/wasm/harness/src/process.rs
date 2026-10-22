//! Worker process launch as the kernel supervisor does it: clean environment,
//! FD3 SDK channel, FD4 Ready/Continue (wasm worker), optional bubblewrap.
use chariox_app_runtime::wire::{Channel, Sender};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const GENERATION: &str = "7";
const INSTALLATION: &str = "installation_1";
const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const TOOLS: [&str; 7] = ["echo", "counter", "fetch", "transform", "compute", "spin", "hog"];

pub struct Variant {
    pub name: String,
    pub sandbox: &'static str,
    wasm: bool,
}

impl Variant {
    pub fn from_args(name: &str, bwrap: bool) -> Self {
        Self { name: name.into(), sandbox: if bwrap { "bwrap" } else { "none" }, wasm: name.starts_with("wasm") }
    }
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("missing {name}"))
}

pub struct Spawned {
    child: Mutex<Child>,
    channel: Mutex<Option<UnixStream>>,
    _stderr: Option<std::thread::JoinHandle<()>>,
}

/// Only exact PIDs this harness started; never 0, 1 or a group.
fn kill(pid: i32) {
    assert!(pid > 1, "refusing to signal pid {pid}");
    unsafe { libc::kill(pid, libc::SIGKILL) };
}

fn descendants(root: u32) -> Vec<u32> {
    let mut found = vec![root];
    let mut index = 0;
    while index < found.len() {
        let parent = found[index];
        for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { continue };
            let after = &stat[stat.rfind(')').unwrap_or(0) + 2..];
            if after.split(' ').nth(1).and_then(|p| p.parse::<u32>().ok()) == Some(parent) && !found.contains(&pid) {
                found.push(pid);
            }
        }
        index += 1;
    }
    found
}

fn status_kb(pid: u32, field: &str) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default().lines()
        .find(|line| line.starts_with(field))
        .and_then(|line| line.split_whitespace().nth(1)?.parse().ok())
        .unwrap_or(0)
}

impl Spawned {
    pub fn channel(&self) -> Channel<tokio::net::UnixStream> {
        let stream = self.channel.lock().unwrap().take().unwrap();
        stream.set_nonblocking(true).unwrap();
        Channel::new(tokio::net::UnixStream::from_std(stream).unwrap(), GENERATION.into(), Sender::Worker).unwrap()
    }

    fn pid(&self) -> u32 {
        self.child.lock().unwrap().id()
    }

    /// The worker is the largest process in the launched tree (bwrap adds two
    /// small ones). Values in KiB from /proc/<pid>/status.
    pub fn memory(&self) -> Value {
        let tree = descendants(self.pid());
        let worker = *tree.iter().max_by_key(|pid| status_kb(**pid, "VmRSS:")).unwrap();
        json!({
            "rssKb": status_kb(worker, "VmRSS:"), "hwmKb": status_kb(worker, "VmHWM:"),
            "anonKb": status_kb(worker, "RssAnon:"), "treeRssKb": tree.iter().map(|p| status_kb(*p, "VmRSS:")).sum::<u64>(),
            "processes": tree.len(),
        })
    }

    pub fn exit_status(&self) -> Value {
        match self.child.lock().unwrap().try_wait() {
            Ok(Some(status)) => json!(format!("{status}")),
            _ => json!("running"),
        }
    }

    pub fn kill_and_reap(&self) -> bool {
        let tree = descendants(self.pid());
        kill(self.pid() as i32);
        let _ = self.child.lock().unwrap().wait();
        let deadline = Instant::now() + Duration::from_secs(5);
        while tree.iter().any(|pid| std::path::Path::new(&format!("/proc/{pid}")).exists()) {
            if Instant::now() > deadline { return false; }
            std::thread::sleep(Duration::from_micros(200));
        }
        true
    }

    pub fn stop(&self) {
        self.kill_and_reap();
    }
}

fn scratch() -> (String, String) {
    static RUN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let base = format!("{}/run/{}-{}", env("SPIKE_ROOT"), std::process::id(), RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let (data, tmp) = (format!("{base}/data"), format!("{base}/tmp"));
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&tmp).unwrap();
    (data, tmp)
}

/// The same per-App memory budget for both runtimes: wasm linear memory cap
/// or V8 old space.
fn memory_mb() -> String {
    std::env::var("SPIKE_MEMORY_MB").unwrap_or("128".into())
}

pub fn spawn(variant: &Variant) -> Spawned {
    let root = env("SPIKE_ROOT");
    let package = format!("{root}/artifacts/{}", variant.name);
    let runtime = format!("{root}/artifacts/runtime");
    let (data, tmp) = scratch();
    let mut environment = vec![
        ("CHARIOX_APP_GENERATION".to_string(), GENERATION.to_string()),
        ("CHARIOX_APP_INSTALLATION".into(), INSTALLATION.into()),
        ("CHARIOX_APP_RELEASE_DIGEST".into(), DIGEST.into()),
        ("CHARIOX_APP_PACKAGE".into(), package.clone()),
        ("CHARIOX_APP_DATA".into(), data.clone()),
        ("CHARIOX_APP_TMP".into(), tmp.clone()),
    ];
    let (program, mut arguments, binds) = if variant.wasm {
        environment.push(("CHARIOX_WASM_ENTRY".into(), std::env::var("SPIKE_WASM_ENTRY").unwrap_or("app.cwasm".into())));
        environment.push(("CHARIOX_WASM_MEMORY_MB".into(), memory_mb()));
        environment.push(("CHARIOX_WASM_CALL_MS".into(), "1000".into()));
        environment.push(("CHARIOX_WASM_POOL".into(), std::env::var("SPIKE_WASM_POOL").unwrap_or("1".into())));
        let worker = env("SPIKE_WORKER");
        let dir = std::path::Path::new(&worker).parent().unwrap().to_string_lossy().to_string();
        (worker, Vec::new(), vec![dir])
    } else {
        let mut tools = TOOLS.to_vec();
        if variant.name == "node-jco" { tools.push("__warm"); }
        let launch = json!({ "version": 1, "entry": "runtime/main.mjs", "declarations": { "tools": tools, "incomingEvents": [] }, "startupTimeoutMs": 10000 });
        let bootstrap = format!("{runtime}/bootstrap.cjs");
        let script = format!("require('node:module').createRequire({b:?})({b:?}).start({launch});", b = bootstrap);
        let node = env("SPIKE_NODE");
        let mut arguments: Vec<String> = vec!["--no-warnings".into(), format!("--max-old-space-size={}", memory_mb()), "--permission".into(), "--allow-worker".into(), "--no-addons".into()];
        for path in [&runtime, &package, &data, &tmp] { arguments.push(format!("--allow-fs-read={path}")); }
        for path in [&data, &tmp] { arguments.push(format!("--allow-fs-write={path}")); }
        arguments.extend(std::env::var("SPIKE_NODE_FLAGS").unwrap_or_default().split_whitespace().map(String::from));
        arguments.extend(["-e".into(), script]);
        let dir = std::path::Path::new(&node).parent().unwrap().parent().unwrap().to_string_lossy().to_string();
        (node, arguments, vec![dir])
    };
    let (parent3, child3) = UnixStream::pair().unwrap();
    let (parent4, child4) = UnixStream::pair().unwrap();
    let mut command = if variant.sandbox == "bwrap" {
        let mut wrap = Command::new("/usr/bin/bwrap");
        wrap.args(["--unshare-user", "--unshare-pid", "--unshare-ipc", "--unshare-net", "--unshare-uts", "--hostname", "chariox-app",
            "--die-with-parent", "--new-session", "--cap-drop", "ALL", "--clearenv",
            "--ro-bind", "/usr", "/usr", "--symlink", "usr/lib", "/lib", "--symlink", "usr/lib64", "/lib64", "--symlink", "usr/bin", "/bin",
            "--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"]);
        for dir in binds.iter().chain([&runtime, &package]) { wrap.args(["--ro-bind", dir, dir]); }
        for dir in [&data, &tmp] { wrap.args(["--bind", dir, dir]); }
        for (key, value) in &environment { wrap.args(["--setenv", key, value]); }
        wrap.args(["--chdir", &data, "--", &program]);
        wrap.args(&arguments);
        arguments.clear();
        wrap
    } else {
        let mut direct = Command::new(&program);
        direct.args(&arguments).env_clear().envs(environment.iter().cloned()).current_dir(&data);
        direct
    };
    let (fd3, fd4) = (child3.as_raw_fd(), child4.as_raw_fd());
    unsafe {
        command.pre_exec(move || {
            let high3 = libc::fcntl(fd3, libc::F_DUPFD, 100);
            let high4 = libc::fcntl(fd4, libc::F_DUPFD, 100);
            if high3 < 0 || high4 < 0 || libc::dup2(high3, 3) < 0 || libc::dup2(high4, 4) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::close(high3);
            libc::close(high4);
            Ok(())
        });
    }
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn worker");
    drop(child3);
    drop(child4);
    let stderr = child.stderr.take().unwrap();
    let label = variant.name.clone();
    let reader = std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
            eprintln!("[{label}] {line}");
        }
    });
    if variant.wasm {
        continue_startup(parent4);
    }
    Spawned { child: Mutex::new(child), channel: Mutex::new(Some(parent3)), _stderr: Some(reader) }
}

/// Checks the worker's FD4 identity record and sends the continuation.
fn continue_startup(mut startup: UnixStream) {
    startup.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
    let mut magic = [0u8; 8];
    startup.read_exact(&mut magic).unwrap();
    assert_eq!(&magic, b"CXAWR001");
    for expected in [GENERATION, INSTALLATION, DIGEST] {
        let mut length = [0u8; 4];
        startup.read_exact(&mut length).unwrap();
        let mut value = vec![0u8; u32::from_be_bytes(length) as usize];
        startup.read_exact(&mut value).unwrap();
        assert_eq!(value, expected.as_bytes(), "launch identity mismatch");
    }
    startup.write_all(b"CXAWGO01").unwrap();
}
