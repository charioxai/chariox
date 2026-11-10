//! `chariox.visual.show` and `chariox.visual.preview`: an agent shows the person a page, and checks it first.
use super::*;

const TOKEN: &str = "visual-tools-token";

async fn runtime_with_agent(root: &crate::test_support::TestWorktree) -> KernelRuntimeState {
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            root.path().display().to_string(),
            root.path().display().to_string(),
        ))
        .unwrap();
    let launch = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "dev-stub",
        "dev-stub",
        "default",
        "test",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "visual-tools-run",
        &launch,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "visual tools".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: Some(root.path().to_owned()),
            structured_endpoint: None,
        },
    );
    run.mark_running();
    run.set_runtime_mcp_auth_token(Some(TOKEN.into()));
    app.providers_mut().insert_run_for_test(run);
    owned_runtime_state(&Arc::new(Mutex::new(app))).await
}

async fn call(
    runtime: &KernelRuntimeState,
    tool: &str,
    arguments: serde_json::Value,
) -> Result<serde_json::Value, DaemonError> {
    runtime
        .dispatch_authenticated_runtime_tool_call(TOKEN, tool, arguments)
        .await
        .map(|result| result.payload)
}

#[tokio::test]
async fn a_run_is_offered_both_visual_tools() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("visual-tools-offer");
    let runtime = runtime_with_agent(&root).await;
    let names = runtime
        .runtime_tool_specs_for_auth_token(TOKEN)
        .into_iter()
        .map(|spec| spec.name)
        .collect::<Vec<_>>();
    for tool in ["chariox.visual.show", "chariox.visual.preview"] {
        assert!(
            names.iter().any(|name| name == tool),
            "{tool} is not offered"
        );
    }
}

#[tokio::test]
async fn show_keeps_a_copy_of_the_page_that_outlives_its_file() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("visual-tools-show");
    let runtime = runtime_with_agent(&root).await;
    let page = "<!doctype html><html><body><h1>Sales</h1></body></html>";
    std::fs::write(root.path().join("chart.html"), page).unwrap();
    let shown = call(
        &runtime,
        "chariox.visual.show",
        serde_json::json!({"path": "chart.html", "wide": true}),
    )
    .await
    .unwrap();
    std::fs::remove_file(root.path().join("chart.html")).unwrap();
    let stored = shown["path"].as_str().unwrap();
    assert_eq!(
        std::fs::read_to_string(stored).unwrap(),
        page,
        "the copy is the page"
    );
    assert!(stored.ends_with(".html") && std::path::Path::new(stored).is_absolute());
    assert_eq!(shown["bytes"], page.len());
    assert_eq!(shown["wide"], true);
    let again = call(
        &runtime,
        "chariox.visual.show",
        serde_json::json!({"html": page}),
    )
    .await
    .unwrap();
    assert_eq!(again["path"], shown["path"], "the same page is one file");
    assert_eq!(again["wide"], false);
    // A harness may normalize the dots of the tool name.
    let aliased = call(
        &runtime,
        "mcp__chariox__chariox_visual_show",
        serde_json::json!({"html": page}),
    )
    .await
    .unwrap();
    assert_eq!(aliased["path"], shown["path"]);
}

#[tokio::test]
async fn show_refuses_what_it_cannot_draw_and_says_why() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("visual-tools-refuse");
    let runtime = runtime_with_agent(&root).await;
    std::fs::write(root.path().join("notes.txt"), "hello").unwrap();
    let too_big = "x".repeat(1024 * 1024 + 1);
    let cases = [
        (serde_json::json!({}), "either as path or as html"),
        (
            serde_json::json!({"path": "a.html", "html": "<p>x</p>"}),
            "either as path or as html",
        ),
        (
            serde_json::json!({"path": "notes.txt"}),
            "not an .html file",
        ),
        (serde_json::json!({"path": "missing.html"}), "cannot read"),
        (serde_json::json!({"html": "  "}), "empty"),
        (serde_json::json!({"html": too_big}), "larger than 1 MB"),
        (
            serde_json::json!({"html": "<p>x</p>", "extra": 1}),
            "invalid arguments",
        ),
    ];
    for (arguments, reason) in cases {
        let error = call(&runtime, "chariox.visual.show", arguments.clone())
            .await
            .unwrap_err();
        assert!(error.to_string().contains(reason), "{arguments}: {error}");
    }
}

#[tokio::test]
async fn preview_returns_a_picture_the_height_and_the_console() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("visual-tools-preview");
    let runtime = runtime_with_agent(&root).await;
    let page = r#"<!doctype html><html><body style="margin:0"><div style="height:700px;background:#2876dc"></div><script>console.log("chart ready")</script></body></html>"#;
    let payload = match call(
        &runtime,
        "chariox.visual.preview",
        serde_json::json!({"html": page, "width": 800}),
    )
    .await
    {
        Ok(payload) => payload,
        Err(error) if error.to_string().contains("no Chromium or Chrome") => return,
        Err(error) => panic!("{error}"),
    };
    assert_eq!(payload["mime_type"], "image/png");
    let png = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        payload["image_base64"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(&png[..4], b"\x89PNG");
    assert_eq!(payload["content_height"], 700);
    assert_eq!(payload["clipped"], false);
    assert_eq!(payload["console"], serde_json::json!(["chart ready"]));
    // A page taller than the window is cut off in the picture, and says so.
    let tall = r#"<!doctype html><html><body style="margin:0"><div style="height:4000px"></div></body></html>"#;
    let payload = call(
        &runtime,
        "chariox.visual.preview",
        serde_json::json!({"html": tall}),
    )
    .await
    .unwrap();
    assert_eq!(payload["content_height"], 4000);
    assert_eq!(payload["clipped"], true);
}
