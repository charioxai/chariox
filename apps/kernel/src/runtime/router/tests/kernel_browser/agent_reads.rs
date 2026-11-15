//! Agent-only kernel_browser operations reach the host through the real tool dispatch.
use super::*;
use base64::Engine;

async fn tool(
    router: &CommandRouter,
    token: &str,
    arguments: Value,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, crate::DaemonError> {
    router
        .dispatch_authenticated_runtime_tool_call(token, "chariox.kernel_browser", arguments)
        .await
}

/// Host requests of one operation, in order, as the fixture controller received them.
fn host_requests(root: &std::path::Path, op: &str) -> Vec<Value> {
    std::fs::read_to_string(root.join("requests.log"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|request| request["params"]["op"] == op)
        .map(|request| request["params"].clone())
        .collect()
}

struct Browser {
    router: CommandRouter,
    token: String,
    root: std::path::PathBuf,
    generation: u64,
    workspace: crate::test_support::TestWorktree,
    owner_user: String,
}

impl Browser {
    async fn new(name: &str) -> Self {
        Self::start(name, false).await
    }

    /// `real`: the kernel's own browser host and a real Chromium instead of the fixture controller.
    async fn start(name: &str, real: bool) -> Self {
        let workspace = crate::test_support::TestWorktree::new(name);
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(workspace.session_request())
            .unwrap();
        let run = launch_test_provider(
            &mut app,
            session.id(),
            agent.id(),
            "dev-stub",
            "dev-stub",
            "default",
        );
        let token = run.runtime_mcp_auth_token().unwrap().to_string();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
        let root = std::env::temp_dir().join(format!(
            "chariox-browsertools-{:032x}",
            rand::random::<u128>()
        ));
        std::fs::create_dir(&root).unwrap();
        if !real {
            router
                .runtime_state()
                .install_kernel_browser_fixture(agent.owner_user_id(), &root);
            let png = base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\nfixture");
            std::fs::write(root.join("screenshot.b64"), png).unwrap();
        }
        focus(&router, session.id(), agent.id()).await;
        tool_named(&router, &token, "chariox.load_kernel_browser", json!({}))
            .await
            .unwrap();
        let state = tool(&router, &token, json!({"command":{"op":"state"}}))
            .await
            .unwrap()
            .payload;
        Self {
            generation: state["generation"].as_u64().unwrap(),
            router,
            token,
            root,
            workspace,
            owner_user: agent.owner_user_id().to_string(),
        }
    }

    async fn call(&self, mut command: Value) -> Result<Value, crate::DaemonError> {
        command["tab_id"] = "host-tab-fixture".into();
        command["generation"] = self.generation.into();
        tool(&self.router, &self.token, json!({"command":command}))
            .await
            .map(|result| result.payload)
    }

    async fn finish(self) {
        self.router
            .runtime_state()
            .shutdown_cleanup()
            .await
            .unwrap();
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
#[ignore = "needs native Chromium; run with CHARIOX_KERNEL_BROWSER_EXECUTABLE and HEADLESS=1"]
fn find_reaches_fields_beyond_a_large_snapshot_without_exposing_protected_values() {
    run_test(|| {
        Box::pin(async {
            use std::io::{Read, Write};
            let browser = Browser::start("browsertools-find-long-page", true).await;
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let mut html = "<!doctype html><title>Long checkout</title>".to_string();
            for i in 0..100 {
                html.push_str(&format!(
                    "<p>Public article {i} {}</p>",
                    "ordinary text ".repeat(30)
                ));
            }
            html.push_str("<label>Card number find-protected-sentinel<input id=number></label><label>Card expiry<input id=expiry></label><label>Card security code<input id=cvc></label><button>Pay</button><p>find-protected-sentinel</p>");
            std::thread::spawn(move || {
                for mut stream in listener.incoming().flatten() {
                    let mut request = [0; 4096];
                    let _ = stream.read(&mut request);
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", html.len(), html);
                }
            });
            let opened = tool(
                &browser.router,
                &browser.token,
                json!({"command":{"op":"open","url":format!("http://127.0.0.1:{port}/")}}),
            )
            .await
            .unwrap()
            .payload;
            let call = |op: &str, extra: Value| {
                let mut command =
                    json!({"op":op,"tab_id":opened["tab_id"],"generation":opened["generation"]});
                for (key, value) in extra.as_object().unwrap() {
                    command[key] = value.clone();
                }
                tool(&browser.router, &browser.token, json!({"command":command}))
            };
            browser
                .router
                .runtime_state()
                .register_kernel_browser_fixture_value(
                    &browser.owner_user,
                    "find-protected-sentinel",
                );
            let snapshot = call("snapshot", json!({})).await.unwrap().payload;
            let (_, bounded) =
                crate::transport::mcp_server::runtime_tool_content_for_test(snapshot.clone());
            assert_eq!(
                bounded["chariox_truncated"], true,
                "fixture must exceed the actual MCP cap"
            );
            let found = call("find", json!({"query":"card","kind":"field"}))
                .await
                .unwrap()
                .payload;
            assert_eq!(found["matches"].as_array().unwrap().len(), 3);
            assert!(!found.to_string().contains("find-protected-sentinel"));
            assert!(bounded["chariox_guidance"]
                .as_str()
                .unwrap()
                .contains("find"));
            assert_eq!(found["document_id"], snapshot["snapshot"]["document_id"]);
            for id in ["number", "expiry", "cvc"] {
                let reference = node_ref(&snapshot, id);
                assert!(found["matches"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|node| node["node_ref"] == reference));
            }
            let (content, bounded) =
                crate::transport::mcp_server::runtime_tool_content_for_test(found);
            assert!(bounded.get("chariox_truncated").is_none());
            assert!(content[0]["text"].as_str().unwrap().len() < 16 * 1024);
            let all = call("find", json!({"query":"","kind":"any"}))
                .await
                .unwrap()
                .payload;
            assert!(!all.to_string().contains("find-protected-sentinel"));
            browser.finish().await;
        })
    });
}

async fn tool_named(
    router: &CommandRouter,
    token: &str,
    name: &str,
    arguments: Value,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, crate::DaemonError> {
    router
        .dispatch_authenticated_runtime_tool_call(token, name, arguments)
        .await
}

#[test]
fn a_screenshot_of_an_element_a_region_or_the_page_reaches_the_host_and_returns_an_image() {
    run_test(|| {
        Box::pin(async {
            let browser = Browser::new("browsertools-screenshot-region").await;
            for region in [
                json!({"node_ref":"backend:7"}),
                json!({"clip":{"x":1,"y":2,"width":30,"height":40}}),
                json!({"full_page":true}),
            ] {
                let mut command = json!({"op":"screenshot"});
                for (key, value) in region.as_object().unwrap() {
                    command[key] = value.clone();
                }
                let payload = browser.call(command).await.unwrap();
                let (content, _) =
                    crate::transport::mcp_server::runtime_tool_content_for_test(payload);
                assert_eq!(content[0]["type"], "image", "{region}");
            }
            let sent = host_requests(&browser.root, "screenshot");
            assert_eq!(sent.len(), 3);
            assert_eq!(sent[0]["node_ref"], "backend:7");
            assert_eq!(sent[1]["clip"], json!({"x":1,"y":2,"width":30,"height":40}));
            assert_eq!(sent[2]["full_page"], true);
            assert!(sent.iter().all(|params| params["focused_agent"] == true));
            for bad in [
                json!({"op":"screenshot","node_ref":"backend:7","full_page":true}),
                json!({"op":"screenshot","clip":{"x":0,"y":0,"width":0,"height":9}}),
                json!({"op":"screenshot","node_ref":"not-a-ref"}),
            ] {
                assert!(browser.call(bad.clone()).await.is_err(), "{bad}");
            }
            assert_eq!(
                host_requests(&browser.root, "screenshot").len(),
                3,
                "refused before the host"
            );
            browser.finish().await;
        })
    });
}

#[test]
fn a_save_reaches_the_host_with_the_room_workspace_as_its_only_root() {
    run_test(|| {
        Box::pin(async {
            let browser = Browser::new("browsertools-save").await;
            let saved = browser
                .call(json!({"op":"save","node_ref":"backend:7","path":"images/logo.png"}))
                .await
                .unwrap();
            assert_eq!(saved["path"], "images/logo.png");
            assert_eq!(saved["bytes"], 4);
            let sent = host_requests(&browser.root, "save");
            assert_eq!(sent.len(), 1);
            assert_eq!(sent[0]["node_ref"], "backend:7");
            assert_eq!(sent[0]["path"], "images/logo.png");
            assert_eq!(
                sent[0]["_save_roots"],
                json!([browser.workspace.path().display().to_string()]),
                "the kernel names the root; the agent never does"
            );
            for bad in [
                json!({"op":"save","path":"x.png"}),
                json!({"op":"save","url":"https://a.example/x.png","node_ref":"backend:7","path":"x.png"}),
                json!({"op":"save","url":"https://a.example/x.png","path":"x.png","_save_roots":["/"]}),
            ] {
                assert!(browser.call(bad.clone()).await.is_err(), "{bad}");
            }
            assert_eq!(
                host_requests(&browser.root, "save").len(),
                1,
                "refused before the host"
            );
            browser.finish().await;
        })
    });
}

#[test]
fn a_scroll_to_a_node_reaches_the_host_as_an_observed_document_input() {
    run_test(|| {
        Box::pin(async {
            let browser = Browser::new("browsertools-scroll-node").await;
            let scroll = |input: Value| json!({"document_id":"document","command":{"op":"input","tab_id":"host-tab-fixture","generation":browser.generation,"input":input}});
            tool(
                &browser.router,
                &browser.token,
                scroll(json!({"kind":"scroll","target":"backend:7"})),
            )
            .await
            .unwrap();
            let sent = host_requests(&browser.root, "input");
            assert_eq!(sent.len(), 1);
            assert_eq!(
                sent[0]["input"],
                json!({"kind":"scroll","target":"backend:7"})
            );
            assert_eq!(sent[0]["document_id"], "document");
            assert_eq!(sent[0]["focused_agent"], true);
            let mut no_document = scroll(json!({"kind":"scroll","target":"backend:7"}));
            no_document.as_object_mut().unwrap().remove("document_id");
            for bad in [
                no_document,
                scroll(json!({"kind":"scroll","target":"backend:7","delta_y":50})),
                scroll(json!({"kind":"scroll","target":"x"})),
            ] {
                assert!(
                    tool(&browser.router, &browser.token, bad.clone())
                        .await
                        .is_err(),
                    "{bad}"
                );
            }
            assert_eq!(
                host_requests(&browser.root, "input").len(),
                1,
                "refused before the host"
            );
            // The coordinate wheel is unchanged.
            tool(
                &browser.router,
                &browser.token,
                scroll(json!({"kind":"scroll","x":5,"y":6,"delta_x":0,"delta_y":120})),
            )
            .await
            .unwrap();
            assert_eq!(host_requests(&browser.root, "input").len(), 2);
            browser.finish().await;
        })
    });
}

#[test]
fn a_wait_is_joined_from_short_host_slices_and_answers_matched() {
    run_test(|| {
        Box::pin(async {
            let browser = Browser::new("browsertools-wait").await;
            let missed = browser
                .call(json!({"op":"wait","text":"Done","timeout_ms":3100}))
                .await
                .unwrap();
            assert_eq!(missed["matched"], false);
            assert!(missed["waited_ms"].as_u64().unwrap() >= 3100);
            assert!(
                missed["tabs"].is_array(),
                "the final state comes with the answer"
            );
            let slices: Vec<u64> = host_requests(&browser.root, "wait")
                .iter()
                .map(|params| params["timeout_ms"].as_u64().unwrap())
                .collect();
            assert!(slices.len() >= 3, "{slices:?}");
            assert_eq!(slices[0], 1500, "no host call is held longer than a slice");
            assert!(slices.iter().all(|slice| *slice <= 1500), "{slices:?}");
            std::fs::write(browser.root.join("matched"), "").unwrap();
            let before = host_requests(&browser.root, "wait").len();
            let hit = browser
                .call(json!({"op":"wait","url_contains":"/done","timeout_ms":30000}))
                .await
                .unwrap();
            assert_eq!(hit["matched"], true);
            assert_eq!(
                host_requests(&browser.root, "wait").len(),
                before + 1,
                "a match ends the wait at once"
            );
            for bad in [
                json!({"op":"wait"}),
                json!({"op":"wait","text":"a","node_ref":"backend:1"}),
                json!({"op":"wait","text":"a","timeout_ms":30001}),
            ] {
                assert!(browser.call(bad.clone()).await.is_err(), "{bad}");
            }
            assert_eq!(
                host_requests(&browser.root, "wait").len(),
                before + 1,
                "refused before the host"
            );
            browser.finish().await;
        })
    });
}

#[test]
fn a_save_and_a_wait_never_start_a_stopped_browser() {
    run_test(|| {
        Box::pin(async {
            let browser = Browser::new("browsertools-stopped").await;
            let starts = |root: &std::path::Path| {
                std::fs::read_to_string(root.join("starts")).map_or(0, |text| text.lines().count())
            };
            let started = starts(&browser.root);
            tool(
                &browser.router,
                &browser.token,
                json!({"command":{"op":"stop"}}),
            )
            .await
            .unwrap();
            for command in [
                json!({"op":"save","url":"https://a.example/x.png","path":"x.png"}),
                json!({"op":"wait","text":"Done","timeout_ms":100}),
            ] {
                let error = browser.call(command.clone()).await.unwrap_err().to_string();
                assert!(error.contains("explicitly start"), "{command}: {error}");
            }
            assert_eq!(
                starts(&browser.root),
                started,
                "a read does not undo a stop"
            );
            assert!(host_requests(&browser.root, "save").is_empty());
            assert!(host_requests(&browser.root, "wait").is_empty());
            browser.finish().await;
        })
    });
}

#[test]
fn the_tool_schema_and_description_offer_the_four_reads_in_product_words() {
    run_test(|| {
        Box::pin(async {
            let browser = Browser::new("browsertools-schema").await;
            let spec = browser
                .router
                .runtime_tool_specs_for_auth_token(&browser.token)
                .into_iter()
                .find(|spec| spec.name == "chariox.kernel_browser")
                .unwrap();
            let command = &spec.input_schema["properties"]["command"]["properties"];
            let ops: Vec<&str> = command["op"]["enum"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect();
            for op in ["screenshot", "save", "wait", "input"] {
                assert!(ops.contains(&op), "{op} is not an op of the tool: {ops:?}");
            }
            for field in [
                "node_ref",
                "clip",
                "full_page",
                "path",
                "text",
                "url_contains",
                "timeout_ms",
                "url",
            ] {
                assert!(command.get(field).is_some(), "{field} is not in the schema");
            }
            assert!(command["input"]["properties"]["target"]["description"]
                .as_str()
                .unwrap()
                .contains("Scroll"));
            for word in [
                "full_page",
                "save writes",
                "wait ends",
                "scroll with target",
            ] {
                assert!(
                    spec.description.contains(word),
                    "no {word} in the description"
                );
            }
            for name in [
                "Codex",
                "Claude",
                "OpenAI",
                "Chrome",
                "Chromium",
                "computer-use",
            ] {
                assert!(
                    !spec.description.contains(name),
                    "{name} in the agent description"
                );
            }
            browser.finish().await;
        })
    });
}

const LOGO_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

/// A page on loopback: a tall document, an image served only to a request that carries the page's cookie,
/// text that appears later and an address that changes later.
fn serve_page() -> u16 {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let logo = base64::engine::general_purpose::STANDARD
        .decode(LOGO_PNG)
        .unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut request = [0u8; 4096];
            let read = stream.read(&mut request).unwrap_or(0);
            let request = String::from_utf8_lossy(&request[..read]).to_string();
            let (head, body): (String, Vec<u8>) = if request.starts_with("GET /logo.png") {
                if request.contains("session=abc") {
                    ("200 OK\r\nContent-Type: image/png".into(), logo.clone())
                } else {
                    (
                        "403 Forbidden\r\nContent-Type: text/plain".into(),
                        b"no session".to_vec(),
                    )
                }
            } else if request.starts_with("GET / ") {
                (
                    "200 OK\r\nContent-Type: text/html\r\nSet-Cookie: session=abc; Path=/".into(),
                    concat!(
                        "<!doctype html><title>reads</title><body style=\"margin:0\">",
                        "<h1 id=\"top\" style=\"margin:0;height:120px\">Top</h1>",
                        "<img id=\"logo\" src=\"/logo.png\" width=\"40\" height=\"30\">",
                        "<div style=\"height:2400px\"></div>",
                        "<p id=\"far\" style=\"margin:0;height:80px;background:#fdc\">Far down</p>",
                        "<div style=\"height:600px\"></div>",
                        "<script>setTimeout(()=>{const p=document.createElement('p');p.textContent='Appeared later';document.body.append(p)},600);",
                        "setTimeout(()=>history.pushState({},'','/pushed/state'),900)</script>"
                    )
                    .as_bytes()
                    .to_vec(),
                )
            } else {
                (
                    "404 Not Found\r\nContent-Type: text/plain".into(),
                    b"none".to_vec(),
                )
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {head}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(&body);
        }
    });
    port
}

fn node_ref(snapshot: &Value, id: &str) -> String {
    snapshot["snapshot"]["dom_nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["attributes"]["id"] == id)
        .unwrap_or_else(|| panic!("no node with id {id}"))["node_ref"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Opt-in: the kernel's own host process and a real Chromium (`CHARIOX_KERNEL_BROWSER_EXECUTABLE`,
/// `CHARIOX_KERNEL_BROWSER_HEADLESS=1`, a disposable `TMPDIR` outside `/root`), through the real tool dispatch.
#[test]
#[ignore = "needs a native Chromium and node: see the test's doc comment"]
fn the_agent_reads_work_through_the_kernel_on_a_real_chromium() {
    run_test(|| {
        Box::pin(async {
            let browser = Browser::start("browsertools-real", true).await;
            let port = serve_page();
            let url = format!("http://127.0.0.1:{port}/");
            let opened = tool(
                &browser.router,
                &browser.token,
                json!({"command":{"op":"open","url":url}}),
            )
            .await
            .unwrap()
            .payload;
            let tab = opened["tab_id"].as_str().unwrap().to_string();
            let generation = opened["generation"].as_u64().unwrap();
            let call = |extra: Value| {
                let mut command = json!({"tab_id":tab,"generation":generation});
                for (key, value) in extra.as_object().unwrap() {
                    command[key] = value.clone();
                }
                tool(&browser.router, &browser.token, json!({"command":command}))
            };
            let mut snapshot = Value::Null;
            for _ in 0..50 {
                snapshot = call(json!({"op":"snapshot"})).await.unwrap().payload;
                if snapshot["snapshot"]["dom_nodes"]
                    .as_array()
                    .is_some_and(|nodes| nodes.iter().any(|node| node["attributes"]["id"] == "far"))
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            let (far, logo) = (node_ref(&snapshot, "far"), node_ref(&snapshot, "logo"));
            let image = |payload: Value| {
                let (content, structured) =
                    crate::transport::mcp_server::runtime_tool_content_for_test(payload);
                assert_eq!(content[0]["type"], "image");
                structured
            };
            let clip = image(
                call(json!({"op":"screenshot","clip":{"x":0,"y":0,"width":200,"height":100}}))
                    .await
                    .unwrap()
                    .payload,
            );
            // A screenshot never moves the page: the off-screen node is refused until the agent's admitted scroll.
            assert!(call(json!({"op":"screenshot","node_ref":far}))
                .await
                .is_err());
            let document = snapshot["snapshot"]["document_id"].as_str().unwrap();
            tool(
                &browser.router,
                &browser.token,
                json!({"document_id":document,"command":{"op":"input","tab_id":tab,"generation":generation,"input":{"kind":"scroll","target":far}}}),
            )
            .await
            .unwrap();
            let element = image(
                call(json!({"op":"screenshot","node_ref":far}))
                    .await
                    .unwrap()
                    .payload,
            );
            let page = image(
                call(json!({"op":"screenshot","full_page":true}))
                    .await
                    .unwrap()
                    .payload,
            );
            assert_eq!(element["clip"]["height"], 80);
            assert!(page["page_height"].as_u64().unwrap() >= 3100);
            let saved = call(json!({"op":"save","node_ref":logo,"path":"saved/logo.png"}))
                .await
                .unwrap()
                .payload;
            let by_url =
                call(json!({"op":"save","url":format!("{url}logo.png"),"path":"saved/again.png"}))
                    .await
                    .unwrap()
                    .payload;
            let expected = base64::engine::general_purpose::STANDARD
                .decode(LOGO_PNG)
                .unwrap();
            for (answer, file) in [(&saved, "saved/logo.png"), (&by_url, "saved/again.png")] {
                assert_eq!(answer["path"], file);
                assert_eq!(answer["bytes"], expected.len());
                assert_eq!(answer["mime_type"], "image/png");
                assert_eq!(
                    std::fs::read(browser.workspace.path().join(file)).unwrap(),
                    expected
                );
            }
            assert!(call(
                json!({"op":"save","url":format!("{url}logo.png"),"path":"../escape.png"})
            )
            .await
            .is_err());
            let later = call(json!({"op":"wait","text":"appeared LATER","timeout_ms":5000}))
                .await
                .unwrap()
                .payload;
            let pushed = call(json!({"op":"wait","url_contains":"/pushed/","timeout_ms":5000}))
                .await
                .unwrap()
                .payload;
            let never = call(json!({"op":"wait","text":"never on this page","timeout_ms":1800}))
                .await
                .unwrap()
                .payload;
            assert_eq!(
                (
                    later["matched"].clone(),
                    pushed["matched"].clone(),
                    never["matched"].clone()
                ),
                (json!(true), json!(true), json!(false))
            );
            println!(
                "agent reads through the kernel: {}",
                json!({"clip":[clip["width"],clip["height"]],"element":element["clip"],"page":[page["width"],page["height"],page["page_height"]],
                    "saved":[saved["bytes"],by_url["bytes"]],"waited_ms":[later["waited_ms"],pushed["waited_ms"],never["waited_ms"]]})
            );
            browser.finish().await;
        })
    });
}
