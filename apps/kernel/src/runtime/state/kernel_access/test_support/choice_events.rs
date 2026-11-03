//! The pending decision tells an outside agent its question, and the close
//! tells how it ended; the refusal rule is the same for the event and the grant.
use super::*;
use crate::session::{
    RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionChoiceStyle,
    RuntimeInteractionCustomChoice, RuntimeInteractionKind, RuntimeInteractionLevel,
};

struct Fixture {
    state: KernelRuntimeState,
    session_id: String,
    agent_id: String,
    owner: String,
    grant: String,
    _workspace: crate::test_support::TestWorktree,
}

fn fixture(name: &str) -> Fixture {
    let workspace = crate::test_support::TestWorktree::new(name);
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(workspace.session_request())
        .unwrap();
    let state =
        crate::runtime::state::workflow_prompt_queue_owned_state::tests::runtime_state_from_app(
            app,
        );
    let grant = state.insert_access_grant_for_test(session.id());
    let target = EventTarget {
        kernel: state.owned.config_projection.snapshot().daemon_id,
        session_id: session.id().into(),
        agent_id: Some(agent.id().into()),
    };
    state
        .subscribe_outside_part(&grant, true, vec![target])
        .unwrap();
    Fixture {
        owner: session.owner_user_id().into(),
        session_id: session.id().into(),
        agent_id: agent.id().into(),
        state,
        grant,
        _workspace: workspace,
    }
}

impl Fixture {
    fn interaction(&self, id: &str, level: RuntimeInteractionLevel) -> RuntimeInteraction {
        self.interaction_with(id, level, None)
    }

    fn interaction_with(
        &self,
        id: &str,
        level: RuntimeInteractionLevel,
        custom: Option<RuntimeInteractionCustomChoice>,
    ) -> RuntimeInteraction {
        RuntimeInteraction::new(
            id,
            &self.agent_id,
            RuntimeInteractionKind::Choice,
            level,
            Some("Which colour?".to_string()),
            "Pick one",
            vec![
                RuntimeInteractionChoice::new("red", "Red", "red", None),
                RuntimeInteractionChoice::new(
                    "blue",
                    "Blue",
                    "blue",
                    Some(RuntimeInteractionChoiceStyle::Primary),
                ),
            ],
            custom,
            Some(300),
            Some("red".to_string()),
        )
    }

    fn register(
        &self,
        interaction: RuntimeInteraction,
        credential_owner: Option<&str>,
    ) -> tokio::sync::oneshot::Receiver<crate::runtime::state::PendingInteractionResolution> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.state
            .owned
            .register_runtime_interaction(
                &self.session_id,
                interaction,
                tx,
                None,
                None,
                credential_owner,
            )
            .unwrap();
        rx
    }

    fn interaction_events(&self) -> Vec<serde_json::Value> {
        self.state
            .outside_store()
            .outside_list(&self.grant, None, 100)
            .unwrap()
            .unwrap()
            .events
            .into_iter()
            .filter(|event| event.refs.interaction_id.is_some())
            .map(|event| serde_json::to_value(event).unwrap())
            .collect()
    }

    fn event(&self, id: &str, state: &str) -> serde_json::Value {
        self.interaction_events()
            .into_iter()
            .find(|event| event["refs"]["interaction_id"] == id && event["value"]["state"] == state)
            .unwrap_or_else(|| panic!("no {state} event for {id}"))
    }

    fn respond(&self, id: &str, choice: &str) -> LocalDaemonRequest {
        LocalDaemonRequest::RespondToInteraction(
            serde_json::from_value(serde_json::json!({
                "session_id": self.session_id,
                "interaction_id": id,
                "choice_id": choice,
            }))
            .unwrap(),
        )
    }

    async fn answer(
        &self,
        id: &str,
        choice: &str,
        class: crate::local::KernelConnectionClass,
    ) -> Result<(), DaemonError> {
        self.state
            .answer_terminal_runtime_interaction(
                &self.session_id,
                id,
                choice,
                None,
                Some(&self.owner),
                None,
                None,
                Some(class),
            )
            .await
    }
}

