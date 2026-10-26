//! L3 stage 2b (AUDIT2 §4): the worker side of a kernel display lease. Node
//! grants a lease with one of its published frames; the codec thread then sends
//! the following whole frames on the kernel's frame channel and those readbacks
//! never reach Node. MP-11: every retire (Node's retire pipe, the kernel's
//! `retire`, the record's expiry) is applied under this one lock right before
//! each send, so no frame read back after a retire leaves the worker.
use super::worker::Auto;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::io::Write;
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Frames sent and not yet delivered (the kernel queues at most four).
const IN_FLIGHT: u64 = 2;
/// No damage this long ends a lease, so Node's refiner repairs the frame.
const QUIET: Duration = Duration::from_millis(120);
/// A sent frame the kernel never takes ends the lease.
const STALL: Duration = Duration::from_secs(1);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Grant {
    generation: u64,
    /// The display sequence of the frame Node published with this grant.
    sequence: u64,
    /// That frame's codec row sequence: the reference the client holds.
    chain: u64,
    template: Map<String, Value>,
}

struct Lease {
    generation: u64,
    next: u64,
    chain: u64,
    template: Map<String, Value>,
    sent: u64,
    delivered: u64,
    since: Instant,
}

pub(super) struct Direct {
    pub(super) auto: Auto,
    lease: Option<Lease>,
    link: Option<UnixStream>,
    retires: RawFd,
    partial: Vec<u8>,
    incoming: Vec<u8>,
    root: PathBuf,
    /// Lease generations that ended; the main loop tells Node, then reads back for it.
    pub(super) ended: Vec<u64>,
}

impl Direct {
    /// `retires` is Node's synchronous retire pipe (non-blocking), or -1: no records.
    pub(super) fn new(retires: RawFd, link: Option<UnixStream>, root: PathBuf) -> Self {
        Self {
            auto: Auto::default(),
            lease: None,
            link,
            retires,
            partial: Vec::new(),
            incoming: Vec::new(),
            root,
            ended: Vec::new(),
        }
    }

    pub(super) fn fds(&self) -> [RawFd; 2] {
        [
            self.retires,
            self.link.as_ref().map_or(-1, |l| l.as_raw_fd()),
        ]
    }

    /// Apply every retire Node wrote so far. Node closing the pipe retires all.
    pub(super) fn drain(&mut self) {
        if self.retires < 0 {
            self.auto.retire(u64::MAX);
            return self.end();
        }
        let mut bytes = [0u8; 512];
        loop {
            let read = unsafe { libc::read(self.retires, bytes.as_mut_ptr().cast(), bytes.len()) };
            if read > 0 {
                self.partial.extend_from_slice(&bytes[..read as usize]);
                continue;
            }
            if read == 0 || std::io::Error::last_os_error().kind() != std::io::ErrorKind::WouldBlock
            {
                unsafe { libc::close(self.retires) };
                self.retires = -1;
                self.partial.clear();
                self.auto.retire(u64::MAX);
                return self.end();
            }
            break;
        }
        while let Some(end) = self.partial.iter().position(|b| *b == b'\n') {
            let generation = std::str::from_utf8(&self.partial[..end])
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(u64::MAX);
            self.partial.drain(..=end);
            self.retire(generation);
        }
        if self.partial.len() > 64 {
            self.partial.clear();
            self.retire(u64::MAX);
        }
    }

    pub(super) fn retire(&mut self, generation: u64) {
        self.auto.retire(generation);
        if self
            .lease
            .as_ref()
            .is_some_and(|l| l.generation <= generation)
        {
            self.end();
        }
    }

    /// End the lease: the kernel refuses its generation and Node resumes.
    pub(super) fn end(&mut self) {
        if let Some(lease) = self.lease.take() {
            self.auto.retire(lease.generation);
            self.tell(json!({ "end": lease.generation }));
            self.ended.push(lease.generation);
        }
    }

    fn tell(&mut self, message: Value) {
        if let Some(link) = &mut self.link {
            if link.write_all(format!("{message}\n").as_bytes()).is_err() {
                self.link = None;
            }
        }
    }

    /// Node's grant, valid only for the current record and a connected channel.
    pub(super) fn grant(&mut self, grant: Grant, now: f64) -> Result<(), String> {
        if grant.template.get("kind") != Some(&json!("video"))
            || grant.template.contains_key("protected_regions")
        {
            return Err("MP-11: native lease template".into());
        }
        self.drain();
        if self.link.is_none()
            || self.lease.is_some()
            || self.auto.current(now) != Some(grant.generation)
        {
            self.tell(json!({ "end": grant.generation }));
            self.ended.push(grant.generation);
            return Ok(());
        }
        self.lease = Some(Lease {
            generation: grant.generation,
            next: grant.sequence + 1,
            chain: grant.chain,
            template: grant.template,
            sent: 0,
            delivered: 0,
            since: Instant::now(),
        });
        Ok(())
    }

    /// The generation leased for this readback, or none (Node gets it).
    pub(super) fn leased(&mut self, now: f64) -> Option<u64> {
        let generation = self.lease.as_ref()?.generation;
        if self.auto.current(now) != Some(generation) {
            self.end();
            return None;
        }
        Some(generation)
    }

