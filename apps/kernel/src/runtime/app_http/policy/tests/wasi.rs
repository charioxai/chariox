//! Real WASI adapters must reach the kernel policy without transport headers.
use super::*;
use chariox_app_runtime::wire::{Channel, Message, Outcome, Sender, Success};
use chariox_app_wasm_worker::{
    ipc::{wall_ms, Ipc},
    limits::Limits,
    worker::{AppWorker, RetryPolicy, WorkerEngine},
};
use serde_json::{json, Value};
use std::{path::PathBuf, time::Duration};

async fn check(name: &str) {
    let root =
        PathBuf::from(std::env::var("CHARIOX_WORKER_DRILL_DIR").expect("run wasi-policy-drill.sh"));
    let core = std::fs::read(root.join("wasi.core.wasm")).unwrap();
    let bytes = wit_component::ComponentEncoder::default()
        .module(&core)
        .unwrap()
        .validate(true)
        .encode()
        .unwrap();
    let engine = WorkerEngine::new().unwrap();
    let component = engine.compile(&bytes, "chariox:app/app@1.0.0").unwrap();
    let worker =
        AppWorker::new(engine, component, Limits::default(), RetryPolicy::default()).unwrap();
    let (stream, peer) = tokio::net::UnixStream::pair().unwrap();
    let mut transport = Ipc::start(stream, "1".into()).unwrap();
    let task = tokio::spawn(async move {
        worker
            .run(transport.ipc.clone(), &mut transport.incoming)
            .await
    });
    let mut channel = Channel::new(peer, "1".into(), Sender::Worker).unwrap();
    let wait = Duration::from_secs(10);
    let Message::Request { id, method, .. } = channel.receive(wait).await.unwrap() else {
        panic!("worker registration expected")
    };
    assert_eq!(method, "worker.ready");
    channel
        .send(
            &Message::Response {
                version: 1,
                generation: "1".into(),
                id,
                outcome: Outcome::Success(Success {
                    result: Value::Null,
                }),
            },
            wait,
        )
        .await
        .unwrap();
    channel
        .send(
            &Message::Request {
                version: 1,
                generation: "1".into(),
                id: name.into(),
                method: "tools.invoke".into(),
                params: json!({"name":name,"input":{}}),
                deadline_ms: wall_ms() + 10_000,
                context: None,
            },
            wait,
        )
        .await
        .unwrap();
    let Message::Request { method, params, .. } = channel.receive(wait).await.unwrap() else {
        panic!("WASI HTTP request expected")
    };
    assert_eq!(method, "http.open");
    let super::super::super::decode::Command::Open(open) =
        super::super::super::decode::command(&method, params).unwrap()
    else {
        panic!("HTTP open expected")
    };
    let rules = Rules {
        destinations: BTreeMap::from([(
            "https://example.com".into(),
            BTreeSet::from([HttpMethod::Get]),
        )]),
        protected_origins: BTreeSet::new(),
        protected_routes: BTreeMap::new(),
    };
    // This is the production policy, including request_headers. Do not replace
    // it with a broker success fixture: that hid the synthesized Host header.
    let approved = rules.target(&open.url, &open.method, &open.headers);
    drop(channel);
    tokio::time::timeout(wait, task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let target = approved
        .unwrap_or_else(|error| panic!("{name}: kernel policy rejected WASI request: {error}"));
    assert_eq!(target.url().as_str(), "https://example.com/wasi");
    assert_eq!(target.method(), hyper::Method::GET);
    assert!(target.headers().is_empty());
    // The kernel still refuses an App-selected authority; only the URL may
    // supply the Host used by its HTTP transport.
    assert!(matches!(
        rules.target(
            &open.url,
            &open.method,
            &[("Host".into(), "other.example".into())]
        ),
        Err(HttpError::Invalid)
    ));
    println!(
        "DRILL {name}: real WASI adapter → kernel decode/policy; URL authority preserved: PASS"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "source WASI guest required; run scripts/wasi-policy-drill.sh"]
async fn wasi_http_policy_accepts_p2_requests() {
    check("p2-http").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "source WASI guest required; run scripts/wasi-policy-drill.sh"]
async fn wasi_http_policy_accepts_p3_requests() {
    check("p3-http").await;
}
