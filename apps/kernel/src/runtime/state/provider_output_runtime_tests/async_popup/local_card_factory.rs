//! Authenticated local factory with production stdio/geometry and deterministic external CDP.
//! D3 live browser/provider acceptance remains a separate cell.
use super::*;

#[tokio::test]
async fn taxonomy_authorized_local_browser_factory_registers_the_observed_view_and_answers() {
    let f = fixture("taxonomy-local-browser-factory", true).await;
    f.begin(&f.prompt_id);
    let root = crate::test_support::TestTempDir::new("taxonomy-browser-host");
    let modules =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("slice-linux-docker/docker");
    std::fs::write(
        root.path().join("factory.mjs"),
        include_str!("local_card_factory.fixture.mjs"),
    )
    .unwrap();
    let script = root.path().join("factory.sh");
    std::fs::write(
        &script,
        format!(
            "exec node \"$1/factory.mjs\" \"$1\" '{}'\n",
            modules.to_string_lossy().replace('\'', "'\\''")
        ),
    )
    .unwrap();
    let agent = f.runtime.owned.agent_store.get_agent(&f.agent).unwrap();
    let user = f
        .runtime
        .provider_account_authority_owner_user_id(agent.owner_user_id());
    let host = f.runtime.kernel_browser_host_for_test();
    host.install_fixture_backend(&user, &script, root.path());
    let outcome = std::panic::AssertUnwindSafe(async {
        host.protected_request(
            &user,
            None,
            "host.browser",
            serde_json::json!({"op":"start"}),
            serde_json::json!({"values":[],"targets":[],"unknown":false}),
        )
        .unwrap();
        host.load_for(
            &user,
            &f.agent,
            crate::runtime::kernel_browser_host::KernelBrowserCapability::Browser,
        );
        let token = f
            .runtime
            .owned
            .provider_store
            .get_run(&f.run)
            .unwrap()
            .runtime_mcp_auth_token()
            .unwrap()
            .to_owned();
        for wait in [false, true] {
            let snapshot = f
                .runtime
                .kernel_browser_operation(
                    &user,
                    Some(&f.agent),
                    "host.browser",
                    serde_json::json!({"op":"snapshot","tab_id":"one","generation":1}),
                )
                .await
                .unwrap();
            let reference = snapshot["snapshot"]["dom_nodes"][0]["node_ref"]
                .as_str()
                .expect("the production host returned a current reference")
                .to_owned();
            let message = format!("Observed browser step wait={wait}");
            let call = f.runtime.dispatch_authenticated_runtime_tool_call(
                &token,
                crate::transport::runtime_tools::REQUEST_POPUP_TOOL,
                serde_json::json!({"wait":wait,"message":message,
                    "choices":[{"id":"continue","label":"Continue","reply":"continue"}],
                    "view":{"browser_region":{"tab_id":"one","ref":reference}},"timeout_sec":30}),
            );
            let answer = async {
                let card = tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
                        if let Some(card) = session
                            .active_interactions()
                            .iter()
                            .find(|card| card.message() == message)
                            .cloned()
                        {
                            break card;
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .expect("the authenticated factory admits the browser card");
                assert_eq!(card.card_type(), "popup_browser_region");
                assert_eq!(serde_json::to_value(&card).unwrap()["surface"], "card");
                let region = &card
                    .view()
                    .expect("the factory observed the view")
                    .browser_region;
                assert_eq!(region.tab_id, "one");
                assert_eq!(region.reference, reference);
                assert_eq!(region.document_id, "doc");
                assert_eq!(region.generation, snapshot["generation"].as_u64().unwrap());
                assert_eq!(region.origin, "https://example.com");
                assert_eq!(
                    (
                        region.rect.x,
                        region.rect.y,
                        region.rect.width,
                        region.rect.height
                    ),
                    (64, 74, 132, 82)
                );
                assert_eq!(
                    card.choices()
                        .iter()
                        .filter(|choice| choice.id() == "cancel")
                        .count(),
                    1
                );
                assert_eq!(
                    card.choices()
                        .iter()
                        .filter(|choice| choice.id() == "done")
                        .count(),
                    1
                );
                let response =
                    answer_choice_through_router(&f, card.id(), "done", &user, "factory-owner")
                        .await
                        .unwrap();
                assert_eq!(
                    serde_json::to_value(response).unwrap()["InteractionResponded"]["choice_id"],
                    "done"
                );
                card.id().to_owned()
            };
            let (result, id) = if wait {
                tokio::time::timeout(Duration::from_secs(5), async { tokio::join!(call, answer) })
                    .await
                    .expect("the owner answer returns to the waiting provider")
            } else {
                let result = tokio::time::timeout(Duration::from_secs(2), call)
                    .await
                    .expect("wait:false returns before the owner answers");
                let pending_id = result.as_ref().unwrap().payload["interaction_id"]
                    .as_str()
                    .unwrap();
                assert!(f
                    .runtime
                    .owned
                    .session_store
                    .get_session(&f.room)
                    .unwrap()
                    .active_interactions()
                    .iter()
                    .any(|card| card.id() == pending_id));
                let id = answer.await;
                (result, id)
            };
            let result = result.expect("the authenticated popup tool returns its outcome");
            assert!(result.ok);
            if wait {
                assert_eq!(result.payload["status"], "answered");
                assert_eq!(result.payload["choice_id"], "done");
                assert_eq!(result.payload["reply"], "done");
            } else {
                assert_eq!(result.payload["interaction_id"], id);
            }
            assert!(f
                .runtime
                .owned
                .session_store
                .get_session(&f.room)
                .unwrap()
                .active_interactions()
                .is_empty());
        }
        let operations =
            std::fs::read_to_string(root.path().join("factory-operations.log")).unwrap();
        assert!(operations.contains("host.browser:snapshot"));
        assert!(operations.contains("host.browser:popup_region"));
        assert!(operations.contains("DOM.resolveNode"));
    });
    use futures_util::FutureExt;
    let outcome = outcome.catch_unwind().await;
    host.shutdown().unwrap();
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
