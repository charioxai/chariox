//! A kernel browser screenshot reaches the model as one image part, never as base64 text.
use super::*;
use base64::Engine;

async fn tool(
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
fn an_agent_screenshot_larger_than_the_text_limit_is_an_image_part() {
    run_test(|| {
        Box::pin(async {
            let workspace = crate::test_support::TestWorktree::new("shotfix-screenshot-image");
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
            let token = run.runtime_mcp_auth_token().unwrap();
            let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
            let root = std::env::temp_dir().join(format!(
                "chariox-shotfix-screenshot-{:032x}",
                rand::random::<u128>()
            ));
            std::fs::create_dir(&root).unwrap();
            router
                .runtime_state()
                .install_kernel_browser_fixture(agent.owner_user_id(), &root);
            // A Retina 2560x1600 PNG: signature plus 192 KiB of body, well past 64 KiB of text.
            let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
            png.extend(std::iter::repeat(7u8).take(192 * 1024));
            let encoded = base64::engine::general_purpose::STANDARD.encode(&png);
            std::fs::write(root.join("screenshot.b64"), &encoded).unwrap();
            focus(&router, session.id(), agent.id()).await;
            tool(&router, token, "chariox.load_kernel_browser", json!({}))
                .await
                .unwrap();
            let state = tool(
                &router,
                token,
                "chariox.kernel_browser",
                json!({"command":{"op":"state"}}),
            )
            .await
            .unwrap()
            .payload;
            let generation = state["generation"].as_u64().unwrap();
            let payload = tool(
                &router,
                token,
                "chariox.kernel_browser",
                json!({"command":{"op":"screenshot","tab_id":"host-tab-fixture","generation":generation}}),
            )
            .await
            .unwrap()
            .payload;

            let (content, structured) =
                crate::transport::mcp_server::runtime_tool_content_for_test(payload);
            assert_eq!(content.len(), 2, "one image part and one text part");
            assert_eq!(content[0]["type"], "image");
            assert_eq!(content[0]["mimeType"], "image/png");
            assert_eq!(content[0]["data"], encoded.as_str());
            let text = content[1]["text"].as_str().unwrap();
            assert!(
                !text.contains("chariox_truncated"),
                "no truncation: {}",
                &text[..text.len().min(200)]
            );
            assert!(
                !text.contains(&encoded[..64]),
                "the PNG is not repeated as text"
            );
            assert!(structured.get("data_base64").is_none());
            assert!(structured.get("image_base64").is_none());
            for key in ["document_id", "generation", "width", "height", "mime_type"] {
                assert!(
                    structured.get(key).is_some(),
                    "{key} stays in the text part"
                );
            }
            router.runtime_state().shutdown_cleanup().await.unwrap();
            std::fs::remove_dir_all(root).unwrap();
        })
    });
}
