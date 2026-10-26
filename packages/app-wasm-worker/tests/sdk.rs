//! Frozen C/C++ SDK artifacts run through the real worker subprocess.
//! Set CHARIOX_WORKER_SDK_DIR using scripts/sdk-drill.sh.
#[path = "process/support.rs"]
mod support;
use chariox_app_runtime::wire::{Message, Outcome};
use serde_json::{json, Value};
use support::Worker;
fn code(outcome: &Outcome) -> &str {
    match outcome {
        Outcome::Failure(e) => &e.error.code,
        _ => "OK",
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "pinned SDK artifacts required; use scripts/sdk-drill.sh"]
async fn c_and_cpp_templates_and_lifetimes_in_real_worker() {
    let root = std::path::PathBuf::from(std::env::var("CHARIOX_WORKER_SDK_DIR").unwrap());
    for language in ["c", "cpp"] {
        let mut worker =
            Worker::start_component(&root.join(format!("{language}-template.wasm")), 3).await;
        worker.echo("template-echo").await;
        worker.call("counter", "counter", 2000).await;
        let mut get = false;
        loop {
            match worker.next().await {
                Message::Request {
                    id, method, params, ..
                } => {
                    if method == "state.get" {
                        get = true;
                        assert_eq!(params["key"], "count");
                        worker.answer(id, Value::Null).await;
                    } else {
                        assert_eq!(method, "state.transaction");
                        assert!(get);
                        worker.answer(id, json!({"revision":1,"receipts":[]})).await;
                    }
                }
                Message::Response {
                    outcome: Outcome::Success(s),
                    ..
                } => {
                    assert_eq!(s.result, json!({"count":1,"revision":1}));
                    break;
                }
                other => panic!("template counter failed: {other:?}"),
            }
        }
        worker
            .call_input("clock", "wait", json!({"ms":1}), 2000)
            .await;
        assert!(matches!(
            worker.response("clock").await,
            Outcome::Success(_)
        ));
        println!("SDK {language} actual template echo/state/clock: PASS");
        drop(worker);
        let mut worker =
            Worker::start_component(&root.join(format!("{language}-drill.wasm")), 5).await;
        for tool in ["sdk-stream", "sdk-yield", "sdk-cancel-start", "sdk-held"] {
            worker.call(tool, tool, 2000).await;
            let mut starts = 0;
            let mut drops = 0;
            let mut reads = 0;
            let mut cancels = 0;
            let mut import_started = false;
            let mut returned = None;
            while returned.is_none() || drops == 0 {
                match worker.next().await {
                    Message::Request {
                        id, method, params, ..
                    } => {
                        match method.as_str() {
                            "log.write" => {
                                assert_eq!(params["fields"], tool);
                                if params["message"] == "sdk.start" {
                                    starts += 1;
                                    if tool == "sdk-yield" {
                                        worker.cancel(tool).await;
                                    }
                                } else {
                                    assert_eq!(params["message"], "sdk.destroy");
                                    drops += 1;
                                }
                                worker.answer(id, Value::Null).await;
                            }
                            "state.get" => {
                                import_started = true;
                                if tool == "sdk-held" {
                                    tokio::time::sleep(std::time::Duration::from_millis(15)).await;
                                    worker.cancel(tool).await;
                                } else {
                                    assert_eq!(tool, "sdk-cancel-start");
                                }
                            }
                            "http.open" => worker.answer(id, json!({"streamId":"sdk"})).await,
                            "http.headers" => worker
                                .answer(
                                    id,
                                    json!({"status":200,"headers":[],"url":"https://example.com/"}),
                                )
                                .await,
                            "http.read" => {
                                reads += 1;
                                assert!(reads <= 2);
                                worker
                                    .answer(
                                        id,
                                        if reads == 1 {
                                            json!({"chunkBase64":"YWJj"})
                                        } else {
                                            json!({"done":true})
                                        },
                                    )
                                    .await;
                            }
                            "http.cancel" => {
                                cancels += 1;
                                worker.answer(id, Value::Null).await;
                            }
                            other => panic!("unexpected SDK broker call {other}"),
                        }
                    }
                    Message::Cancel { .. } => {}
                    Message::Response { id, outcome, .. } => {
                        assert_eq!(id, tool);
                        assert!(returned.is_none(), "one invocation result");
                        returned = Some(outcome);
                    }
                    other => panic!("unexpected SDK message {other:?}"),
                }
            }
            assert_eq!(starts, 1);
            assert_eq!(
                drops, 1,
                "invocation/coroutine state destroyed exactly once"
            );
            let outcome = returned.unwrap();
            match tool {
                "sdk-stream" => {
                    assert!(
                        matches!(outcome,Outcome::Success(ref s) if s.result==json!({"bytes":3}))
                    );
                    assert_eq!(reads, 2);
                }
                "sdk-cancel-start" => {
                    assert_eq!(code(&outcome), "EXPECTED");
                    assert!(import_started);
                }
                _ => assert_eq!(code(&outcome), "CANCELLED"),
            }
            if tool == "sdk-stream" && cancels == 0 {
                match worker.next().await {
                    Message::Request { id, method, .. } => {
                        assert_eq!(method, "http.cancel");
                        worker.answer(id, Value::Null).await;
                    }
                    other => panic!("HTTP body resource was not released: {other:?}"),
                }
            }
            println!("SDK {language} {tool} and exactly-once destruction: PASS");
        }
        drop(worker);
        let core = std::fs::read(root.join("pre-cancel.core.wasm")).unwrap();
        let proxy = root.join("pre-cancel-proxy.wasm");
        std::fs::write(
            &proxy,
            wit_component::ComponentEncoder::default()
                .module(&core)
                .unwrap()
                .validate(true)
                .encode()
                .unwrap(),
        )
        .unwrap();
        let config = wasm_compose::config::Config {
            dir: root.clone(),
            definitions: vec![root.join(format!("{language}-drill.wasm"))],
            ..Default::default()
        };
        let composed = wasm_compose::composer::ComponentComposer::new(&proxy, &config)
            .compose()
            .unwrap();
        chariox_app_wit::check_component(&composed, chariox_app_wit::APP_WORLD).unwrap();
        let path = root.join(format!("{language}-before-start.wasm"));
        std::fs::write(&path, composed).unwrap();
        let mut worker = Worker::start_component(&path, 5).await;
        worker.call("pre-cancel", "sdk-before-start", 2000).await;
        let outcome = worker.response("pre-cancel").await;
        assert_eq!(code(&outcome), "CANCELLED");
        // response() refuses log/state/HTTP calls, proving no App body or
        // C++ frame started before the already-written cancellation value.
        println!("SDK {language} canonical cancellation ready before SDK entry: no App code: PASS");
    }
}
