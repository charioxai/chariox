//! L3 (AUDIT2 §4): whole motion frames go from the native worker straight to the kernel pump.
//! Node grants a display lease (token, state generation, next sequence) in an ordinary frame
//! reply; until it ends, the pump takes that lease's frames from the worker's connection.
//! MP-11: one connection per token, only from a process the kernel's own controller spawned;
//! the kernel refuses every frame of a generation at or below the latest retire, every frame
//! not marked unprotected, and every break in the lease's sequence chain.
use super::display_packets::DisplayPackets;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const QUEUE: usize = 4;

pub(crate) struct DirectFrames {
    path: PathBuf,
    packets: Arc<DisplayPackets>,
    controller: AtomicU32,
    links: Mutex<HashMap<String, Arc<Link>>>,
    accept: Mutex<Option<std::thread::JoinHandle<()>>>,
    #[cfg(test)]
    listener: i32,
}

pub(crate) struct Link {
    /// Dropped when the worker disconnects; the link itself stays as the spent-token record.
    writer: Mutex<Option<UnixStream>>,
    state: Mutex<LinkState>,
    wake: tokio::sync::Notify,
}

#[derive(Default)]
struct LinkState {
    lease: Option<Lease>,
    retired: u64,
    frames: VecDeque<(Value, u64, u64)>,
    closed: bool,
}

struct Lease {
    subscription: String,
    generation: u64,
    next: u64,
}

#[derive(Debug)]
pub(crate) enum Next {
    /// A hydrated display frame and the worker revision to report delivered.
    Frame(Value, u64),
    Idle,
    Ended,
}

