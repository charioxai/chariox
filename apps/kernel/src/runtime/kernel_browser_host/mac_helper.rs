//! macOS M1: kernel-supervised Computer helper, one seat behind the shared adapter.
//! LaunchServices starts the installed helper with only a 0700 rendezvous path in
//! argv. The helper dials the kernel's socket; each end verifies the other's
//! audit token against a code requirement, then the helper echoes the one-use
//! 0600 bootstrap token and epoch. A 1 s heartbeat holds the helper's 2 s lease.
//! Disconnect, lease loss, Stop or a stale reply fences the seat and reaps it.
use super::computer_backend::ComputerBackend;
use crate::error::{HostFailure, UserDomainRefusalReason};
use crate::runtime::browser_controller_process::BrowserCancellation;
use code_identity::Peer;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

mod code_identity;
#[cfg(test)]
mod tests;

const HELPER_ID: &str = "ai.chariox.computer-helper";
const HEARTBEAT: Duration = Duration::from_secs(1);
const LEASE: Duration = Duration::from_secs(2);
const PAIRING: Duration = Duration::from_secs(10);
const REQUEST: Duration = Duration::from_secs(20);

type Launch = Box<dyn Fn(&Path) -> Result<(), String> + Send>;

pub(crate) struct MacComputerHelper {
    run_root: PathBuf,
    requirement: String,
    launch: Launch,
    link: Option<Arc<Link>>,
    rendezvous_guard: Arc<Mutex<()>>,
}

struct Link {
    rendezvous_guard: Arc<Mutex<()>>,
    epoch: String,
    pid: i32,
    started: Option<(u64, u64)>,
    dir: PathBuf,
    alive: AtomicBool,
    control: UnixStream,
    channel: Mutex<(BufReader<UnixStream>, u64)>,
}

impl MacComputerHelper {
    /// Feature-disabled unless the owner configures the installed helper.
    pub(crate) fn configured(root: &Path) -> Result<Option<Self>, String> {
        let Some(app) = std::env::var_os("CHARIOX_MACOS_COMPUTER_HELPER").map(PathBuf::from) else {
            return Ok(None);
        };
        if !app.is_absolute() || app.extension().is_none_or(|e| e != "app") {
            return Err("MP-11: invalid Computer helper install path".into());
        }
        // M5 pins the Developer ID team. Until then only an explicit drill
        // allowlist of one ad-hoc helper build may pair.
        let cdhash = std::env::var("CHARIOX_MACOS_COMPUTER_HELPER_DRILL_CDHASH")
            .ok()
            .filter(|h| h.len() == 40 && h.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or("MP-11: signed Computer helper identity is not configured")?;
        Ok(Some(Self::new(
            rendezvous_root(root)?,
            format!("identifier \"{HELPER_ID}\" and cdhash H\"{cdhash}\""),
            Box::new(move |dir| launch_services(&app, dir)),
        )))
    }
    fn new(run_root: PathBuf, requirement: String, launch: Launch) -> Self {
        Self {
            run_root,
            requirement,
            launch,
            link: None,
            rendezvous_guard: Arc::new(Mutex::new(())),
        }
    }

    fn pair(&mut self) -> Result<Arc<Link>, String> {
        // An old reaper must not remove the empty root between mkdirs below.
        let _rendezvous = self
            .rendezvous_guard
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // One seat per kernel home: rendezvous left by an earlier kernel is stale.
        let _ = std::fs::remove_dir_all(&self.run_root);
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.run_root)
            .map_err(|_| "MP-11: Computer helper rendezvous unavailable")?;
        let dir = self.run_root.join(format!("{:08x}", rand::random::<u32>()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir)
            .map_err(|_| "MP-11: Computer helper rendezvous unavailable")?;
        let result = self.pair_in(&dir);
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir(&self.run_root);
        }
        result
    }
    fn pair_in(&self, dir: &Path) -> Result<Arc<Link>, String> {
        let socket = dir.join("s");
        if socket.as_os_str().len() >= 104 {
            return Err("MP-11: Computer helper rendezvous path is too long".into());
        }
        let listener =
            UnixListener::bind(&socket).map_err(|_| "MP-11: Computer helper socket unavailable")?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "MP-11: Computer helper socket unavailable")?;
        let epoch = format!("{:016x}", rand::random::<u64>());
        let token = format!(
            "{:032x}{:032x}",
            rand::random::<u128>(),
            rand::random::<u128>()
        );
        let bootstrap = dir.join("bootstrap");
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&bootstrap)
            .and_then(|mut file| {
                serde_json::to_writer(
                    &mut file,
                    &json!({"token":token,"epoch":epoch,"kernel_pid":std::process::id()}),
                )?;
                Ok(())
            })
            .map_err(|_| "MP-11: Computer helper bootstrap unavailable")?;
        (self.launch)(dir)?;
        let deadline = Instant::now() + PAIRING;
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(20))
                }
                Err(_) => return Err("MP-11: Computer helper did not pair".into()),
            }
        };
        // No second connection can reach this pairing; the token is single use.
        drop(listener);
        let _ = std::fs::remove_file(&socket);
        let _ = std::fs::remove_file(&bootstrap);
        admit(
            stream,
            &self.requirement,
            &token,
            epoch,
            dir.to_path_buf(),
            self.rendezvous_guard.clone(),
        )
    }
}

