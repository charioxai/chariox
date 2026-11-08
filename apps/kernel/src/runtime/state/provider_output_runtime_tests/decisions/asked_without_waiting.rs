//! A sub-agent's question that does not block its turn (`request_popup` wait:false) is the same
//! decision as a blocking one: its holder is told when it is raised, or it reaches the user.
use super::reparent::cards;
use super::*;
use crate::runtime::state::decision_holder::DecisionHolder;

impl Tree {
    /// `agent` calls `request_popup` wait:false in its running turn; returns the question id.
    async fn ask_through_popup(&self, agent: &str) -> String {
        self.begin_task(agent);
        let token = self
            .runtime
            .owned
            .provider_store
            .get_run_for_agent(&self.room, agent)
            .unwrap()
            .runtime_mcp_auth_token()
            .unwrap()
            .to_owned();
        let result = self
            .runtime
            .dispatch_authenticated_runtime_tool_call(
                &token,
                crate::transport::runtime_tools::REQUEST_POPUP_TOOL,
                serde_json::json!({"wait": false, "message": "Deploy now?", "choices": [
                    {"id": "yes", "label": "Yes", "reply": "Deploy."}
                ]}),
            )
            .await
            .unwrap();
        result.payload["interaction_id"]
            .as_str()
            .unwrap()
            .to_owned()
    }
}

/// Live (uiqa hk-10, pcdrive): the parent was given the question but never told, and the user
/// had no card, so the child waited for nobody.
#[tokio::test]
async fn a_question_asked_without_waiting_tells_its_holder_once() {
    let t = tree(
        "decision-async-told",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;

    let id = t.ask_through_popup(&t.c).await;

    assert_eq!(t.holder(&id), DecisionHolder::Agent(t.p.clone()));
    let events = t.decision_events(&t.p);
    assert_eq!(events.len(), 1, "the holder is told once: {events:?}");
    assert_eq!(events[0].payload["interaction_id"], id);
    assert!(
        t.decision_events(&t.g).is_empty(),
        "only the holder is told"
    );
}

/// With every ancestor parked nobody would take the question: it is the user's, in the top panel.
#[tokio::test]
async fn a_question_asked_without_waiting_under_parked_ancestors_reaches_the_user() {
    let t = tree(
        "decision-async-parked",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    t.end_turn_yielded(&t.p).await;
    t.end_turn_yielded(&t.g).await;

    let id = t.ask_through_popup(&t.c).await;

    assert_eq!(t.holder(&id), DecisionHolder::User, "parked holders");
    assert_eq!(
        t.panels(),
        [(t.g.clone(), cards(&[&id]))],
        "the user's card is listed in the top panel"
    );
    assert_eq!(t.needs().1, 1, "the user's decision is counted");
    assert!(t.decision_events(&t.p).is_empty());
    assert!(t.decision_events(&t.g).is_empty());
}
