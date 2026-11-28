//! The production worker tool emits through the real encrypted peer transport to its home.
use super::*;

#[test]
fn taxonomy_leased_worker_popup_keeps_both_question_shapes_through_the_peer_wire() {
    run_test(leased_worker_popup_keeps_both_shapes);
}

async fn leased_worker_popup_keeps_both_shapes() {
    let _tools = with_room_tools();
    let mut fixture = LiveWorker::start().await;
    let worker = start_agent_worker(&fixture, agent_worker_state(&fixture)).await;
    let room = fixture.rooms[0].clone();
    let attachment = owner_attachment(&fixture, &room).await;
    let leased = spawn_on_agent_worker(&mut fixture, &room).await;
    run_turn(
        &fixture,
        &room,
        &attachment,
        &leased,
        "Answer two Question shapes",
    )
    .await;
    let task = store(&fixture)
        .await
        .agent_tasks(Some(&room), Some(&leased))
        .unwrap()
        .pop()
        .expect("home admitted the leased turn");
    let run = worker.run_for(&fixture, &leased, &task.prompt_id).await;
    let token = run.runtime_mcp_auth_token().unwrap().to_string();
    for fixed in [false, true] {
        let message = format!("Which route? fixed={fixed}");
        let choices = if fixed {
            json!([{"id":"a","label":"Route A","reply":"A"}])
        } else {
            json!([])
        };
        let call = worker.router.dispatch_authenticated_runtime_tool_call(
            &token,
            crate::transport::runtime_tools::REQUEST_POPUP_TOOL,
            json!({"message":message,"choices":choices,"timeout_sec":30}),
        );
        let answer = async {
            let shown = eventually_async("the production worker popup reaches home", || async {
                fixture
                    .home
                    .runtime_state()
                    .session_snapshot(&room)
                    .await
                    .ok()
                    .and_then(|session| {
                        session
                            .active_interactions()
                            .iter()
                            .find(|item| item.message() == message)
                            .cloned()
                    })
            })
            .await;
            assert_eq!(shown.card_type(), "Question");
            assert_eq!(shown.agent_id(), Some(leased.as_str()));
            assert_eq!(shown.custom_choice().unwrap().id(), "reply");
            assert_eq!(
                shown.choices().iter().any(|choice| choice.id() == "a"),
                fixed
            );
            if !fixed {
                assert!(shown.choices().is_empty());
            }
            dispatch_json(&fixture.home, json!({"RespondToInteraction":{
                "session_id":room,"interaction_id":shown.id(),"choice_id":"reply","custom_reply":"Route Ω"
            }})).await.expect("owner answers at home through the normal request");
        };
        let (result, ()) = tokio::join!(call, answer);
        let result = result.expect("the peer returns the exact home resolution to the worker");
        assert!(result.ok);
        assert_eq!(result.payload["status"], "answered");
        assert_eq!(result.payload["choice_id"], "reply");
        assert_eq!(result.payload["reply"], "Route Ω");
        assert!(fixture
            .home
            .runtime_state()
            .session_snapshot(&room)
            .await
            .unwrap()
            .active_interactions()
            .iter()
            .all(|item| item.message() != message));
    }
    // A leased provider cannot turn a popup view into user-domain browser authority.
    // These unresolved inputs must be refused before any browser lookup or card admission.
    let browser_refusal = worker
        .router
        .dispatch_authenticated_runtime_tool_call(
            &token,
            crate::transport::runtime_tools::REQUEST_POPUP_TOOL,
            json!({
                "message":"Leased browser authority boundary",
                "choices":[{"id":"cancel","label":"Cancel","reply":"cancel"}],
                "view":{"browser_region":{"tab_id":"unresolved-tab","ref":"unresolved-ref"}},
                "timeout_sec":30
            }),
        )
        .await
        .expect_err("a leased popup must not grant user-domain browser control");
    assert!(
        browser_refusal.to_string().contains("MP-08: leased or remote agents"),
        "refuse on the existing authority boundary, not an invented resolved view: {browser_refusal}"
    );
    for runtime in [fixture.home.runtime_state(), worker.router.runtime_state()] {
        assert!(runtime
            .session_snapshot(&room)
            .await
            .unwrap()
            .active_interactions()
            .iter()
            .all(|item| item.message() != "Leased browser authority boundary"));
    }
    worker.stop().await;
    fixture.stop().await;
}
