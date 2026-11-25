//! The agent that raised a card reads why it does not take the person's input (row 560).
use super::subtree::{room_command, world, World};
use super::*;
use crate::runtime::command::{KernelCommand, KernelCommandSource};
use crate::runtime::state::KernelBrowserDisplayRequest;
use crate::session::{
    RuntimeInteraction, RuntimeInteractionBrowserRect, RuntimeInteractionBrowserRegion,
    RuntimeInteractionChoice, RuntimeInteractionKind, RuntimeInteractionLevel,
    RuntimeInteractionView,
};

fn browser_card(id: &str, asker: &str) -> RuntimeInteraction {
    RuntimeInteraction::new(
        id,
        asker,
        RuntimeInteractionKind::Choice,
        RuntimeInteractionLevel::Warning,
        Some("Sign in".into()),
        "Sign in to the page, then press Done",
        vec![RuntimeInteractionChoice::new("done", "Done", "done", None)],
        None,
        None,
        None,
    )
    .with_view(RuntimeInteractionView {
        browser_region: RuntimeInteractionBrowserRegion {
            tab_id: "host-tab-1".into(),
            reference: "backend:42".into(),
            generation: 9,
            document_id: "DOC-A".into(),
            origin: "https://claude.ai".into(),
            rect: RuntimeInteractionBrowserRect {
                x: 0,
                y: 0,
                width: 400,
                height: 300,
            },
            viewport_width: 1280,
            viewport_height: 800,
            verification: None,
            verification_result: None,
            done_text: None,
        },
    })
}

async fn raised(label: &str, id: &str) -> World {
    let world = world(label).await;
    world
        .router
        .runtime_state
        .create_runtime_interaction(
            crate::runtime::state::CardKind::NativePermissionInput,
            &world.room,
            browser_card(id, &world.ids["a"]),
        )
        .await
        .unwrap();
    world
}

