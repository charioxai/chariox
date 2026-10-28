//! Run with scripts/drill.sh, which builds the source guest and real worker.
use chariox_app_runtime::wire::{Channel, Message, Outcome, Sender, Success, WIRE_VERSION};
use chariox_app_wasm_worker::ipc::wall_ms;
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::process::CommandExt,
    },
    path::PathBuf,
    process::{Child, Command},
    time::Duration,
};
use tokio::net::UnixStream;

pub(super) struct Worker {
    child: Child,
    pub(super) channel: Option<Channel<UnixStream>>,
    directory: PathBuf,
}
impl Worker {
    pub(super) async fn start(retries: Vec<&str>) -> Self {
        let root =
            PathBuf::from(std::env::var("CHARIOX_WORKER_DRILL_DIR").expect("run scripts/drill.sh"));
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let directory = root.join(format!(
            "worker-{}-{}-{unique}",
            std::process::id(),
            wall_ms()
        ));
        fs::create_dir_all(&directory).unwrap();
        let core =
            fs::read(std::env::var("CHARIOX_WORKER_DRILL_CORE").expect("build source guest first"))
                .unwrap();
        let bytes = wit_component::ComponentEncoder::default()
            .module(&core)
            .unwrap()
            .validate(true)
            .encode()
            .unwrap();
        chariox_app_wit::check_component(&bytes, chariox_app_wit::APP_WORLD).unwrap();
        let component = directory.join("app.wasm");
        fs::write(&component, bytes).unwrap();
        let fixture: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../app-package/tests/vectors/v2/package.json"
        )))
        .unwrap();
        let mut manifest = fixture["shared"]["manifest"].clone();
        manifest["idempotentTools"] = json!(retries);
        let launch = directory.join("launch.json");
        fs::write(&launch,json!({"component":component,"generation":"1","installation_id":"worker-drill","release_digest":format!("sha256:{}","0".repeat(64)),"manifest":manifest,"limits":{"memory_bytes":16<<20,"slice_ms":100,"call_ms":30000,"cancel_grace_ms":50}}).to_string()).unwrap();
        let (supervisor, worker) = std::os::unix::net::UnixStream::pair().unwrap();
        let (mut control, ready) = std::os::unix::net::UnixStream::pair().unwrap();
        control
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        // Relocate first so dup2 never clobbers the other source descriptor.
        fn relocated(fd: i32) -> OwnedFd {
            let copy = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 10) };
            assert!(copy >= 10);
            unsafe { OwnedFd::from_raw_fd(copy) }
        }
        let worker_fd = relocated(worker.as_raw_fd());
        let ready_fd = relocated(ready.as_raw_fd());
        let mut command = Command::new(env!("CARGO_BIN_EXE_chariox-app-wasm-worker"));
        command
            .args(["serve", launch.to_str().unwrap()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(fs::File::create(directory.join("stderr.log")).unwrap());
        unsafe {
            command.pre_exec(move || {
                if libc::dup2(worker_fd.as_raw_fd(), 3) < 0
                    || libc::dup2(ready_fd.as_raw_fd(), 4) < 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().unwrap();
        drop(worker);
        drop(ready);
        supervisor.set_nonblocking(true).unwrap();
        let channel = Channel::new(
            UnixStream::from_std(supervisor).unwrap(),
            "1".into(),
            Sender::Worker,
        )
        .unwrap();
        let mut worker = Self {
            child,
            channel: Some(channel),
            directory,
        };
        let mut header = [0; 8];
        control.read_exact(&mut header).unwrap();
        assert_eq!(&header, b"CXAWR001");
        for expected in ["1", "worker-drill", &"0".repeat(64)] {
            let mut size = [0; 4];
            control.read_exact(&mut size).unwrap();
            let size = u32::from_be_bytes(size);
            assert!(size < 256);
            let mut value = vec![0; size as usize];
            control.read_exact(&mut value).unwrap();
            assert_eq!(value, expected.as_bytes());
        }
        control.write_all(b"CXAWGO01").unwrap();
        let Message::Request {
            id, method, params, ..
        } = worker.next().await
        else {
            panic!("worker.ready expected")
        };
        assert_eq!(method, "worker.ready");
        assert_eq!(params["tools"].as_array().unwrap().len(), 13);
        worker.answer(id, Value::Null).await;
        worker
    }
    pub(super) fn peer(
        &mut self,
        broker: std::sync::Arc<dyn chariox_app_runtime::worker_peer::Broker>,
    ) -> (
        chariox_app_runtime::worker_peer::WorkerPeer,
        tokio::sync::mpsc::Receiver<chariox_app_runtime::worker_peer::ControlEvent>,
        chariox_app_runtime::worker_peer::PeerTask,
    ) {
        chariox_app_runtime::worker_peer::WorkerPeer::start(
            self.channel
                .take()
                .expect("channel already handed to kernel peer"),
            broker,
            chariox_app_runtime::worker_peer::PeerLimits::default(),
        )
        .unwrap()
    }
    pub(super) async fn next(&mut self) -> Message {
        let message = tokio::time::timeout(
            Duration::from_secs(10),
            self.channel
                .as_mut()
                .unwrap()
                .receive(Duration::from_secs(2)),
        )
        .await
        .expect("worker stalled")
        .unwrap();
        match &message {
            Message::Request { id, method, .. } => {
                println!("TRACE {} request {id} {method}", self.child.id())
            }
            Message::Response { id, outcome, .. } => println!(
                "TRACE {} response {id} {}",
                self.child.id(),
                match outcome {
                    Outcome::Success(_) => "ok",
                    Outcome::Failure(error) => &error.error.code,
                }
            ),
            Message::Cancel { id, .. } => println!("TRACE {} cancel {id}", self.child.id()),
            _ => {}
        }
        message
    }
    pub(super) async fn send(&mut self, message: Message) {
        self.channel
            .as_mut()
            .unwrap()
            .send(&message, Duration::from_secs(2))
            .await
            .unwrap();
    }
    pub(super) async fn call(&mut self, id: &str, name: &str, budget: u64) {
        self.call_input(id, name, json!({"hello":"world"}), budget)
            .await;
    }
    pub(super) async fn call_input(&mut self, id: &str, name: &str, input: Value, budget: u64) {
        self.send(Message::Request {
            version: WIRE_VERSION,
            generation: "1".into(),
            id: id.into(),
            method: "tools.invoke".into(),
            params: json!({"name":name,"input":input}),
            deadline_ms: wall_ms() + budget,
            context: Some(json!({"actor":{"kind":"human","id":"drill"}})),
        })
        .await;
    }
    pub(super) async fn answer(&mut self, id: String, value: Value) {
        self.send(Message::Response {
            version: WIRE_VERSION,
            generation: "1".into(),
            id,
            outcome: Outcome::Success(Success { result: value }),
        })
        .await;
    }
    pub(super) async fn cancel(&mut self, id: &str) {
        self.send(Message::Cancel {
            version: WIRE_VERSION,
            generation: "1".into(),
            id: id.into(),
        })
        .await;
    }
    pub(super) async fn response(&mut self, expected: &str) -> Outcome {
        loop {
            match self.next().await {
                Message::Response { id, outcome, .. } => {
                    assert_eq!(id, expected);
                    return outcome;
                }
                Message::Request { id, method, .. } if method == "http.cancel" => {
                    self.answer(id, Value::Null).await
                }
                Message::Cancel { .. } => {}
                message => panic!("unexpected worker message: {message:?}"),
            }
        }
    }
    pub(super) async fn echo(&mut self, id: &str) {
        self.call(id, "echo", 2000).await;
        let Outcome::Success(success) = self.response(id).await else {
            panic!("echo failed")
        };
        assert_eq!(success.result, json!({"hello":"world"}));
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        // Child wait ownership prevents PID reuse; never signal a system PID.
        if self.child.id() > 1 && self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        for name in ["app.wasm", "launch.json"] {
            let _ = fs::remove_file(self.directory.join(name));
        }
    }
}
