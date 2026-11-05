//! Real Wasmtime execution of the two allowed WASI versions and migration world.
#[path = "process/support.rs"]
mod process;
#[path = "wasi/support.rs"]
mod support;
use chariox_app_runtime::wire::{Message, Outcome, RemoteError};
use chariox_app_wasm_worker::{
    ipc::{wall_ms, Ipc},
    limits::Limits,
    migration::{types::InvocationContext, MigrationWorker},
    scope::Scope,
    wasi::InstallationDirectories,
    worker::WorkerEngine,
};
use serde_json::{json, Value};
use std::time::Duration;
use support::Harness;
#[path = "wasi/migration.rs"]
mod migration;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "source WASI guest required; run scripts/wasi-drill.sh"]
async fn wasi_02_and_03_http_use_broker_and_preserve_denials() {
    let mut worker = Harness::start().await;
    for name in ["p2-http", "p3-http"] {
        for denied in [false, true] {
            worker.call(name, name).await;
            let mut reads = 0;
            loop {
                match worker.next().await {
                Message::Request {id,method,params,..}=>match method.as_str() {
                    "http.open"=> {
                        assert_eq!(params["url"],"https://example.com/wasi");
                        assert_eq!(params["method"],"GET");assert_eq!(params["hasBody"],false);
                        worker.answer(id,if denied {Err(RemoteError {code:"APP_HTTP_DESTINATION_DENIED".into(),message:"destination not declared".into(),retryable:Some(false)})}else{Ok(json!({"streamId":name}))}).await;
                    }
                    "http.headers"=>worker.answer(id,Ok(json!({"status":200,"headers":[],"url":"https://example.com/wasi"}))).await,
                    "http.read"=>{reads+=1;assert!(reads<=2);worker.answer(id,Ok(if reads==1 {json!({"chunkBase64":"YWJj"})}else{json!({"done":true})})).await;},
                    "http.cancel"=>worker.answer(id,Ok(Value::Null)).await,
                    other=>panic!("WASI bypassed broker semantics: {other}"),
                },
                Message::Cancel {..}=>{},
                Message::Response {outcome:Outcome::Success(s),..}=>{
                    assert_eq!(s.result,if denied {json!({"denied":true})}else{json!({"body":"abc"})});break;
                }
                other=>panic!("WASI HTTP failed {other:?}"),
            }
            }
            println!("DRILL {name} broker HTTP denial={denied}: PASS");
        }
    }
    worker.stop().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "source WASI guest required; run scripts/wasi-drill.sh"]
async fn wasi_preopens_are_scoped_and_clocks_random_output_work() {
    let mut worker = Harness::start().await;
    worker.call("files", "preopens").await;
    assert_eq!(worker.completed("files").await, json!({"preopens":true}));
    for name in ["data", "tmp"] {
        assert_eq!(
            std::fs::read(worker.root.join("install").join(name).join("value")).unwrap(),
            b"scoped"
        );
        assert!(!worker.root.join("other").join(name).join("value").exists());
    }
    assert_eq!(
        std::fs::read(worker.root.join("outside")).unwrap(),
        b"outside"
    );
    println!("DRILL WASI 0.2/0.3 data/tmp share installation files; absolute/traversal/symlink escapes return not-permitted: PASS");
    worker.call("runtime", "runtime").await;
    let mut outputs = Vec::new();
    loop {
        match worker.next().await {
            Message::Request {
                id, method, params, ..
            } => {
                assert_eq!(method, "log.write");
                assert_eq!(params["fields"]["source"], "wasi");
                outputs.push(params["message"].as_str().unwrap().to_owned());
                worker.answer(id, Ok(Value::Null)).await;
            }
            Message::Response {
                outcome: Outcome::Success(s),
                ..
            } => {
                assert_eq!(s.result, json!({"runtime":true}));
                break;
            }
            other => panic!("WASI runtime failed {other:?}"),
        }
    }
    assert!(outputs.iter().any(|s| s == "wasi-drill"));
    assert!(outputs.iter().any(|s| s == "wasi-p3"));
    println!("DRILL WASI 0.2/0.3 clocks/random and brokered stdout/stderr: PASS");
    worker.stop().await;
}

