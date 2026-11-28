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
            .expect("the production host returned a current reference");
        let result = ask(
            &f,
            serde_json::json!({"view":{"browser_region":{"tab_id":"one","ref":reference}}}),
        )
        .await;
        let id = result["interaction_id"].as_str().unwrap();
        let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
        assert_eq!(session.active_interactions().len(), 1);
        let card = &session.active_interactions()[0];
        assert_eq!(card.card_type(), "popup_browser_region");
        assert_eq!(serde_json::to_value(card).unwrap()["surface"], "card");
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
        let response = answer_choice_through_router(&f, id, "done", &user, "factory-owner")
            .await
            .unwrap();
        let response = serde_json::to_value(response).unwrap();
        assert_eq!(response["InteractionResponded"]["choice_id"], "done");
        assert!(f
            .runtime
            .owned
            .session_store
            .get_session(&f.room)
            .unwrap()
            .active_interactions()
            .is_empty());
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