fn codes(response: &serde_json::Value) -> Vec<String> {
    response["browser"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|finding| finding["code"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn cardstate_01_the_asking_agent_reads_the_tab_generation_and_document_its_card_is_bound_to() {
    run_large_stack_async_test("cardstate-01", binding);
}
async fn binding() {
    let world = raised("cardstate-01", "card-binding").await;
    let result = room_command(&world, "a", "room card card-binding").await;
    assert!(result.ok, "{result:?}");
    let card = &result.payload["response"];
    assert_eq!(card["id"], "card-binding");
    assert_eq!(card["room_id"], world.room.as_str());
    let bound = &card["browser"]["bound"];
    assert_eq!(bound["tab_id"], "host-tab-1");
    assert_eq!(bound["generation"], 9);
    assert_eq!(bound["document_id"], "DOC-A");
    assert_eq!(bound["origin"], "https://claude.ai");
    // The browser knows no such Tab here, and no input ever arrived: both are said in words.
    assert_eq!(card["browser"]["tab"]["tab_known"], false);
    let codes = codes(card);
    assert!(codes.contains(&"tab_gone".to_owned()), "{codes:?}");
    assert!(codes.contains(&"no_input_seen".to_owned()), "{codes:?}");
}

#[test]
fn cardstate_02_the_last_refused_input_and_its_reason_are_readable() {
    run_large_stack_async_test("cardstate-02", refusal);
}
async fn refusal() {
    let world = raised("cardstate-02", "card-refusal").await;
    let identity = world
        .router
        .local_terminal_caller(KernelCommandSource::LocalCli, "cardstate-02")
        .await;
    let person = KernelCommand::from_local_request_with_caller(
        "cardstate-02",
        KernelCommandSource::LocalCli,
        identity,
        None,
        None,
        &crate::local::LocalDaemonRequest::KernelBrowser(crate::local::KernelBrowserRequest {
            command: crate::local::KernelBrowserCommand::State,
        }),
    );
    let refused = world
        .router
        .runtime_state
        .kernel_browser_display_request(
            &person,
            KernelBrowserDisplayRequest::Input {
                binding: crate::runtime::kernel_browser_host::KernelBrowserDocumentBinding {
                    tab_id: "host-tab-1".into(),
                    generation: 9,
                    document_id: String::new(),
                },
                input: crate::local::KernelBrowserInput::Text {
                    text: "never-kept-private-text".into(),
                },
            },
        )
        .await;
    assert!(
        refused.is_err(),
        "an input with no observed document is refused"
    );
    let result = room_command(&world, "a", "room card card-refusal").await;
    assert!(result.ok, "{result:?}");
    let card = &result.payload["response"];
    let last = &card["browser"]["input"]["last_refused"];
    assert_eq!(last["kind"], "text");
    assert_eq!(last["generation"], 9);
    assert!(
        last["reason"]
            .as_str()
            .unwrap()
            .contains("user_domain_stale_reference"),
        "{last:?}"
    );
    assert!(last["age_ms"].as_u64().is_some());
    assert!(card["browser"]["input"]["last_taken"].is_null());
    assert!(codes(card).contains(&"last_input_refused".to_owned()));
    assert!(
        !card.to_string().contains("never-kept-private-text"),
        "the typed text is never kept"
    );
}

#[test]
fn cardstate_03_a_card_without_a_browser_view_has_none_and_an_unknown_card_is_refused() {
    run_large_stack_async_test("cardstate-03", plain);
}
async fn plain() {
    let world = world("cardstate-03").await;
    let mut question = browser_card("card-plain", &world.ids["a"]);
    question = serde_json::from_value({
        let mut value = serde_json::to_value(question).unwrap();
        value.as_object_mut().unwrap().remove("view");
        value
    })
    .unwrap();
    world
        .router
        .runtime_state
        .create_runtime_interaction(
            crate::runtime::state::CardKind::NativePermissionInput,
            &world.room,
            question,
        )
        .await
        .unwrap();
    let result = room_command(&world, "a", "room card card-plain").await;
    assert!(result.ok, "{result:?}");
    assert!(result.payload["response"]["browser"].is_null());
    let missing = room_command(&world, "a", "room card no-such-card").await;
    assert!(!missing.ok);
    assert!(
        super::subtree::error_text(&missing).contains("no pending card"),
        "{missing:?}"
    );
}

/// The person's web input (`MirrorInput`, used by the card and the full Tab) goes through the
/// same log as the display input: the agent reads why it was refused.
async fn mirror_input(world: &World, document_id: &str, action: serde_json::Value) {
    let identity = world
        .router
        .local_terminal_caller(KernelCommandSource::LocalCli, "cardstate-mirror")
        .await;
    let command = crate::local::KernelBrowserCommand::MirrorInput {
        tab_id: "host-tab-1".into(),
        generation: 9,
        document_id: document_id.into(),
        subscription_id: "host-mirror-1".into(),
        sequence: 1,
        action: serde_json::from_value(action).unwrap(),
    };
    let person = KernelCommand::from_local_request_with_caller(
        "cardstate-mirror",
        KernelCommandSource::LocalCli,
        identity,
        None,
        None,
        &crate::local::LocalDaemonRequest::KernelBrowser(crate::local::KernelBrowserRequest {
            command: command.clone(),
        }),
    );
    let refused = world
        .router
        .runtime_state
        .kernel_browser_terminal_request(&person, command)
        .await;
    assert!(refused.is_err(), "no browser is running here: {refused:?}");
}

#[test]
fn cardstate_04_a_refused_web_input_is_readable_like_a_display_input() {
    run_large_stack_async_test("cardstate-04", web_refusal);
}
async fn web_refusal() {
    let world = raised("cardstate-04", "card-web").await;
    mirror_input(
        &world,
        "",
        serde_json::json!({"kind": "text", "text": "never-kept-web-text"}),
    )
    .await;
    mirror_input(
        &world,
        "DOC-A",
        serde_json::json!({"kind": "key", "key": "a"}),
    )
    .await;
    let result = room_command(&world, "a", "room card card-web").await;
    assert!(result.ok, "{result:?}");
    let card = &result.payload["response"];
    let last = &card["browser"]["input"]["last_refused"];
    assert_eq!(
        last["kind"], "key",
        "the latest refusal is the one read: {last:?}"
    );
    assert_eq!(last["generation"], 9);
    assert_eq!(last["document_id"], "DOC-A");
    assert!(
        last["reason"]
            .as_str()
            .is_some_and(|reason| !reason.is_empty()),
        "{last:?}"
    );
    assert!(codes(card).contains(&"last_input_refused".to_owned()));
    assert!(!card.to_string().contains("never-kept-web-text"));
}
