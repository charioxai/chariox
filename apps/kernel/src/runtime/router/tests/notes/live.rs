//! MD-N5 / MP-08 / MP-10 / MP-11: opt-in native user + local Room-controller drill.
//! The fixture controller scopes real /proc inventory to its own Chromium child.
use super::*;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct RoomBrowser(Child);
impl Drop for RoomBrowser {
    fn drop(&mut self) {
        use wait_timeout::ChildExt;
        if let Some(stdin) = self.0.stdin.as_mut() {
            let _ = stdin.write_all(b"stop\n");
        }
        if self
            .0
            .wait_timeout(Duration::from_secs(15))
            .ok()
            .flatten()
            .is_none()
        {
            assert!(self.0.id() > 1, "MD-N5: unsafe owned child PID");
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
struct Fixture {
    url: String,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Fixture {
    fn new() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/fixture", listener.local_addr().unwrap());
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = std::thread::spawn(move || {
            while !stopped.load(std::sync::atomic::Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        socket
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        let mut data = [0; 4096];
                        let _ = socket.read(&mut data);
                        let body=format!("<!doctype html><title>MD-N5 selected text</title><span>🙂{}</span><p id='quote'>MD notes selected quote</p><p>{}🙂</p>","x".repeat(63),"x".repeat(63));
                        let _=write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            url,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn driver(root: &Path, url: &str, operation: &str) {
    let driver = std::env::var_os("CHARIOX_MDNOTES_DRIVER").expect("MD-N5 driver path");
    let result = Command::new("node")
        .arg(driver)
        .arg("stimulus")
        .arg(root)
        .arg(url)
        .arg(operation)
        .status()
        .unwrap();
    assert!(result.success(), "MD-N5 stimulus failed");
}
#[test]
#[ignore = "MD-N5: requires explicit disposable CHARIOX_HOME, normal Unix user and sandboxed native Chrome"]
fn notes_user_and_room_native_integration_drill() {
    fn future_size<F: std::future::Future>(_: fn() -> F) -> usize {
        std::mem::size_of::<F>()
    }
    assert!(
        future_size(check_live) < 64 * 1024,
        "MD-N5: fixture coordinator future must fit ordinary caller stacks"
    );
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
        .block_on(Box::pin(check_live()));
}
async fn check_live() {
    let root =
        PathBuf::from(std::env::var_os("CHARIOX_MDNOTES_DRILL_ROOT").expect("MD-N5 owned root"));
    let home = PathBuf::from(std::env::var_os("CHARIOX_HOME").expect("MD-N5 explicit state"));
    assert!(root.is_absolute() && home.starts_with(&root));
    assert_ne!(unsafe { libc::geteuid() }, 0);
    // The fixture uses a private Unix socket to stimulate the same native
    // host pipe. The product host never exposes a debugger endpoint.
    std::env::set_var(
        "CHARIOX_KERNEL_BROWSER_SCRIPT",
        std::env::var_os("CHARIOX_MDNOTES_DRIVER").unwrap(),
    );
    let fixture = Fixture::new();
    let room_root = root.join("room-browser");
    std::fs::create_dir_all(&room_root).unwrap();
    let child = Command::new("node")
        .arg(std::env::var_os("CHARIOX_MDNOTES_DRIVER").unwrap())
        .arg("room")
        .arg(&room_root)
        .arg(&fixture.url)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let _room = RoomBrowser(child);
    let endpoint = room_root.join("endpoint.txt");
    for _ in 0..200 {
        if endpoint.is_file() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        endpoint.is_file(),
        "MD-N5 native Room browser did not become ready"
    );
    std::env::set_var(
        "CHARIOX_BROWSER_DEBUGGER_ENDPOINT",
        std::fs::read_to_string(&endpoint).unwrap(),
    );
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut app = DaemonApp::bootstrap({
        let mut config = DaemonConfig::load_from_env();
        config.provider_process_orphan_ttl_ms = u64::MAX;
        config.user_config.state.path = Some(home.join("state.db").display().to_string());
        config.user_config_path = home.join("config.toml");
        config.user_config.credential_vault.path =
            home.join("vault/vault.json").display().to_string();
        config
    })
    .unwrap();
    let (session, first) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            workspace.display().to_string(),
            workspace.display().to_string(),
        ))
        .unwrap();
    if std::env::var_os("CHARIOX_MD_HARDENING_DRILL").is_some() {
        let mut sessions = app.sessions_mut();
        let mut shared = sessions.get_session(session.id()).unwrap();
        shared.add_member(
            "collaborator-fixture",
            Some(first.owner_user_id().into()),
            crate::session::CollaborationLevel::Full,
        );
        sessions.restore_session(shared);
    }
    let second = spawn_test_agent(&mut app, session.id(), "second", "dev-stub");
    let first_run = launch_test_provider(
        &mut app,
        session.id(),
        first.id(),
        "dev-stub",
        "dev-stub",
        "default",
    );
    let second_run = launch_test_provider(
        &mut app,
        session.id(),
        second.id(),
        "dev-stub",
        "dev-stub",
        "default",
    );
    let a = first_run.runtime_mcp_auth_token().unwrap();
    let b = second_run.runtime_mcp_auth_token().unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    use futures_util::FutureExt;
    let assertions=Box::pin(std::panic::AssertUnwindSafe(async {
        focus(&router,session.id(),first.id()).await;
        router.dispatch_authenticated_runtime_tool_call(a,"chariox.load_notes",json!({})).await.unwrap();
        let open=LocalDaemonRequest::KernelBrowser(crate::local::KernelBrowserRequest{command:crate::local::KernelBrowserCommand::Open{url:fixture.url.clone()}});
        let LocalDaemonResponse::KernelBrowser{result}=router.dispatch(terminal_command("MD-N5-open",&open),open).await.unwrap() else{panic!("browser expected")};
        let tab=result["tabs"].as_array().unwrap().iter().find(|t|t["url"]==fixture.url).unwrap();
        let user_window=NoteWindow::KernelBrowser{tab_id:tab["tab_id"].as_str().unwrap().into(),generation:result["generation"].as_u64().unwrap()};
        let owner=router.runtime_state.provider_account_authority_owner_user_id(first.owner_user_id());
        let user_root=router.runtime_state.kernel_browser_profile_root(&owner);
        let _=router.runtime_state.ensure_browser_controller_process_started(session.id()).await.unwrap();
        router.runtime_state.start_room_environment(session.id(),crate::session::CanonicalViewport::new(1280,800,1,1280,800).unwrap()).unwrap();
        let room_state=Box::pin(router.runtime_state.reconcile_browser_controller_environment(session.id())).await.unwrap();
        let room_tab=room_state.tabs.iter().find(|t|t.url==fixture.url).unwrap();
        let room_window=NoteWindow::RoomBrowser{session_id:session.id().into(),tab_id:room_tab.tab_id.clone()};
        println!("MD-N5 / MP-10 phase: App fixture open");
        // Kernel-supplied fixture assets go through the existing App controller
        // path. This proves note capture in an App view, not package admission.
        use base64::Engine;
        use crate::runtime::browser_controller_app_view::{BrowserAppViewRequest, BrowserAppViewAsset};
        let room_app_request=BrowserAppViewRequest::Open { instance_id: None,
            origin_label:"mdnotes-fixture".into(),installation_id:"mdnotes-fixture".into(),entry:"index.html".into(),page:None,
            assets:vec![BrowserAppViewAsset {path:"index.html".into(),content_type:"text/html; charset=utf-8".into(),body_base64:base64::engine::general_purpose::STANDARD.encode(format!("<!doctype html><title>MD-N5 App</title><span>🙂{}</span><p id='quote'>MD notes selected quote</p><p>{}🙂</p>","x".repeat(63),"x".repeat(63)))}],
        };
        router.runtime_state.notes_drill_app_view(session.id(),room_app_request.clone()).await.unwrap();
        let app_url="https://app.mdnotes-fixture.invalid/".to_string();
        let app_tab = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let state = Box::pin(router.runtime_state.reconcile_browser_controller_environment(session.id())).await.unwrap();
                if let Some(tab) = state.tabs.into_iter().find(|tab| tab.url == app_url) {
                    break tab;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }).await.expect("MD-N5 owned App fixture document must commit");
        let app_window=NoteWindow::RoomBrowser{session_id:session.id().into(),tab_id:app_tab.tab_id.clone()};
        let mut windows=vec![(user_window,user_root.clone(),fixture.url.clone()),(room_window,room_root.clone(),fixture.url.clone()),(app_window,room_root.clone(),app_url)];
        if std::env::var_os("CHARIOX_MD_HARDENING_DRILL").is_some() {
            let mut request=room_app_request;
            if let BrowserAppViewRequest::Open{origin_label,instance_id,..}=&mut request {
                *origin_label="md-hardening-user-app".into();
                *instance_id=Some("md-hardening-native-view".into());
            }
            let shown=router.runtime_state.kernel_browser_app_view(owner.clone(),request,None).await.unwrap();
            windows.push((NoteWindow::KernelBrowser{tab_id:shown["tab_id"].as_str().unwrap().into(),generation:shown["generation"].as_u64().unwrap()},user_root,"https://app.md-hardening-user-app.invalid/".into()));
        }
        let mut receipts=Vec::new();
        let mut user_note=None;
        let mut hardening_notes=Vec::new();
        for (index,(window,browser_root,url)) in windows.into_iter().enumerate() {
            println!("MD-N5 / MP-10 phase: selection/MCP/anchor/Ask window {index}");
            driver(&browser_root,&url,"select");
            let NoteResult::SelectionChanged{selection:Some(selection)}=request(&router,NoteCommand::CaptureSelection{window:window.clone()}).await else{panic!("MD-N5 selection expected")};
            assert_eq!(selection.anchor.quote.exact,"MD notes selected quote");assert_eq!(selection.anchor.quote.prefix,format!("🙂{}","x".repeat(63)));assert_eq!(selection.anchor.quote.suffix,format!("{}🙂","x".repeat(63)));assert!(selection.box_css.as_ref().unwrap().width>0.);
            let NoteResult::NoteChanged{note}=request(&router,NoteCommand::Create{selection_id:selection.selection_id,comment:"Explain the selected quote".into()}).await else{panic!("note expected")};
            let list=router.dispatch_authenticated_runtime_tool_call(a,"chariox.list_notes",json!({"window":window})).await.unwrap();
            assert_eq!(list.payload["notes"][0]["note_id"],note.note_id);
            let read=router.dispatch_authenticated_runtime_tool_call(a,"chariox.read_note",json!({"note_id":note.note_id})).await.unwrap();
            assert_eq!(read.payload["note"]["anchor_state"],"attached");
            assert!(router.dispatch_authenticated_runtime_tool_call(b,"chariox.read_note",json!({"note_id":note.note_id})).await.is_err());
            driver(&browser_root,&url,"move");
            let NoteResult::NoteChanged{note:moved}=request(&router,NoteCommand::Reanchor{note_id:note.note_id.clone()}).await else{panic!("moved note expected")};
            assert_eq!(moved.anchor_state,"attached");assert!(moved.box_css.as_ref().unwrap().y>note.box_css.as_ref().unwrap().y);assert_eq!(moved.anchor.quote,note.anchor.quote);
            driver(&browser_root,&url,"page_cannot_see");
            let NoteResult::PromptDraft{text,attachment,..}=request(&router,NoteCommand::Ask{note_id:note.note_id.clone()}).await else{panic!("marked draft expected")};
            assert!(text.contains("[Chariox note ") && text.contains("MD notes selected quote") && text.contains("Explain the selected quote"));assert_eq!(attachment.mime(),"text/plain");
            driver(&browser_root,&url,"missing");
            let NoteResult::NoteChanged{note:missing}=request(&router,NoteCommand::Read{note_id:note.note_id.clone()}).await else{panic!("missing note expected")};
            assert_eq!(missing.anchor_state,"missing");assert_eq!(missing.anchor.quote,note.anchor.quote);
            hardening_notes.push(note.clone());
            if index==0 {user_note=Some(note.clone());}
            receipts.push(json!({"window":window,"selection":true,"focused_mcp":true,"nonfocused_denied":true,"page_forgery_rejected":true,"reanchored":true,"original_quote_preserved":true,"ask_marked_attachment":true}));
        }
        if std::env::var_os("CHARIOX_MD_HARDENING_DRILL").is_some() {
            let checks=super::hardening::check_native(&router,session.id(),first.id(),second.id(),a,b,&hardening_notes).await;
            std::fs::write(root.join("MD-H-NATIVE-RECEIPT.json"),serde_json::to_vec_pretty(&checks).unwrap()).unwrap();
        }
        println!("MD-N5 / MP-10 phase: browser restart");
        let note=user_note.unwrap();
        let crate::local::NoteWindow::KernelBrowser{tab_id,generation:old_generation}=&note.anchor.window else {panic!("user note expected")};
        for op in [crate::local::KernelBrowserCommand::Stop,crate::local::KernelBrowserCommand::State] {
            let request=LocalDaemonRequest::KernelBrowser(crate::local::KernelBrowserRequest{command:op});
            router.dispatch(terminal_command("MD-N5-restart",&request),request).await.unwrap();
        }
        let state_request=LocalDaemonRequest::KernelBrowser(crate::local::KernelBrowserRequest{command:crate::local::KernelBrowserCommand::State});
        let LocalDaemonResponse::KernelBrowser{result:state}=router.dispatch(terminal_command("MD-N5-restored",&state_request),state_request).await.unwrap() else {panic!("restored state expected")};
        let generation=state["generation"].as_u64().unwrap();assert!(generation>*old_generation);
        let restored_window=NoteWindow::KernelBrowser{tab_id:tab_id.clone(),generation};
        let list=router.dispatch_authenticated_runtime_tool_call(a,"chariox.list_notes",json!({"window":restored_window})).await.unwrap();
        assert_eq!(list.payload["notes"][0]["note_id"],note.note_id);
        let restored=router.dispatch_authenticated_runtime_tool_call(a,"chariox.read_note",json!({"note_id":note.note_id})).await.unwrap();
        assert_eq!(restored.payload["note"]["anchor_state"],"attached");
        assert_eq!(restored.payload["note"]["anchor"]["quote"]["exact"],note.anchor.quote.exact);
        let stale_request=LocalDaemonRequest::Notes(NotesRequest{command:NoteCommand::CaptureSelection{window:note.anchor.window.clone()}});
        assert!(router.dispatch(terminal_command("MD-N5-stale-selection",&stale_request),stale_request).await.is_err());
        receipts.push(json!({"browser_restart_reanchored":true,"stale_selection_denied":true}));
        assert!(!router.runtime_state.session_snapshot(session.id()).await.unwrap().has_active_prompt());
        std::fs::write(root.join("MD-N5-RECEIPT.json"),serde_json::to_vec_pretty(&json!({"MD":"MD-N5","MP":["MP-08","MP-10","MP-11"],"topology":"native user browser, local Room tab and App view; no Docker/relay/provider model or App package admission","protocol":crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,"checks":receipts})).unwrap()).unwrap();
    })).catch_unwind().await;
    router.runtime_state.shutdown_cleanup().await.unwrap();
    if let Err(panic) = assertions {
        std::panic::resume_unwind(panic);
    }
}