/// Verifies the connected helper before reading anything it sends.
fn admit(
    stream: UnixStream,
    requirement: &str,
    token: &str,
    epoch: String,
    dir: PathBuf,
    rendezvous_guard: Arc<Mutex<()>>,
) -> Result<Arc<Link>, String> {
    const REFUSED: &str = "MP-11: Computer helper pairing refused";
    stream.set_nonblocking(false).map_err(|_| REFUSED)?;
    stream.set_read_timeout(Some(LEASE)).map_err(|_| REFUSED)?;
    stream.set_write_timeout(Some(LEASE)).map_err(|_| REFUSED)?;
    let peer = Peer::of(&stream)?;
    if peer.euid() != unsafe { libc::geteuid() } || !peer.satisfies(requirement) {
        return Err(REFUSED.into());
    }
    let mut reader = BufReader::new(stream.try_clone().map_err(|_| REFUSED)?);
    let mut line = String::new();
    (&mut reader)
        .take(4096)
        .read_line(&mut line)
        .map_err(|_| REFUSED)?;
    let hello: Value = serde_json::from_str(&line).map_err(|_| REFUSED)?;
    let echoed = hello["token"].as_str().unwrap_or_default().as_bytes();
    // Constant-time token comparison; the epoch binds this launch.
    let same = echoed.len() == token.len()
        && echoed
            .iter()
            .zip(token.as_bytes())
            .fold(0, |acc, (a, b)| acc | (a ^ b))
            == 0;
    if !same || hello["epoch"] != epoch.as_str() || hello["pid"] != peer.pid() {
        return Err(REFUSED.into());
    }
    Ok(Arc::new(Link {
        rendezvous_guard,
        epoch,
        pid: peer.pid(),
        started: code_identity::started(peer.pid()),
        dir,
        alive: AtomicBool::new(true),
        control: stream.try_clone().map_err(|_| REFUSED)?,
        channel: Mutex::new((reader, 1)),
    }))
}