#[tokio::test]
async fn a_routine_choice_is_told_with_its_question() {
    let f = fixture("choice-question");
    let interaction = f.interaction_with(
        "routine",
        RuntimeInteractionLevel::Info,
        Some(RuntimeInteractionCustomChoice::new(
            "other",
            "Something else",
            Some("type here".into()),
            Some(1),
            Some(80),
        )),
    );
    let _rx = f.register(interaction, None);
    let pending = f.event("routine", "pending");
    let question = &pending["value"]["question"];
    assert_eq!(question["title"], "Which colour?");
    assert_eq!(question["message"], "Pick one");
    assert_eq!(
        question["choices"],
        serde_json::json!([
            {"id": "red", "label": "Red"},
            {"id": "blue", "label": "Blue", "style": "primary"},
        ]),
        "ids, labels and styles only: no reply text, no passkey flag"
    );
    assert_eq!(question["text_reply"]["id"], "other");
    assert_eq!(question["text_reply"]["max_length"], 80);
    assert_eq!(question["default_on_timeout"], "red");
    assert!(question["expires_at_ms"].as_u64().unwrap() > 0);
    assert!(pending["value"].get("pointer").is_none());
    f.state
        .authorize_external_request(&f.grant, &f.respond("routine", "blue"))
        .expect("the holder can answer what it is shown");
}

#[tokio::test]
async fn a_decision_the_holder_cannot_answer_has_a_pointer_and_no_question() {
    // One Room agent holds one open decision at a time, so each kind gets its own Room.
    for (n, id) in [
        "critical",
        "secret-entry",
        "provider-auth-recovery:codex",
        "handoff-1",
        "credential-owner",
    ]
    .into_iter()
    .enumerate()
    {
        let f = fixture(&format!("choice-refused-{n}"));
        let routine = RuntimeInteractionLevel::Info;
        let (interaction, credential_owner) = match id {
            "critical" => (f.interaction(id, RuntimeInteractionLevel::Critical), None),
            "secret-entry" => (
                f.interaction_with(
                    id,
                    routine,
                    Some(RuntimeInteractionCustomChoice::secret(
                        "value",
                        "Secret",
                        None,
                        Some(1),
                        Some(64),
                    )),
                ),
                None,
            ),
            "credential-owner" => (f.interaction(id, routine), Some(f.owner.clone())),
            _ => (f.interaction(id, routine), None),
        };
        let _rx = f.register(interaction, credential_owner.as_deref());
        let pending = f.event(id, "pending");
        assert!(
            pending["value"].get("question").is_none(),
            "{id}: a decision the kernel refuses to a holder is not offered as a question"
        );
        assert_eq!(pending["summary"], "owner approval", "{id}");
        assert!(pending["value"]["pointer"]["one_line"].is_string(), "{id}");
        assert!(
            f.state
                .authorize_external_request(&f.grant, &f.respond(id, "red"))
                .is_err(),
            "{id}: the grant is refused too, by the same rule"
        );
    }
}

#[tokio::test]
async fn the_close_says_how_the_decision_ended() {
    use crate::local::KernelConnectionClass::{ExternalAgent, Terminal};
    for (n, (via, choice)) in [
        ("terminal", "blue"),
        ("external_agent", "red"),
        ("timeout", "red"),
    ]
    .into_iter()
    .enumerate()
    {
        let f = fixture(&format!("choice-resolution-{n}"));
        let _rx = f.register(
            f.interaction("closing", RuntimeInteractionLevel::Info),
            None,
        );
        match via {
            "terminal" => f.answer("closing", choice, Terminal).await.unwrap(),
            "external_agent" => f.answer("closing", choice, ExternalAgent).await.unwrap(),
            _ => f
                .state
                .timeout_runtime_interaction(&f.session_id, "closing")
                .await
                .unwrap(),
        }
        let resolved = f.event("closing", "resolved");
        assert_eq!(resolved["value"]["resolution"]["via"], via);
        assert_eq!(
            resolved["value"]["resolution"]["choice_id"], choice,
            "{via}"
        );
        assert!(resolved["value"].get("question").is_none(), "{via}");
    }
}

#[tokio::test]
async fn a_second_answer_gets_interaction_closed_and_one_audit_record() {
    use crate::local::KernelConnectionClass::{ExternalAgent, Terminal};
    let f = fixture("choice-first-wins");
    let _rx = f.register(f.interaction("race", RuntimeInteractionLevel::Info), None);
    f.answer("race", "blue", ExternalAgent).await.unwrap();
    let late = f.answer("race", "red", Terminal).await.unwrap_err();
    assert_eq!(
        crate::transport::kernel_protocol::map_kernel_error(&late).code,
        "interaction_closed",
        "{late}"
    );
    let audit = f
        .state
        .owned
        .durable_state_store
        .load_subject_events_by_kind("race", "interaction.answered", 10)
        .unwrap();
    assert_eq!(audit.len(), 1, "only the winning answer is recorded");
    assert_eq!(audit[0].payload["choice_id"], "blue");
    assert_eq!(audit[0].payload["connection_class"], "external_agent");
    assert!(
        !audit[0].payload.to_string().contains("reply"),
        "the reply text is never recorded"
    );
}