    /// Credit pacing: hold readbacks while the kernel has frames to take.
    pub(super) fn hold(&mut self, idle: Duration) -> bool {
        let Some(lease) = &self.lease else {
            return false;
        };
        let waiting = lease.sent - lease.delivered;
        if (waiting == 0 && idle > QUIET) || (waiting > 0 && lease.since.elapsed() > STALL) {
            self.end();
            return false;
        }
        waiting >= IN_FLIGHT
    }

    /// The first leased delta must follow the packet the client holds.
    pub(super) fn needs_key(&self, last: Option<u64>) -> bool {
        self.lease.as_ref().is_some_and(|l| last != Some(l.chain))
    }

    /// Kernel messages: `retire` ends the lease, `delivered` returns credit.
    /// Returns the delivered revisions; a closed channel ends the lease.
    pub(super) fn receive(&mut self) -> Vec<u64> {
        let mut delivered = Vec::new();
        let Some(link) = &self.link else {
            return delivered;
        };
        let mut bytes = [0u8; 4096];
        let read = unsafe {
            libc::recv(
                link.as_raw_fd(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
                libc::MSG_DONTWAIT,
            )
        };
        if read == 0
            || (read < 0
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::WouldBlock)
        {
            self.link = None;
            if let Some(lease) = self.lease.take() {
                self.auto.retire(lease.generation);
                self.ended.push(lease.generation);
            }
            return delivered;
        }
        if read < 0 {
            return delivered;
        }
        self.incoming.extend_from_slice(&bytes[..read as usize]);
        while let Some(end) = self.incoming.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.incoming.drain(..=end).collect();
            let Ok(message) = serde_json::from_slice::<Value>(&line) else {
                continue;
            };
            if let Some(generation) = message["retire"].as_u64() {
                self.retire(generation);
            } else if let (Some(revision), Some(lease)) =
                (message["delivered"].as_u64(), self.lease.as_mut())
            {
                lease.delivered = (lease.delivered + 1).min(lease.sent);
                lease.since = Instant::now();
                delivered.push(revision);
            }
        }
        if self.incoming.len() > 4096 {
            self.incoming.clear();
        }
        delivered
    }

    /// The codec thread's send, after the encode: refused unless the lease,
    /// its record and the codec chain still hold right now.
    pub(super) fn send(&mut self, generation: u64, value: &Value, now: f64) -> bool {
        self.drain();
        let row = &value["stripes"][0];
        let (Some(sequence), Some(packet)) = (row["sequence"].as_u64(), value.get("packet")) else {
            self.end();
            return false;
        };
        let chained = row["key"] == true
            || row["reference_sequence"].as_u64() == self.lease.as_ref().map(|l| l.chain);
        if self.leased(now) != Some(generation) || !packet.is_object() || !chained {
            self.end();
            return false;
        }
        let lease = self.lease.as_mut().unwrap();
        let mut frame = lease.template.clone();
        frame.insert("key".into(), row["key"].clone());
        frame.insert("sequence".into(), json!(lease.next));
        frame.insert("native_packet".into(), packet.clone());
        let message = json!({"frame":frame,"state_generation":generation,"revision":value["revision"],"unprotected":true});
        lease.next += 1;
        lease.chain = sequence;
        lease.sent += 1;
        if lease.sent - lease.delivered == 1 {
            lease.since = Instant::now();
        }
        self.tell(message);
        if self.link.is_none() {
            self.lease = None;
            self.auto.retire(generation);
            self.ended.push(generation);
        }
        true
    }