/// Socket paths are limited to 104 bytes, so pair under the owner-only Darwin
/// user temp dir (not `TMPDIR`), keyed by this kernel's private state root.
fn rendezvous_root(root: &Path) -> Result<PathBuf, String> {
    use sha2::{Digest, Sha256};
    let mut buffer = [0u8; 1024];
    let length = unsafe {
        libc::confstr(
            libc::_CS_DARWIN_USER_TEMP_DIR,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    let temp = std::ffi::CStr::from_bytes_until_nul(&buffer)
        .ok()
        .filter(|_| length > 1 && length <= buffer.len())
        .and_then(|path| path.to_str().ok())
        .ok_or("MP-11: Computer helper rendezvous unavailable")?;
    let key = format!("{:x}", Sha256::digest(root.as_os_str().as_encoded_bytes()));
    Ok(Path::new(temp).join(format!("chariox-computer-{}", &key[..16])))
}

fn launch_services(app: &Path, dir: &Path) -> Result<(), String> {
    // A fresh background instance outside the kernel's process tree, so TCC
    // attributes the helper rather than the kernel or terminal.
    let status = std::process::Command::new("/usr/bin/open")
        .args(["-n", "-g", "-a"])
        .arg(app)
        .args(["--args", "--pair"])
        .arg(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|_| "MP-11: Computer helper launch failed")?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "MP-11: Computer helper launch failed".into())
}

impl Link {
    fn call(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, HostFailure> {
        if !self.alive.load(Ordering::Acquire) {
            return Err("MP-11: Computer helper is not running".into());
        }
        let mut channel = self.channel.lock().unwrap_or_else(|e| e.into_inner());
        let id = channel.1;
        channel.1 += 1;
        let request = json!({"id":id,"epoch":self.epoch,"method":method,"params":params});
        let response = match exchange(&mut channel.0, &self.control, &request, timeout) {
            Ok(response) => response,
            Err(error) => {
                self.fence();
                return Err(HostFailure::Other(error));
            }
        };
        // A reply from another pairing or request is never accepted.
        if response["id"] != id || response["epoch"] != self.epoch.as_str() {
            self.fence();
            return Err(HostFailure::Refused(UserDomainRefusalReason::StaleEpoch));
        }
        if response["ok"] == true {
            return Ok(response["result"].clone());
        }
        Err(
            match response["error"]["code"]
                .as_str()
                .and_then(UserDomainRefusalReason::from_code)
            {
                Some(reason) => HostFailure::Refused(reason),
                // Helper text is untrusted; only the kernel's fixed message leaves.
                None => HostFailure::Other("MP-11: macOS Computer helper refused".into()),
            },
        )
    }
    fn fence(&self) {
        self.alive.store(false, Ordering::Release);
        let _ = self.control.shutdown(std::net::Shutdown::Both);
    }
    /// Owned cleanup: fence, let the helper exit, kill only the paired process.
    fn retire(&self) {
        self.fence();
        let deadline = Instant::now() + LEASE;
        let alive = || {
            self.pid != std::process::id() as i32
                && self.started.is_some()
                && code_identity::started(self.pid) == self.started
        };
        while alive() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        if alive() {
            unsafe { libc::kill(self.pid, libc::SIGKILL) };
        }
        let _rendezvous = self
            .rendezvous_guard
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _ = std::fs::remove_dir_all(&self.dir);
        // The per-kernel root goes too unless a newer pairing already uses it.
        let _ = self.dir.parent().map(std::fs::remove_dir);
    }
}

fn exchange(
    reader: &mut BufReader<UnixStream>,
    mut writer: &UnixStream,
    request: &Value,
    timeout: Duration,
) -> Result<Value, String> {
    const LOST: &str = "MP-11: Computer helper disconnected";
    reader
        .get_ref()
        .set_read_timeout(Some(timeout))
        .map_err(|_| LOST)?;
    let mut line = serde_json::to_vec(request).map_err(|_| LOST)?;
    line.push(b'\n');
    writer.write_all(&line).map_err(|_| LOST)?;
    let mut line = String::new();
    reader
        .take(1 << 20)
        .read_line(&mut line)
        .map_err(|_| LOST)?;
    if !line.ends_with('\n') {
        return Err(LOST.into());
    }
    serde_json::from_str(&line).map_err(|_| LOST.into())
}

fn watch(link: Weak<Link>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(HEARTBEAT);
        let Some(link) = link.upgrade() else { return };
        if !link.alive.load(Ordering::Acquire) {
            return;
        }
        if link.call("heartbeat", json!({}), LEASE).is_err() {
            tracing::warn!("MP-11: macOS Computer helper lease ended; seat fenced");
            link.retire();
            return;
        }
    });
}

impl ComputerBackend for MacComputerHelper {
    fn ready(&mut self) -> bool {
        if self
            .link
            .as_ref()
            .is_some_and(|l| !l.alive.load(Ordering::Acquire))
        {
            self.link.take().unwrap().retire();
        }
        self.link.is_some()
    }
    fn start(&mut self) -> Result<(), String> {
        if self.ready() {
            return Ok(());
        }
        let link = self.pair()?;
        watch(Arc::downgrade(&link));
        self.link = Some(link);
        Ok(())
    }
    fn stop(&mut self) -> Result<(), String> {
        if let Some(link) = self.link.take() {
            let _ = link.call("stop", json!({}), LEASE);
            // End this epoch under the seat lock. Only reaping may outlive Stop.
            link.fence();
            std::thread::spawn(move || link.retire());
        }
        Ok(())
    }
    fn request(
        &mut self,
        method: &str,
        params: Value,
        cancellation: Option<Arc<BrowserCancellation>>,
    ) -> Result<Value, HostFailure> {
        if cancellation.is_some_and(|c| c.requested()) {
            return Err("MP-11: Computer action cancelled before dispatch".into());
        }
        if !self.ready() {
            return Err("MP-11: browser_unavailable: explicitly start the Computer seat".into());
        }
        let link = self.link.as_ref().unwrap();
        let result = link.call(method, params, REQUEST)?;
        // The desktop generation is the kernel's pairing epoch, never helper-chosen.
        if result
            .get("generation")
            .is_some_and(|g| g != link.epoch.as_str())
        {
            link.fence();
            return Err(HostFailure::Refused(UserDomainRefusalReason::StaleEpoch));
        }
        Ok(result)
    }
}

impl Drop for MacComputerHelper {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