impl DirectFrames {
    /// Linux only: admission needs the peer's credentials. Elsewhere there is no
    /// channel and Node publishes every frame.
    pub(super) fn listen(packets: Arc<DisplayPackets>) -> Result<Arc<Self>, String> {
        if !cfg!(target_os = "linux") {
            return Err("MP-11: native frame channel unavailable".into());
        }
        let path = packets.root().join("frames.sock");
        let listener = UnixListener::bind(&path).map_err(|_| "MP-11: native frame channel")?;
        let frames = Arc::new(Self {
            path,
            packets,
            controller: AtomicU32::new(0),
            links: Mutex::new(HashMap::new()),
            accept: Mutex::new(None),
            #[cfg(test)]
            listener: std::os::fd::AsRawFd::as_raw_fd(&listener),
        });
        let weak = Arc::downgrade(&frames);
        let accept = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Some(frames) = weak.upgrade() else { return };
                if let Ok(stream) = stream {
                    frames.admit(stream);
                }
            }
        });
        *frames.accept.lock().unwrap_or_else(|e| e.into_inner()) = Some(accept);
        Ok(frames)
    }

    pub(super) fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// The controller the kernel spawned; only its children may connect.
    pub(super) fn bind_controller(&self, pid: u32) {
        self.controller.store(pid, Ordering::SeqCst);
    }

    fn admit(self: &Arc<Self>, stream: UnixStream) {
        let controller = self.controller.load(Ordering::SeqCst);
        if controller <= 1 || peer_parent(&stream) != Some(controller) {
            return;
        }
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let Ok(reader) = stream.try_clone() else {
            return;
        };
        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        if (&mut reader).take(4096).read_line(&mut line).is_err() {
            return;
        }
        let token = match serde_json::from_str::<Value>(&line) {
            Ok(Value::Object(m)) if m.len() == 1 => {
                m.get("hello").and_then(Value::as_str).map(str::to_owned)
            }
            _ => None,
        };
        let Some(token) =
            token.filter(|t| t.len() == 32 && t.bytes().all(|b| b.is_ascii_hexdigit()))
        else {
            return;
        };
        let Ok(writer) = stream.try_clone() else {
            return;
        };
        let link = Arc::new(Link {
            writer: Mutex::new(Some(writer)),
            state: Mutex::new(LinkState::default()),
            wake: tokio::sync::Notify::new(),
        });
        {
            let Ok(mut links) = self.links.lock() else {
                return;
            };
            // A token is spent by its first connection, even after that one closes.
            if links.contains_key(&token) {
                return;
            }
            links.insert(token, link.clone());
        }
        let _ = stream.set_read_timeout(None);
        let frames = self.clone();
        std::thread::spawn(move || {
            loop {
                line.clear();
                match (&mut reader).take(64 * 1024).read_line(&mut line) {
                    Ok(n) if n > 0 && line.ends_with('\n') => {}
                    _ => break,
                }
                if !frames.receive(&link, &line) {
                    break;
                }
            }
            *link.writer.lock().unwrap_or_else(|e| e.into_inner()) = None;
            let mut state = link.lock();
            state.closed = true;
            state.lease = None;
            for (frame, _, _) in state.frames.drain(..) {
                frames.discard(&frame);
            }
            drop(state);
            link.wake.notify_one();
        });
    }

    fn receive(&self, link: &Link, line: &str) -> bool {
        let Ok(Value::Object(message)) = serde_json::from_str::<Value>(line) else {
            return false;
        };
        let mut state = link.lock();
        if let Some(generation) = message.get("end").and_then(Value::as_u64) {
            state.retired = state.retired.max(generation);
            Self::end_retired(self, &mut state);
        } else if let (Some(frame), Some(generation), Some(revision)) = (
            message.get("frame"),
            message.get("state_generation").and_then(Value::as_u64),
            message.get("revision").and_then(Value::as_u64),
        ) {
            // MP-11: only a whole video frame the worker encoded under an unprotected record.
            let admitted = message.get("unprotected") == Some(&Value::Bool(true))
                && frame["kind"] == "video"
                && frame.get("protected_regions").is_none()
                && generation > state.retired
                && state.frames.len() < QUEUE;
            if !admitted {
                self.discard(frame);
                return true;
            }
            state
                .frames
                .push_back((frame.clone(), generation, revision));
        } else {
            return false;
        }
        drop(state);
        link.wake.notify_one();
        true
    }

    fn end_retired(&self, state: &mut LinkState) {
        let retired = state.retired;
        if state
            .lease
            .as_ref()
            .is_some_and(|l| l.generation <= retired)
        {
            state.lease = None;
        }
        let kept: VecDeque<_> = state
            .frames
            .drain(..)
            .filter(|f| {
                f.1 > retired || {
                    self.discard(&f.0);
                    false
                }
            })
            .collect();
        state.frames = kept;
    }

    fn discard(&self, frame: &Value) {
        if let Some(name) = frame.pointer("/native_packet/name").and_then(Value::as_str) {
            self.packets.discard(name);
        }
    }

    fn link(&self, token: &str) -> Option<Arc<Link>> {
        self.links.lock().ok()?.get(token).cloned()
    }

    /// Bind a lease Node granted with the frame it just returned (`sequence`).
    pub(crate) fn bind(
        &self,
        token: &str,
        subscription: &str,
        generation: u64,
        sequence: u64,
    ) -> bool {
        let Some(link) = self.link(token) else {
            return false;
        };
        let mut state = link.lock();
        if state.closed || generation <= state.retired || state.lease.is_some() {
            // The worker took Node's grant too: refuse it there as well.
            state.retired = state.retired.max(generation);
            Self::end_retired(self, &mut state);
            drop(state);
            link.send(json!({ "retire": generation }));
            return false;
        }
        state.lease = Some(Lease {
            subscription: subscription.into(),
            generation,
            next: sequence + 1,
        });
        true
    }

    /// The next frame of the lease, or Idle after `budget`, or Ended once it is gone.
    pub(crate) async fn next(&self, token: &str, subscription: &str, budget: Duration) -> Next {
        let Some(link) = self.link(token) else {
            return Next::Ended;
        };
        let deadline = tokio::time::Instant::now() + budget;
        loop {
            let wake = link.wake.notified();
            match self.take(&link, token, subscription) {
                Some(next) => return next,
                None if tokio::time::timeout_at(deadline, wake).await.is_err() => {
                    return Next::Idle
                }
                None => {}
            }
        }
    }

    fn take(&self, link: &Link, token: &str, subscription: &str) -> Option<Next> {
        let mut state = link.lock();
        loop {
            let Some(lease) = state
                .lease
                .as_ref()
                .filter(|l| l.subscription == subscription)
            else {
                return Some(Next::Ended);
            };
            let (generation, next) = (lease.generation, lease.next);
            let (frame, frame_generation, revision) = state.frames.pop_front()?;
            if frame_generation != generation || frame_generation <= state.retired {
                self.discard(&frame);
                continue;
            }
            if frame["subscription_id"] != subscription || frame["sequence"].as_u64() != Some(next)
            {
                self.discard(&frame);
                drop(state);
                self.retire(token);
                return Some(Next::Ended);
            }
            let mut result = json!({ "display_frame": frame });
            if self.packets.hydrate(&mut result).is_err() {
                drop(state);
                self.retire(token);
                return Some(Next::Ended);
            }
            if let Some(lease) = state.lease.as_mut() {
                lease.next += 1;
            }
            return Some(Next::Frame(result["display_frame"].take(), revision));
        }
    }

    /// The kernel ends the lease: later frames of its generation are refused.
    pub(crate) fn retire(&self, token: &str) {
        let Some(link) = self.link(token) else { return };
        let mut state = link.lock();
        let Some(generation) = state.lease.as_ref().map(|l| l.generation) else {
            return;
        };
        state.retired = state.retired.max(generation);
        self.end_retired(&mut state);
        drop(state);
        link.send(json!({ "retire": generation }));
    }

    pub(crate) fn delivered(&self, token: &str, revision: u64) {
        if let Some(link) = self.link(token) {
            link.send(json!({ "delivered": revision }));
        }
    }
}

