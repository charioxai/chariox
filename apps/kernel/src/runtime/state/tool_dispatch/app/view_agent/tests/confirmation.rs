//! A person-confirmed action's card through the real kernel state: the frame the kernel
//! writes, the preview the browser host finds itself, and the answers only a person may give.
use super::*;
use crate::local::AppConfirmationFrame;
use crate::runtime::state::room_cards::AgentInteractionAnswer;

const TAB: &str = "host-tab-native-test";

fn frame() -> AppConfirmationFrame {
    AppConfirmationFrame {
        installation_id: APP.into(),
        app_id: "com.chariox.campaign".into(),
        app_name: "Campaign".into(),
        publisher_id: "com.chariox".into(),
        publisher_name: "Chariox".into(),
        action_name: "pay".into(),
    }
}

/// An owner's App Tab served by a fake browser controller that answers the card's
/// region requests and logs every request it gets.
async fn preview_rig() -> (Rig, ViewContext, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let mut rig = rig_for_catalog(true);
    rig.grant().await;
    rig.stage_signed_v2_fixture();
    let view_id = rig.open_user_view(None);
    rig.start_signed_v2_fixture_worker().await;
    let origin = rig
        .runtime
        .app_control()
        .user_views()
        .get("alice", &view_id)
        .unwrap()
        .1
        .origin;
    let log = rig.root.join("confirmation-requests.log");
    let controller = rig.root.join("confirmation-controller.sh");
    std::fs::write(
        &controller,
        format!(
            r#"#!/bin/sh
while IFS= read -r request; do
  id=${{request#*:}}; id=${{id%%,*}}
  printf '%s\n' "$request" >> '{log}'
  case "$request" in
    *'"method":"health"'*) printf '{{"id":%s,"ok":true,"result":{{"state":"ready","process_id":%s,"diagnostic_code":null}}}}\n' "$id" "$$" ;;
    *'"release":true'*) printf '{{"id":%s,"ok":true,"result":{{"released":true}}}}\n' "$id" ;;
    *'"popup_region"'*)
      case "$request" in *'"after"'*) sleep 1 ;; esac
      printf '{{"id":%s,"ok":true,"result":{{"state":"present","region":{{"tab_id":"{TAB}","ref":"backend:42","generation":1,"document_id":"doc-1","origin":"{origin}","rect":{{"x":0,"y":0,"width":480,"height":120}},"viewport_width":1280,"viewport_height":800}}}}}}\n' "$id" ;;
    *) printf '{{"id":%s,"ok":true,"result":{{}}}}\n' "$id" ;;
  esac
done
"#,
            log = log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&controller, std::fs::Permissions::from_mode(0o700)).unwrap();
    let host = rig.runtime.kernel_browser_host_for_test();
    host.install_fixture_backend("alice", &controller, &rig.root);
    host.protected_request(
        "alice",
        None,
        "host.browser",
        json!({"op":"start"}),
        json!({"values":[],"targets":[],"unknown":false}),
    )
    .unwrap();
    let context = rig.runtime.view_context(&rig.agent, TAB).await.unwrap();
    (rig, context, log)
}