    /// A leased packet nobody will send; the kernel never names it.
    pub(super) fn discard(&self, value: &Value) {
        if let Some(name) = value.pointer("/packet/name").and_then(Value::as_str) {
            if name.len() == 37
                && name.ends_with(".json")
                && name[..32]
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            {
                let _ = std::fs::remove_file(self.root.join(name));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    // Node's stdio "pipe" is a socketpair.
    fn pipe() -> (RawFd, RawFd) {
        let mut fds = [0; 2];
        let kind = libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC;
        assert_eq!(
            unsafe { libc::socketpair(libc::AF_UNIX, kind, 0, fds.as_mut_ptr()) },
            0
        );
        (fds[0], fds[1])
    }

    fn leased() -> (Direct, RawFd, BufReader<UnixStream>) {
        let (retires, write) = pipe();
        let (worker, kernel) = UnixStream::pair().unwrap();
        let mut direct = Direct::new(retires, Some(worker), PathBuf::from("/nonexistent"));
        direct
            .auto
            .arm(serde_json::from_value(json!({"generation":2,"encoder":"motion","bitrate":8000000,"reduced":false,"until_ms":1e15})).unwrap())
            .unwrap();
        let template = json!({"kind":"video","subscription_id":"s"})
            .as_object()
            .unwrap()
            .clone();
        direct
            .grant(
                Grant {
                    generation: 2,
                    sequence: 10,
                    chain: 40,
                    template,
                },
                1.,
            )
            .unwrap();
        assert_eq!(direct.leased(1.), Some(2));
        (direct, write, BufReader::new(kernel))
    }

    fn encoded(sequence: u64, reference: u64) -> Value {
        json!({"stripes":[{"key":false,"sequence":sequence,"reference_sequence":reference}],"packet":{"name":"p"},"revision":sequence})
    }

    fn line(kernel: &mut BufReader<UnixStream>) -> Value {
        let mut line = String::new();
        kernel.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    // MP-11: a retire Node wrote before the send holds, however late the encode.
    #[test]
    fn mp11_no_frame_after_a_retire_leaves_the_worker_even_one_readback_later() {
        let (mut direct, write, mut kernel) = leased();
        assert!(direct.send(2, &encoded(41, 40), 1.));
        let frame = line(&mut kernel);
        assert_eq!(
            (
                &frame["frame"]["sequence"],
                &frame["frame"]["kind"],
                &frame["state_generation"],
                &frame["unprotected"]
            ),
            (&json!(11), &json!("video"), &json!(2), &json!(true))
        );
        // The main loop is inside its next readback when Node retires.
        assert_eq!(unsafe { libc::write(write, b"2\n".as_ptr().cast(), 2) }, 2);
        assert!(
            !direct.send(2, &encoded(42, 41), 1.),
            "the codec thread checks the pipe itself"
        );
        assert_eq!(line(&mut kernel), json!({"end":2}));
        assert_eq!(direct.ended, vec![2]);
        assert!(direct.leased(1.).is_none() && direct.auto.current(1.).is_none());
        unsafe { libc::close(write) };
    }

    // A dropped encode (no packet: an oversized recovery key) is not a frame: the lease ends so
    // Node's exact fallback gets a fresh readback instead of the viewer freezing on a held lease.
    #[test]
    fn a_dropped_encode_ends_the_lease_for_node() {
        let (mut direct, write, mut kernel) = leased();
        assert!(!direct.send(2, &json!({"dropped":true,"revision":41}), 1.));
        assert_eq!(line(&mut kernel), json!({"end":2}));
        assert_eq!(direct.ended, vec![2]);
        assert!(direct.leased(1.).is_none());
        unsafe { libc::close(write) };
    }

    // MP-11: the kernel's retire, record expiry, a closed pipe and a chain break end it too.
    #[test]
    fn mp11_lease_ends_on_kernel_retire_expiry_closed_pipe_and_chain_break() {
        let (mut direct, write, mut kernel) = leased();
        kernel.get_mut().write_all(b"{\"retire\":2}\n").unwrap();
        while direct.lease.is_some() {
            direct.receive();
        }
        assert!(!direct.send(2, &encoded(41, 40), 1.));
        assert_eq!(line(&mut kernel), json!({"end":2}));

        let (mut direct, _write2, _kernel) = leased();
        assert!(
            direct.leased(2e15).is_none(),
            "an expired verdict ends the lease"
        );

        let (mut direct, write3, _kernel) = leased();
        unsafe { libc::close(write3) };
        // A concurrent test's fork may hold the write end until its exec.
        let mut hup = libc::pollfd {
            fd: direct.retires,
            events: libc::POLLIN,
            revents: 0,
        };
        assert_eq!(unsafe { libc::poll(&mut hup, 1, 5000) }, 1);
        assert!(
            !direct.send(2, &encoded(41, 40), 1.),
            "Node gone: retire all"
        );

        let (mut direct, _write4, _kernel) = leased();
        assert!(direct.needs_key(Some(39)) && !direct.needs_key(Some(40)));
        assert!(
            !direct.send(2, &encoded(41, 39), 1.),
            "a delta off the client's chain"
        );
        unsafe { libc::close(write) };
    }

    // Credit pacing and a grant that cannot hold.
    #[test]
    fn lease_paces_to_delivered_credit_and_refuses_stale_grants() {
        let (mut direct, write, mut kernel) = leased();
        assert!(direct.send(2, &encoded(41, 40), 1.) && direct.send(2, &encoded(42, 41), 1.));
        assert!(
            direct.hold(Duration::ZERO),
            "two frames in flight hold readbacks"
        );
        kernel.get_mut().write_all(b"{\"delivered\":41}\n").unwrap();
        while direct.receive().is_empty() {}
        assert!(!direct.hold(Duration::ZERO));
        let template = json!({"kind":"video"}).as_object().unwrap().clone();
        direct.end();
        direct.ended.clear();
        direct
            .grant(
                Grant {
                    generation: 2,
                    sequence: 1,
                    chain: 1,
                    template: template.clone(),
                },
                1.,
            )
            .unwrap();
        assert!(
            direct.lease.is_none() && direct.ended == vec![2],
            "a retired generation is not leased again"
        );
        let masked = json!({"kind":"video","protected_regions":[]})
            .as_object()
            .unwrap()
            .clone();
        assert!(direct
            .grant(
                Grant {
                    generation: 3,
                    sequence: 1,
                    chain: 1,
                    template: masked
                },
                1.
            )
            .is_err());
        unsafe { libc::close(write) };
    }
}