impl Link {
    fn lock(&self) -> std::sync::MutexGuard<'_, LinkState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn send(&self, message: Value) {
        if let Ok(mut writer) = self.writer.lock() {
            if let Some(writer) = writer.as_mut() {
                let _ = writer.write_all(format!("{message}\n").as_bytes());
            }
        }
    }
}

impl Drop for DirectFrames {
    /// Wake the accept loop (it then finds no channel and closes the listener) and join it.
    fn drop(&mut self) {
        let _ = UnixStream::connect(&self.path);
        let _ = std::fs::remove_file(&self.path);
        let accept = self
            .accept
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(accept) = accept.filter(|a| a.thread().id() != std::thread::current().id()) {
            let _ = accept.join();
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn peer_parent(_: &UnixStream) -> Option<u32> {
    None
}

/// MP-11: the connecting process's parent, from the kernel's own socket credentials.
#[cfg(target_os = "linux")]
fn peer_parent(stream: &UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let status = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut length,
        )
    };
    if status != 0 || credentials.pid <= 1 || credentials.uid != unsafe { libc::geteuid() } {
        return None;
    }
    let stat = std::fs::read_to_string(format!("/proc/{}/stat", credentials.pid)).ok()?;
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

#[cfg(all(test, not(target_os = "linux")))]
mod unavailable_tests {
    use super::*;

    // Without peer credentials there is no channel: Node publishes every frame.
    #[test]
    fn channel_is_unavailable_off_linux() {
        let packets = Arc::new(DisplayPackets::create().unwrap());
        assert!(DirectFrames::listen(packets).is_err());
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    fn channel() -> (Arc<DirectFrames>, Arc<DisplayPackets>) {
        let packets = Arc::new(DisplayPackets::create().unwrap());
        let frames = DirectFrames::listen(packets.clone()).unwrap();
        (frames, packets)
    }

    // A stopped controller leaves no listener thread or socket descriptor behind.
    #[test]
    fn dropping_the_channel_closes_its_listener() {
        let socket = |fd: i32| std::fs::read_link(format!("/proc/self/fd/{fd}")).ok();
        for _ in 0..3 {
            let (frames, _packets) = channel();
            let (fd, open) = (frames.listener, socket(frames.listener));
            assert!(open
                .as_ref()
                .is_some_and(|l| l.to_string_lossy().starts_with("socket:")));
            drop(frames);
            assert_ne!(
                socket(fd),
                open,
                "the listener's socket is closed (its fd may be reused)"
            );
        }
    }

    fn connect(frames: &DirectFrames, token: &str) -> UnixStream {
        let mut stream = UnixStream::connect(frames.path()).unwrap();
        // A refused connection may already be closed by the kernel.
        let _ = stream.write_all(
            format!(
                "{}
",
                json!({ "hello": token })
            )
            .as_bytes(),
        );
        stream
    }

    fn wait_link(frames: &DirectFrames, token: &str) -> bool {
        (0..200).any(|_| {
            std::thread::sleep(Duration::from_millis(5));
            frames.link(token).is_some()
        })
    }

    fn frame(stream: &mut UnixStream, sequence: u64, generation: u64, unprotected: bool) {
        let frame = json!({"kind":"video","subscription_id":"s","sequence":sequence});
        let message = json!({"frame":frame,"state_generation":generation,"revision":sequence,"unprotected":unprotected});
        stream.write_all(format!("{message}\n").as_bytes()).unwrap();
    }

    async fn settle(frames: &DirectFrames, token: &str) -> Next {
        frames.next(token, "s", Duration::from_millis(200)).await
    }

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn mp11_direct_frames_admit_one_connection_per_token_from_the_controller_only() {
        let (frames, _packets) = channel();
        // Not yet bound to a controller: nobody is admitted.
        let _early = connect(&frames, TOKEN);
        assert!(!wait_link(&frames, TOKEN));
        // A process whose parent is not the controller is refused.
        frames.bind_controller(std::process::id());
        let _foreign = connect(&frames, TOKEN);
        assert!(!wait_link(&frames, TOKEN));
        frames.bind_controller(std::os::unix::process::parent_id());
        let first = connect(&frames, TOKEN);
        assert!(wait_link(&frames, TOKEN));
        let first_link = frames.link(TOKEN).unwrap();
        let mut second = connect(&frames, TOKEN);
        std::thread::sleep(Duration::from_millis(50));
        assert!(Arc::ptr_eq(&first_link, &frames.link(TOKEN).unwrap()));
        // The refused second connection is closed by the kernel.
        let mut byte = [0u8; 1];
        second
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        assert_eq!(second.read(&mut byte).unwrap(), 0);
        drop(first);
        let _again = connect(&frames, TOKEN);
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            Arc::ptr_eq(&first_link, &frames.link(TOKEN).unwrap()),
            "a spent token stays spent"
        );
    }

    #[tokio::test]
    async fn mp11_direct_frames_refuse_stale_generations_and_hold_a_retire_before_publish() {
        let (frames, _packets) = channel();
        frames.bind_controller(std::os::unix::process::parent_id());
        let mut worker = connect(&frames, TOKEN);
        assert!(wait_link(&frames, TOKEN));
        assert!(frames.bind(TOKEN, "s", 3, 10));
        // A frame of an older generation than the lease is refused.
        frame(&mut worker, 11, 2, true);
        assert!(matches!(settle(&frames, TOKEN).await, Next::Idle));
        frame(&mut worker, 11, 3, true);
        assert!(matches!(settle(&frames, TOKEN).await, Next::Frame(f, 11) if f["sequence"] == 11));
        // Queued before the kernel retired it: never published.
        frame(&mut worker, 12, 3, true);
        std::thread::sleep(Duration::from_millis(50));
        frames.retire(TOKEN);
        assert!(matches!(settle(&frames, TOKEN).await, Next::Ended));
        let mut reply = String::new();
        BufReader::new(worker.try_clone().unwrap())
            .read_line(&mut reply)
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&reply).unwrap(),
            json!({"retire":3})
        );
        // A retired generation cannot be leased again; a worker-side end retires too.
        assert!(!frames.bind(TOKEN, "s", 3, 12));
        assert!(frames.bind(TOKEN, "s", 4, 12));
        worker.write_all(b"{\"end\":4}\n").unwrap();
        frame(&mut worker, 13, 4, true);
        assert!(matches!(settle(&frames, TOKEN).await, Next::Ended));
    }

