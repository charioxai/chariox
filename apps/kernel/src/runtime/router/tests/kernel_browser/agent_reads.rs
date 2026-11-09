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
}

impl Browser {
    async fn new(name: &str) -> Self {
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
        router
            .runtime_state()
            .install_kernel_browser_fixture(agent.owner_user_id(), &root);
        let png = base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\nfixture");
        std::fs::write(root.join("screenshot.b64"), png).unwrap();
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