#[test]
fn preopen_setup_refuses_symlinks_and_missing_installations() {
    let root = std::env::temp_dir().join(format!("wasi-preopen-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    assert!(InstallationDirectories::open(&root).is_err());
    std::fs::create_dir(root.join("tmp")).unwrap();
    std::os::unix::fs::symlink(root.join("tmp"), root.join("data")).unwrap();
    assert!(InstallationDirectories::open(&root)
        .unwrap_err()
        .to_string()
        .contains("APP_PREOPEN_INVALID"));
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "source migration guest required; run scripts/wasi-drill.sh"]
async fn migration_world_executes_state_only_and_stops_on_cancel_deadline_trap() {
    use chariox_app_runtime::wire::{Channel, Sender, Success};
    let bytes = support::component("migration");
    chariox_app_wit::check_component(&bytes, chariox_app_wit::MIGRATION_WORLD).unwrap();
    assert!(chariox_app_wit::check_component(
        &support::component("wasi"),
        chariox_app_wit::MIGRATION_WORLD
    )
    .is_err());
    for (from, budget, cancel, expected) in [
        (1, 2000, false, "OK"),
        (3, 2000, true, "CANCELLED"),
        (3, 50, false, "DEADLINE_EXCEEDED"),
        (98, 2000, false, "APP_TRAPPED"),
        (99, 2000, false, "APP_CPU_SLICE"),
    ] {
        let engine = WorkerEngine::new().unwrap();
        let component = engine
            .compile(&bytes, chariox_app_wit::MIGRATION_WORLD)
            .unwrap();
        let limits = Limits {
            slice_ms: 100,
            ..Limits::default()
        };
        let migration = MigrationWorker::new(engine, component, limits).unwrap();
        let (stream, peer) = tokio::net::UnixStream::pair().unwrap();
        let transport = Ipc::start(stream, "1".into()).unwrap();
        let scope = Scope::new(
            "migration".into(),
            tokio::time::Instant::now() + Duration::from_millis(budget),
        );
        let signal = scope.clone();
        let context = InvocationContext {
            deadline_ms: wall_ms() + budget,
            installation_id: Some("installation".into()),
            room_id: None,
            operation_id: None,
            agent_id: None,
            task_id: None,
            turn_id: None,
            actor: None,
        };
        let task = tokio::spawn(async move {
            migration
                .run(transport.ipc.clone(), scope, from, from + 1, context)
                .await
        });
        let mut channel = Channel::new(peer, "1".into(), Sender::Worker).unwrap();
        if from < 98 {
            let Message::Request { id, method, .. } =
                channel.receive(Duration::from_secs(2)).await.unwrap()
            else {
                panic!("state import expected")
            };
            assert_eq!(method, "state.get");
            if cancel {
                signal.cancel("CANCELLED");
            }
            if from == 1 {
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
                        Duration::from_secs(1),
                    )
                    .await
                    .unwrap();
                let Message::Request {
                    id, method, params, ..
                } = channel.receive(Duration::from_secs(2)).await.unwrap()
                else {
                    panic!("state transaction expected")
                };
                assert_eq!(method, "state.transaction");
                assert_eq!(params["schemaVersion"], 2);
                channel
                    .send(
                        &Message::Response {
                            version: 1,
                            generation: "1".into(),
                            id,
                            outcome: Outcome::Success(Success {
                                result: json!({"revision":1,"receipts":[]}),
                            }),
                        },
                        Duration::from_secs(1),
                    )
                    .await
                    .unwrap();
            }
        }
        let result = tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            result
                .as_ref()
                .err()
                .map_or("OK", |error| error.code.as_str()),
            expected
        );
        println!(
            "DRILL real migration state/cancel/deadline/trap from={from} result={expected}: PASS"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "source WASI guest required; run scripts/wasi-drill.sh"]
async fn wasi_response_drop_stops_pending_upload_before_invocation_finishes() {
    let mut worker = Harness::start().await;
    worker.call("drop", "p3-drop").await;
    let (mut headers, mut write) = (None, None);
    let mut held = Vec::new();
    let (mut stopped, mut dropped, mut cancelled_write, mut cancelled_stream) =
        (false, false, false, false);
    while !stopped || !dropped || !cancelled_write || !cancelled_stream {
        let message = tokio::time::timeout(Duration::from_millis(750), worker.next())
            .await
            .expect("response drop must stop both pumps promptly");
        match message {
            Message::Request {
                id, method, params, ..
            } => match method.as_str() {
                "http.open" => {
                    assert_eq!(params["method"], "POST");
                    assert_eq!(params["hasBody"], true);
                    worker.answer(id, Ok(json!({"streamId":"early"}))).await;
                }
                "http.headers" => headers = Some(id),
                "http.write" => {
                    assert!(write.is_none());
                    assert_eq!(params["end"], false);
                    write = Some(id);
                }
                "http.read" => {}
                "http.cancel" => {
                    assert!(!cancelled_stream);
                    cancelled_stream = true;
                    worker.answer(id, Ok(Value::Null)).await;
                }
                "state.get" => {
                    match params["key"].as_str().unwrap() {
                        "upload-stopped" => stopped = true,
                        "response-dropped" => dropped = true,
                        other => panic!("unexpected state {other}"),
                    };
                    held.push(id);
                }
                other => panic!("unexpected early-response call {other}"),
            },
            Message::Cancel { id, .. } => {
                if write.as_ref() == Some(&id) {
                    cancelled_write = true;
                }
            }
            other => panic!("handler must remain alive: {other:?}"),
        }
        if write.is_some() {
            if let Some(id) = headers.take() {
                worker
                    .answer(
                        id,
                        Ok(json!({"status":200,"headers":[],"url":"https://example.com/wasi"})),
                    )
                    .await;
            }
        }
    }
    for id in held {
        worker.answer(id, Ok(Value::Null)).await;
    }
    assert_eq!(worker.completed("drop").await, Value::Null);
    worker.stop().await;
    println!("DRILL WASI response drop cancels upload/broker stream before invocation ends: PASS");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "source WASI guest required; run scripts/wasi-drill.sh"]
async fn serialized_installation_root_scopes_real_worker_process_preopens() {
    let root = std::env::temp_dir().join(format!("wasi-cli-preopens-{}", std::process::id()));
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
    let component = root.join("wasi.wasm");
    std::fs::write(&component, support::component("wasi")).unwrap();
    let mut worker = process::Worker::start_installed(&component, 6, &root.join("install")).await;
    worker.call("files", "preopens", 2000).await;
    assert!(
        matches!(worker.response("files").await,Outcome::Success(s) if s.result==json!({"preopens":true}))
    );
    for name in ["data", "tmp"] {
        assert_eq!(
            std::fs::read(root.join("install").join(name).join("value")).unwrap(),
            b"scoped"
        );
        assert!(!root.join("other").join(name).join("value").exists());
    }
    assert_eq!(std::fs::read(root.join("outside")).unwrap(), b"outside");
    drop(worker);
    std::fs::remove_dir_all(root).unwrap();
    println!("DRILL supervisor installation_root reaches real worker process; only installation data/tmp visible: PASS");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "source WASI guest required; run scripts/wasi-drill.sh"]
async fn upload_error_does_not_wait_for_pending_response_headers() {
    let mut worker = Harness::start().await;
    worker.call("upload-error", "p3-upload-error").await;
    let mut response = false;
    let mut released = false;
    while !response || !released {
        let message = tokio::time::timeout(Duration::from_millis(750), worker.next())
            .await
            .expect("upload failure must not wait for pending headers");
        match message {
            Message::Request { id, method, .. } => match method.as_str() {
                "http.open" => {
                    worker
                        .answer(id, Ok(json!({"streamId":"bad-upload"})))
                        .await
                }
                "http.headers" => {} // Never answer: only the upload can finish the call.
                "http.cancel" => {
                    assert!(!released);
                    released = true;
                    worker.answer(id, Ok(Value::Null)).await;
                }
                other => panic!("unexpected upload-error call {other}"),
            },
            Message::Cancel { .. } => {}
            Message::Response {
                outcome: Outcome::Success(s),
                ..
            } => {
                assert_eq!(s.result, json!({"denied":true}));
                response = true;
            }
            other => panic!("unexpected upload-error result {other:?}"),
        }
    }
    worker.stop().await;
    println!(
        "DRILL upload failure cancels pending headers and stream before invocation deadline: PASS"
    );
}