async fn pending_confirmation(rig: &Rig) -> crate::session::RuntimeInteraction {
    for _ in 0..400 {
        let session = rig
            .runtime
            .owned
            .session_store
            .get_session(&rig.session)
            .unwrap();
        if let Some(found) = session
            .active_interactions()
            .iter()
            .find(|i| i.id().starts_with("app-confirm-"))
        {
            return found.clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("the person was not asked to confirm");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_card_carries_the_kernels_frame_and_the_hosts_own_preview_and_confirm_lets_the_action_on(
) {
    let (rig, context, log) = preview_rig().await;
    let input = json!({"amount": 1200});
    let ask =
        rig.runtime
            .ask_person_to_confirm(rig.run.id(), &rig.agent, &context, frame(), &input);
    let (outcome, ()) = tokio::join!(ask, async {
        let card = pending_confirmation(&rig).await;
        assert_eq!(card.app_confirmation(), Some(&frame()));
        let region = &card
            .view()
            .expect("the preview is the card's protected region")
            .browser_region;
        assert_eq!(
            (region.tab_id.as_str(), region.reference.as_str()),
            (TAB, "backend:42")
        );
        let choices: Vec<_> = card.choices().iter().map(|c| c.id()).collect();
        assert_eq!(
            choices,
            ["confirm", "cancel"],
            "no session or lasting approval is offered"
        );
        assert_eq!(card.default_on_timeout(), Some("cancel"));
        assert!(
            card.message().contains(r#"Input: {"amount":1200}"#),
            "{}",
            card.message()
        );
        rig.runtime
            .resolve_runtime_interaction(&rig.session, card.id(), "confirm", None)
            .await
            .unwrap();
    });
    outcome.expect("the person's Confirm lets this one action on");
    let requests = std::fs::read_to_string(&log).unwrap();
    let first = requests
        .lines()
        .find(|line| line.contains(r#""app_confirmation":true"#))
        .unwrap();
    assert!(
        !first.contains(r#""ref""#),
        "the App and the agent name no element: {first}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_stops_the_action_and_says_nothing_ran() {
    let (rig, context, _log) = preview_rig().await;
    let input = json!({"amount": 1200});
    let ask =
        rig.runtime
            .ask_person_to_confirm(rig.run.id(), &rig.agent, &context, frame(), &input);
    let (outcome, ()) = tokio::join!(ask, async {
        let card = pending_confirmation(&rig).await;
        rig.runtime
            .resolve_runtime_interaction(&rig.session, card.id(), "cancel", None)
            .await
            .unwrap();
    });
    let declined = outcome.unwrap_err();
    assert_eq!(declined.code, "confirmation_declined");
    assert!(
        declined.message.contains("Nothing ran"),
        "{}",
        declined.message
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn neither_an_agent_nor_sudo_can_answer_the_card() {
    let (rig, context, _log) = preview_rig().await;
    let input = json!({"amount": 1200});
    let ask =
        rig.runtime
            .ask_person_to_confirm(rig.run.id(), &rig.agent, &context, frame(), &input);
    let (outcome, ()) = tokio::join!(ask, async {
        let card = pending_confirmation(&rig).await;
        for answer in [
            AgentInteractionAnswer::Holder(rig.agent.id()),
            AgentInteractionAnswer::Owner(rig.agent.id()),
        ] {
            let refused = rig.runtime.owned.resolve_runtime_interaction_routed(
                &rig.session,
                card.id(),
                "confirm",
                None,
                Some("alice"),
                false,
                None,
                false,
                false,
                None,
                Some(answer),
            );
            assert!(refused.is_err(), "an agent confirmed a person's card");
        }
        assert!(
            rig.runtime
                .sudo_native_permission_resolution(&rig.session, &card, None)
                .is_none(),
            "/sudo must not stand in for the person"
        );
        pending_confirmation(&rig).await;
        rig.runtime
            .resolve_runtime_interaction(&rig.session, card.id(), "cancel", None)
            .await
            .unwrap();
    });
    assert_eq!(outcome.unwrap_err().code, "confirmation_declined");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn simultaneous_confirm_and_cancel_settle_once() {
    let (rig, context, _log) = preview_rig().await;
    let input = json!({"amount": 1200});
    let ask =
        rig.runtime
            .ask_person_to_confirm(rig.run.id(), &rig.agent, &context, frame(), &input);
    let (outcome, ()) = tokio::join!(ask, async {
        let card = pending_confirmation(&rig).await;
        let answers: Vec<_> = ["confirm", "cancel"]
            .into_iter()
            .map(|choice| {
                let (runtime, session, id) = (
                    rig.runtime.clone(),
                    rig.session.clone(),
                    card.id().to_owned(),
                );
                tokio::spawn(async move {
                    runtime
                        .resolve_runtime_interaction(&session, &id, choice, None)
                        .await
                })
            })
            .collect();
        let mut accepted = 0;
        for answer in answers {
            accepted += usize::from(answer.await.unwrap().is_ok());
        }
        assert_eq!(accepted, 1, "exactly one answer settles a one-shot card");
    });
    // Whichever answer won, the action ran at most once: a confirm is Ok, a cancel is declined.
    assert!(matches!(
        outcome.as_ref().map_err(|e| e.code.as_str()),
        Ok(()) | Err("confirmation_declined")
    ));
}

#[tokio::test]
async fn an_action_that_needs_a_person_never_runs_in_a_view_without_a_tab() {
    let mut rig = rig();
    rig.grant().await;
    let mut declared = declared();
    declared.push(action("pay", UiActionEffect::PersonConfirmed));
    let context = rig.context(declared);
    let refused = rig
        .runtime
        .invoke_declared(
            rig.run.id(),
            &rig.agent,
            &context,
            "pay",
            json!({"text": "1200"}),
        )
        .await
        .unwrap_err();
    assert_eq!(
        refused.code, "confirmation_unavailable",
        "{}",
        refused.message
    );
    let session = rig
        .runtime
        .owned
        .session_store
        .get_session(&rig.session)
        .unwrap();
    assert!(session
        .active_interactions()
        .iter()
        .all(|i| !i.id().starts_with("app-confirm-")));
}