    #[tokio::test]
    async fn mp11_direct_frames_never_pass_protected_or_out_of_chain_frames() {
        let (frames, _packets) = channel();
        frames.bind_controller(std::os::unix::process::parent_id());
        let mut worker = connect(&frames, TOKEN);
        assert!(wait_link(&frames, TOKEN));
        assert!(frames.bind(TOKEN, "s", 5, 0));
        frame(&mut worker, 1, 5, false);
        let masked = json!({"frame":{"kind":"video","subscription_id":"s","sequence":1,"protected_regions":[]},"state_generation":5,"revision":1,"unprotected":true});
        worker.write_all(format!("{masked}\n").as_bytes()).unwrap();
        let tiles = json!({"frame":{"kind":"tiles","subscription_id":"s","sequence":1},"state_generation":5,"revision":1,"unprotected":true});
        worker.write_all(format!("{tiles}\n").as_bytes()).unwrap();
        assert!(matches!(settle(&frames, TOKEN).await, Next::Idle));
        // A gap in the sequence chain ends the lease.
        frame(&mut worker, 2, 5, true);
        assert!(matches!(settle(&frames, TOKEN).await, Next::Ended));
        assert!(!frames.bind(TOKEN, "s", 5, 2));
    }

    #[test]
    fn mp11_direct_frames_release_the_writer_when_the_worker_disconnects() {
        let fds = || std::fs::read_dir("/proc/self/fd").unwrap().count();
        let (frames, _packets) = channel();
        frames.bind_controller(std::os::unix::process::parent_id());
        let before = fds();
        for n in 0..24u32 {
            let token = format!("{n:032x}");
            let worker = connect(&frames, &token);
            assert!(wait_link(&frames, &token));
            drop(worker);
            let closed = (0..400).any(|_| {
                std::thread::sleep(Duration::from_millis(5));
                frames.link(&token).is_some_and(|l| l.lock().closed)
            });
            assert!(closed, "the worker's EOF closes its link");
        }
        // The spent-token record stays, so a reused token is still refused.
        let again = connect(&frames, &format!("{:032x}", 0));
        std::thread::sleep(Duration::from_millis(50));
        drop(again);
        assert!(
            fds() <= before + 2,
            "worker disconnects leaked {} descriptors",
            fds() - before
        );
    }
}
