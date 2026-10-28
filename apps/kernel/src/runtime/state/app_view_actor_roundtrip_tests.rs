//! MP-08/MP-10/MP-11: kernel -> controller -> page -> backend -> page.
use super::*;
use crate::runtime::{
    browser_controller_app_view::ViewAgentCallKind,
    browser_controller_process::BrowserControllerProcessStore,
};
use futures_util::FutureExt;

async fn controller(
    store: &BrowserControllerProcessStore,
    session: &str,
    request: BrowserAppViewRequest,
) -> Value {
    let (store, session) = (store.clone(), session.to_owned());
    tokio::task::spawn_blocking(move || store.app_view(&session, &request))
        .await
        .unwrap()
        .unwrap()
        .unwrap()
}

async fn next_call(store: &BrowserControllerProcessStore, session: &str) -> BrowserAppViewCall {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let batch: BrowserAppViewCalls = serde_json::from_value(
                controller(store, session, BrowserAppViewRequest::Calls).await,
            )
            .unwrap();
            if let Some(call) = batch.calls.into_iter().next() {
                return call;
            }
        }
    })
    .await
    .expect("page's backend call")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn room_controller_backend_actor_and_late_call_revocation() {
    let ViewFixture {
        mut state,
        agent,
        binding,
        observations,
        _cleanup,
        ..
    } = prepare_with_turn(Mode::ToolEcho, true);
    let session = agent.session_id().to_owned();
    let root = std::env::temp_dir().join(format!(
        "app-actor-controller-{:016x}",
        rand::random::<u64>()
    ));
    std::fs::create_dir(&root).unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/runtime/state/app-view-actor.fixture.mjs");
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("slice-linux-docker/docker");
    let store = BrowserControllerProcessStore::new(
        "systemd-run",
        vec![
            "--scope".into(),
            "--quiet".into(),
            "-p".into(),
            "MemoryMax=4G".into(),
            "-p".into(),
            "MemorySwapMax=0".into(),
            "node".into(),
            fixture.display().to_string(),
            directory.display().to_string(),
            root.display().to_string(),
        ],
        Duration::from_secs(3),
    );
    store.acquire(&session).unwrap();
    state.set_browser_controller_process_store_for_test(store.clone());
    state
        .app_control()
        .lifecycle()
        .start_active_blocking("alice", "installed", tokio::runtime::Handle::current())
        .unwrap();
    wait_worker(&state).await;
    let result = std::panic::AssertUnwindSafe(async {
        let opened = controller(
            &store,
            &session,
            BrowserAppViewRequest::Open {
                origin_label: "actor".into(),
                installation_id: "installed".into(),
                instance_id: None,
                entry: "index.html".into(),
                assets: vec![BrowserAppViewAsset {
                    path: "index.html".into(),
                    content_type: "text/html".into(),
                    body_base64: "PG1haW4+PC9tYWluPg==".into(),
                }],
                page: None,
            },
        )
        .await;
        let target = opened["target_id"].as_str().unwrap();
        state
            .app_control()
            .views()
            .register(&session, target, binding);
        let invoke = controller(
            &store,
            &session,
            BrowserAppViewRequest::Agent {
                target_id: target.into(),
                agent: agent.id().into(),
                call: ViewAgentCallKind::Invoke,
                name: Some("roundtrip".into()),
                input: Some(serde_json::json!({"method":tool_method(&state)})),
                since: None,
                wait_ms: None,
                deadline_ms: 20000,
            },
        );
        let drain = async {
            let first = next_call(&store, &session).await;
            let tracked =
                state
                    .app_control()
                    .views()
                    .track_call(&session, target, first.document_id.clone());
            state
                .clone()
                .answer_app_view_call(session.clone(), first, tracked)
                .await;
            state
                .revoke_agent_extension(
                    agent.id(),
                    crate::extension::ExtensionKind::App,
                    "installed",
                    "alice",
                )
                .await
                .unwrap();
            std::fs::write(root.join("resume-handler"), "resume").unwrap();
            let late = next_call(&store, &session).await;
            let tracked =
                state
                    .app_control()
                    .views()
                    .track_call(&session, target, late.document_id.clone());
            state
                .clone()
                .answer_app_view_call(session.clone(), late, tracked)
                .await;
        };
        let (answer, ()) = tokio::join!(invoke, drain);
        answer
    })
    .catch_unwind()
    .await;
    state.shutdown_browser_controller_process().await.unwrap();
    cleanup_worker(&state).await;
    std::fs::remove_dir_all(root).unwrap();
    let result = result.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
    let requests: Vec<_> = observations
        .lock()
        .unwrap()
        .iter()
        .flat_map(|o| o.tool_requests().unwrap())
        .collect();
    assert_eq!(
        requests.len(),
        1,
        "revocation fences the handler's later backend call"
    );
    assert_eq!(
        requests[0]["context"]["actor"],
        serde_json::json!({"kind":"agent","id":agent.id()})
    );
    assert_eq!(requests[0]["context"]["agent_id"], agent.id());
    assert_eq!(
        requests[0]["context"]["turn_id"],
        state.view_agent_turn(&agent).unwrap()
    );
    assert_eq!(result["code"], "AGENT_APP_NOT_GRANTED");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an isolated Chromium and explicit disposable drill root"]
async fn room_browser_input_backend_keeps_agent_turn_and_human_actor() {
    use base64::Engine;
    let ViewFixture {
        mut state,
        agent,
        binding,
        observations,
        _cleanup,
        ..
    } = prepare_with_turn(Mode::ToolEcho, true);
    let session = agent.session_id().to_owned();
    let root = std::path::PathBuf::from(
        std::env::var_os("CHARIOX_APP_INPUT_DRILL_ROOT").expect("explicit disposable root"),
    )
    .join(format!("browser-input-{:016x}", rand::random::<u64>()));
    std::fs::create_dir(&root).unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture = std::env::var_os("CHARIOX_APP_INPUT_FIXTURE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| source.join("src/runtime/state/app-view-input.fixture.mjs"));
    let directory = std::env::var_os("CHARIOX_APP_INPUT_RUNTIME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| source.join("slice-linux-docker/docker"));
    let store = BrowserControllerProcessStore::new(
        "node",
        vec![
            fixture.display().to_string(),
            directory.display().to_string(),
            root.display().to_string(),
        ],
        Duration::from_secs(10),
    );
    store.acquire(&session).unwrap();
    state.set_browser_controller_process_store_for_test(store.clone());
    state
        .app_control()
        .lifecycle()
        .start_active_blocking("alice", "installed", tokio::runtime::Handle::current())
        .unwrap();
    wait_worker(&state).await;
    let result = std::panic::AssertUnwindSafe(async {
        let method = tool_method(&state);
        let html = "<!doctype html><button id='agent'>Action</button><button id='human'>Human</button><form><input name='title' value='Draft'><button>Save</button></form><script src='app.js'></script>";
        let script = format!(r#"
            const method = {};
            const run = chariox.action({{name:'save',effect:'write',inputSchema:{{type:'object'}}}},async (input,caller)=>{{
                await caller.call(method,{{text:input.text}});
                if(input.text!=='human') await caller.call(method,{{text:input.text+'-after'}});
            }});
            document.querySelector('#agent').onclick=()=>run({{text:'agent-click'}});
            document.querySelector('#human').onclick=()=>run({{text:'human'}});
            chariox.form(document.querySelector('form'),async (_input,caller)=>{{
                await caller.call(method,{{text:'agent-form'}});
                await caller.call(method,{{text:'agent-form-after'}});
            }});
            window.ready=true;
        "#,serde_json::to_string(&method).unwrap());
        let assets = [("index.html","text/html",html),("app.js","text/javascript",script.as_str())].into_iter().map(|(path,content_type,body)|BrowserAppViewAsset{
            path:path.into(),content_type:content_type.into(),body_base64:base64::engine::general_purpose::STANDARD.encode(body),
        }).collect();
        let opened = controller(&store,&session,BrowserAppViewRequest::Open{origin_label:"input-actor".into(),installation_id:"installed".into(),instance_id:None,entry:"index.html".into(),assets,page:None}).await;
        let target = opened["target_id"].as_str().unwrap();
        state.app_control().views().register(&session,target,binding);
        for mode in ["browser_click","browser_submit"] {
            let input = |name:&str|BrowserAppViewRequest::Agent{target_id:target.into(),agent:agent.id().into(),call:ViewAgentCallKind::Invoke,name:Some(name.into()),input:None,since:None,wait_ms:None,deadline_ms:20000};
            assert_eq!(controller(&store,&session,input(mode)).await["ok"],true);
            let first = next_call(&store,&session).await;
            assert_eq!(first.actor.as_ref().map(|a|(a.kind.as_str(),a.id.as_str())),Some(("agent",agent.id())));
            assert_eq!(controller(&store,&session,input("human_input")).await["ok"],true);
            let human = next_call(&store,&session).await;
            assert!(human.actor.is_none());
            for call in [human,first] {
                let tracked = state.app_control().views().track_call(&session,target,call.document_id.clone());
                state.clone().answer_app_view_call(session.clone(),call,tracked).await;
            }
            let after = next_call(&store,&session).await;
            assert_eq!(after.actor.as_ref().map(|a|(a.kind.as_str(),a.id.as_str())),Some(("agent",agent.id())));
            let tracked = state.app_control().views().track_call(&session,target,after.document_id.clone());
            state.clone().answer_app_view_call(session.clone(),after,tracked).await;
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }).catch_unwind().await;
    state.shutdown_browser_controller_process().await.unwrap();
    cleanup_worker(&state).await;
    std::fs::remove_dir_all(root).unwrap();
    result.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
    let requests: Vec<_> = observations
        .lock()
        .unwrap()
        .iter()
        .flat_map(|o| o.tool_requests().unwrap())
        .collect();
    assert_eq!(requests.len(), 6);
    for request in requests {
        let context = &request["context"];
        if context["actor"]["kind"] == "human" {
            assert_eq!(
                context["actor"],
                serde_json::json!({"kind":"human","id":"alice"})
            );
            assert!(context["agent_id"].is_null());
            assert!(context["turn_id"].is_null());
        } else {
            assert_eq!(
                context["actor"],
                serde_json::json!({"kind":"agent","id":agent.id()})
            );
            assert_eq!(context["agent_id"], agent.id());
            assert_eq!(context["turn_id"], state.view_agent_turn(&agent).unwrap());
        }
    }
}
