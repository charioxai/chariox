use chariox_app_runtime::wire::{Channel, Failure, Message, Outcome, RemoteError, Sender, Success};
use chariox_app_wasm_worker::{
    ipc::{wall_ms, Ipc},
    limits::Limits,
    wasi::InstallationDirectories,
    worker::{AppWorker, RetryPolicy, WorkerEngine},
};
use serde_json::{json, Value};
use std::{path::PathBuf, time::Duration};
use tokio::net::UnixStream;

pub(super) fn component(guest: &str) -> Vec<u8> {
    let root = PathBuf::from(
        std::env::var("CHARIOX_WORKER_DRILL_DIR").expect("run scripts/wasi-drill.sh"),
    );
    let core = std::fs::read(root.join(format!("{guest}.core.wasm"))).unwrap();
    wit_component::ComponentEncoder::default()
        .module(&core)
        .unwrap()
        .validate(true)
        .encode()
        .unwrap()
}
pub(super) struct Harness {
    pub channel: Channel<UnixStream>,
    pub root: PathBuf,
    task: tokio::task::JoinHandle<wasmtime::Result<()>>,
}
impl Harness {
    pub async fn start() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "wasi-worker-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        for name in ["install/data", "install/tmp", "other/data", "other/tmp"] {
            std::fs::create_dir_all(root.join(name)).unwrap();
        }
        std::fs::write(root.join("outside"), b"outside").unwrap();
        for name in ["data", "tmp"] {
            std::os::unix::fs::symlink(
                root.join("outside"),
                root.join("install").join(name).join("escape"),
            )
            .unwrap();
        }
        let engine = WorkerEngine::new().unwrap();
        let app = engine
            .compile(&component("wasi"), chariox_app_wit::APP_WORLD)
            .unwrap();
        let worker = AppWorker::new(engine, app, Limits::default(), RetryPolicy::default())
            .unwrap()
            .with_directories(InstallationDirectories::open(&root.join("install")).unwrap());
        let (stream, peer) = UnixStream::pair().unwrap();
        let mut transport = Ipc::start(stream, "1".into()).unwrap();
        let task = tokio::spawn(async move {
            worker
                .run(transport.ipc.clone(), &mut transport.incoming)
                .await
        });
        let channel = Channel::new(peer, "1".into(), Sender::Worker).unwrap();
        let mut test = Self {
            channel,
            root,
            task,
        };
        let Message::Request {
            id, method, params, ..
        } = test.next().await
        else {
            panic!("ready expected")
        };
        assert_eq!(method, "worker.ready");
        assert_eq!(params["tools"].as_array().unwrap().len(), 6);
        test.answer(id, Ok(Value::Null)).await;
        test
    }
    pub async fn next(&mut self) -> Message {
        let message = self.channel.receive(Duration::from_secs(3)).await.unwrap();
        match &message {
            Message::Request { method, .. } => println!("WASI broker {method}"),
            Message::Cancel { .. } => println!("WASI broker request cancelled"),
            Message::Response { id, .. } => println!("WASI completed {id}"),
            _ => {}
        }
        message
    }
    pub async fn call(&mut self, id: &str, name: &str) {
        self.channel
            .send(
                &Message::Request {
                    version: 1,
                    generation: "1".into(),
                    id: id.into(),
                    method: "tools.invoke".into(),
                    params: json!({"name":name,"input":{}}),
                    deadline_ms: wall_ms() + 5000,
                    context: None,
                },
                Duration::from_secs(1),
            )
            .await
            .unwrap();
    }
    pub async fn answer(&mut self, id: String, result: Result<Value, RemoteError>) {
        let outcome = match result {
            Ok(result) => Outcome::Success(Success { result }),
            Err(error) => Outcome::Failure(Failure { error }),
        };
        self.channel
            .send(
                &Message::Response {
                    version: 1,
                    generation: "1".into(),
                    id,
                    outcome,
                },
                Duration::from_secs(1),
            )
            .await
            .unwrap();
    }
    pub async fn completed(&mut self, id: &str) -> Value {
        loop {
            match self.next().await {
                Message::Response {
                    id: actual,
                    outcome: Outcome::Success(s),
                    ..
                } => {
                    assert_eq!(id, actual);
                    return s.result;
                }
                Message::Request { id, method, .. } if method == "http.cancel" => {
                    self.answer(id, Ok(Value::Null)).await
                }
                Message::Cancel { .. } => {}
                other => panic!("unexpected completion {other:?}"),
            }
        }
    }
    pub async fn stop(self) {
        drop(self.channel);
        tokio::time::timeout(Duration::from_secs(2), self.task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        std::fs::remove_dir_all(self.root).unwrap();
    }
}
