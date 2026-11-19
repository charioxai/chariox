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

impl Tree {
    /// `agent` asks again in the turn its task already runs, as a retried tool call does.
    async fn ask_with_key(&self, agent: &str, key: &str) -> serde_json::Value {
        let token = self
            .runtime
            .owned
            .provider_store
            .get_run_for_agent(&self.room, agent)
            .unwrap()
            .runtime_mcp_auth_token()
            .unwrap()
            .to_owned();
        self.runtime
            .dispatch_authenticated_runtime_tool_call(
                &token,
                crate::transport::runtime_tools::REQUEST_POPUP_TOOL,
                serde_json::json!({"wait": false, "idempotency_key": key, "message": "Deploy now?",
                    "choices": [{"id": "yes", "label": "Yes", "reply": "Deploy."}]}),
            )
            .await
            .unwrap()
            .payload
    }
}

/// A retried ask (same idempotency key) is the same card: one interaction, one holder notice.
#[tokio::test]
async fn a_retried_ask_with_the_same_key_is_not_raised_twice() {
    let t = tree(
        "decision-async-retry",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    t.begin_task(&t.c);

    let first = t.ask_with_key(&t.c, "deploy-1").await;
    let again = t.ask_with_key(&t.c, "deploy-1").await;

    assert_eq!(again["interaction_id"], first["interaction_id"]);
    assert_eq!(again["deduplicated"], true);
    assert_eq!(t.decision_events(&t.p).len(), 1, "the holder is told once");
}

impl Tree {
    /// A grant of the owner follows "Agent needs your action"; returns it.
    fn follow_owner_alerts(&self) -> String {
        let grant = self.runtime.insert_access_grant_for_test(&self.room);
        let target = crate::local::EventTarget {
            kernel: self.runtime.owned.config_projection.snapshot().daemon_id,
            session_id: self.room.clone(),
            agent_id: Some(self.c.clone()),
        };
        self.runtime
            .subscribe_outside_part(&grant, true, vec![target])
            .unwrap();
        let session = self.runtime.owned.session_store.get_session(&self.room).unwrap();
        let owner = session.owner_user_id().to_owned();
        let source = crate::runtime::state::owner_action_notifications::owner_action_source(&session)
            .source_id;
        self.runtime.followable_notification_sources(&owner).unwrap();
        self.runtime
            .follow_notification_sources(&grant, &owner, &[source])
            .unwrap();
        grant
    }

    /// The text of every owner-action occurrence the grant has been told.
    fn alerts(&self, grant: &str) -> Vec<serde_json::Value> {
        self.runtime
            .outside_store()
            .outside_list(grant, None, 100)
            .unwrap()
            .unwrap()
            .events
            .into_iter()
            .filter_map(|event| match event.value {
                crate::local::OutsideEventValue::Notification { output, .. } => {
                    Some(serde_json::from_str(output.message()).unwrap())
                }
                _ => None,
            })
            .collect()
    }
}

/// A question the person holds wakes the followers of "Agent needs your action" with its words.
#[tokio::test]
async fn a_question_asked_without_waiting_reaches_followers_of_owner_alerts_with_its_text() {
    let t = tree(
        "decision-async-alert",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    t.end_turn_yielded(&t.p).await;
    t.end_turn_yielded(&t.g).await;
    let grant = t.follow_owner_alerts();

    let id = t.ask_through_popup(&t.c).await;

    let alerts = t.alerts(&grant);
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!(alerts[0]["interaction_id"], id);
    assert_eq!(alerts[0]["question"]["message"], "Deploy now?");
}

/// A question an agent holds tells the person's followers only when it is handed up to the person.
#[tokio::test]
async fn a_question_an_agent_holds_alerts_followers_only_once_it_reaches_the_person() {
    let t = tree(
        "decision-async-alert-up",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let grant = t.follow_owner_alerts();
    let id = t.ask_through_popup(&t.c).await;
    assert!(t.alerts(&grant).is_empty(), "held by its parent");

    t.end_turn_yielded(&t.p).await;
    t.end_turn_yielded(&t.g).await;

    let alerts = t.alerts(&grant);
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!(alerts[0]["interaction_id"], id);
}
