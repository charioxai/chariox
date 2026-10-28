//! The agent's side of an open View: every gate that comes before the page,
//! through the real kernel state, grants and interactions. No browser
//! controller is bound, so a call that passes every gate ends at
//! `view_unavailable`; that is how these tests tell "refused" from "went on".
use super::*;
use crate::{
    agent::CreateAgentRequest, extension::ExtensionGrant, provider::LaunchProviderRequest,
    runtime::router::CommandRouter, session::CreateSessionRequest, DaemonApp, DaemonConfig,
};
use std::sync::Arc;
use tokio::sync::Mutex;

const APP: &str = "installed";
const TARGET: &str = "view-1";
const GENERATION: u64 = 3;

struct Rig {
    runtime: KernelRuntimeState,
    session: String,
    agent: crate::agent::AgentInstance,
    run: crate::provider::RuntimeProviderRun,
    root: std::path::PathBuf,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn rig() -> Rig {
    let root =
        std::env::temp_dir().join(format!("chariox-view-agent-{:016x}", rand::random::<u64>()));
    std::fs::create_dir(&root).unwrap();
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    crate::durable_state::app_state::fixture_catalog(&app.durable_state_store());
    let session = app
        .sessions_mut()
        .create_session(
            CreateSessionRequest::new(root.to_string_lossy(), root.to_string_lossy())
                .with_owner_user_id("alice"),
        )
        .unwrap();
    let agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_owner_user_id("alice"))
        .unwrap();
    let run = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "dev-stub", "dev-stub", "default", "model")
                .with_agent_id(agent.id()),
        )
        .unwrap();
    app.update_provider_run_projection(run.clone());
    // The agent is mid-turn, as it is whenever it calls a tool: the approval is tied to that turn.
    crate::test_support::admit_room_test_turn(&mut app, session.id(), agent.id());
    let session = session.id().to_owned();
    let runtime =
        CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4).runtime_state();
    Rig {
        runtime,
        session,
        agent,
        run,
        root,
    }
}

impl Rig {
    async fn grant(&mut self) {
        self.agent = self
            .runtime
            .grant_agent_extension(self.agent.id(), ExtensionGrant::app(APP), "alice")
            .await
            .unwrap();
    }

    /// A Room view of the installed App. No worker runs in this rig, so a
    /// test names the generation itself.
    fn open_view(&self) -> u64 {
        self.bind(&self.session, GENERATION);
        GENERATION
    }

    fn bind(&self, session: &str, generation: u64) {
        self.runtime.app_control().views().register(
            session,
            TARGET,
            AppViewBinding {
                owner: "alice".into(),
                installation: APP.into(),
                generation,
                panel: Default::default(),
                logical_tab: None,
            },
        );
    }

    /// The context an update-aware catalog would build once R4 signs `ui-actions`.
    fn context(&self, declared: Vec<UiActionDeclaration>) -> ViewContext {
        self.open_view();
        let mut context = self.runtime.bound_view(&self.agent, TARGET).unwrap();
        context.declared = declared;
        context
    }

    fn tool_names(&self) -> Vec<String> {
        self.runtime
            .app_view_tool_specs(std::slice::from_ref(&self.run))
            .into_iter()
            .map(|spec| spec.name)
            .collect()
    }
}

fn action(name: &str, effect: UiActionEffect) -> UiActionDeclaration {
    UiActionDeclaration {
        name: name.into(),
        description: format!("{name} the list"),
        input_schema: json!({"type": "object", "additionalProperties": false, "properties": {"text": {"type": "string"}}}),
        output_schema: None,
        effect,
        approval_reason: (effect == UiActionEffect::Approval).then(|| "Deletes the list".into()),
    }
}

fn declared() -> Vec<UiActionDeclaration> {
    vec![
        action("add_todo", UiActionEffect::Write),
        action("wipe", UiActionEffect::Approval),
    ]
}

async fn pending_approval(rig: &Rig) -> Option<crate::session::RuntimeInteraction> {
    for _ in 0..200 {
        let session = rig
            .runtime
            .owned
            .session_store
            .get_session(&rig.session)
            .unwrap();
        if let Some(found) = session
            .active_interactions()
            .iter()
            .find(|i| i.id().starts_with("app-view-action-"))
        {
            return Some(found.clone());
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    None
}

#[tokio::test]
async fn the_view_tools_are_offered_only_to_an_agent_with_an_app_grant() {
    let mut rig = rig();
    assert!(rig.tool_names().is_empty());
    rig.grant().await;
    assert_eq!(rig.tool_names(), TOOLS);
}

#[tokio::test]
async fn an_agent_reaches_only_its_own_rooms_views_and_only_for_an_app_it_holds() {
    let mut rig = rig();
    rig.open_view();
    let not_granted = rig.runtime.bound_view(&rig.agent, TARGET).err().unwrap();
    assert_eq!(not_granted.code, "AGENT_APP_NOT_GRANTED");
    rig.grant().await;
    assert!(rig.runtime.bound_view(&rig.agent, TARGET).is_ok());
    assert_eq!(
        rig.runtime
            .bound_view(&rig.agent, "no-such-view")
            .err()
            .unwrap()
            .code,
        "unknown_view"
    );
    rig.runtime
        .app_control()
        .views()
        .unbind(&rig.session, TARGET);
    rig.bind("another-room", 1);
    assert_eq!(
        rig.runtime
            .bound_view(&rig.agent, TARGET)
            .err()
            .unwrap()
            .code,
        "unknown_view",
        "another Room's view is not addressable"
    );
    let listed = rig.runtime.view_catalog(&rig.agent).await;
    assert_eq!(listed["views"], json!([]));
}

#[tokio::test]
async fn a_view_that_reloaded_onto_a_newer_generation_stops_the_call_before_the_page() {
    let mut rig = rig();
    rig.grant().await;
    let context = rig.context(declared());
    rig.bind(&rig.session.clone(), GENERATION + 1);
    let refused = rig
        .runtime
        .invoke_declared(&rig.run, &rig.agent, &context, "add_todo", json!({}))
        .await
        .unwrap_err();
    assert_eq!(refused.code, "APP_VIEW_STALE");
}

#[tokio::test]
async fn the_catalog_lists_each_open_view_and_says_why_one_cannot_be_read() {
    let mut rig = rig();
    rig.grant().await;
    rig.open_view();
    let listed = rig.runtime.view_catalog(&rig.agent).await;
    let view = &listed["views"][0];
    assert_eq!(
        (
            view["view"].as_str(),
            view["app"].as_str(),
            view["generation"].as_u64()
        ),
        (Some(TARGET), Some(APP), Some(GENERATION))
    );
    assert_eq!(
        view["unavailable"]["code"], "APP_UNAVAILABLE",
        "no worker runs in this rig, so the signed actions cannot be read"
    );
}

#[tokio::test]
async fn an_undeclared_action_or_bad_input_is_refused_before_the_page() {
    let mut rig = rig();
    rig.grant().await;
    let context = rig.context(declared());
    let run = |name: &'static str, input: Value| {
        let (runtime, agent, run) = (&rig.runtime, &rig.agent, &rig.run);
        let context = &context;
        async move {
            runtime
                .invoke_declared(run, agent, context, name, input)
                .await
                .unwrap_err()
        }
    };
    assert_eq!(run("drop_table", json!({})).await.code, "unknown_action");
    assert_eq!(
        run("add_todo", json!({"text": 5})).await.code,
        "invalid_input"
    );
    assert_eq!(
        run("add_todo", json!({"text": "milk"})).await.code,
        "view_unavailable",
        "a declared action with fitting input goes on to the page"
    );
    assert!(
        pending_approval_none(&rig).await,
        "read and write actions never prompt"
    );
}

async fn pending_approval_none(rig: &Rig) -> bool {
    let session = rig
        .runtime
        .owned
        .session_store
        .get_session(&rig.session)
        .unwrap();
    !session
        .active_interactions()
        .iter()
        .any(|i| i.id().starts_with("app-view-action-"))
}

#[tokio::test]
async fn an_approval_action_asks_the_user_and_a_denial_never_reaches_the_page() {
    let mut rig = rig();
    rig.grant().await;
    let context = rig.context(declared());
    let invoke = rig
        .runtime
        .invoke_declared(&rig.run, &rig.agent, &context, "wipe", json!({}));
    let (outcome, ()) = tokio::join!(invoke, async {
        let asked = pending_approval(&rig).await.expect("the user is asked");
        assert!(
            asked.message().contains("Deletes the list") && asked.message().contains("wipe"),
            "{}",
            asked.message()
        );
        rig.runtime
            .resolve_runtime_interaction(&rig.session, asked.id(), "deny", None)
            .await
            .unwrap();
    });
    assert_eq!(outcome.unwrap_err().code, "approval_denied");
}

#[tokio::test]
async fn an_approved_action_goes_on_to_the_page() {
    let mut rig = rig();
    rig.grant().await;
    let context = rig.context(declared());
    let invoke = rig
        .runtime
        .invoke_declared(&rig.run, &rig.agent, &context, "wipe", json!({}));
    let (outcome, ()) = tokio::join!(invoke, async {
        let asked = pending_approval(&rig).await.expect("the user is asked");
        rig.runtime
            .resolve_runtime_interaction(&rig.session, asked.id(), "allow_once", None)
            .await
            .unwrap();
    });
    assert_eq!(outcome.unwrap_err().code, "view_unavailable");
}

#[tokio::test]
async fn a_revoked_grant_stops_the_call_before_the_page() {
    let mut rig = rig();
    rig.grant().await;
    let context = rig.context(declared());
    rig.runtime
        .revoke_agent_extension(rig.agent.id(), ExtensionKind::App, APP, "alice")
        .await
        .unwrap();
    let refused = rig
        .runtime
        .invoke_declared(&rig.run, &rig.agent, &context, "add_todo", json!({}))
        .await
        .unwrap_err();
    assert_eq!(refused.code, "AGENT_APP_NOT_GRANTED");
}

#[tokio::test]
async fn known_secret_values_never_reach_the_model_in_a_state_a_result_or_an_error() {
    let rig = rig();
    rig.runtime
        .owned
        .room_secret_observations
        .register(&rig.session, "hunter2-vault-value")
        .unwrap();
    let state = rig
        .runtime
        .scrub_answer(
            &rig.session,
            Ok(json!({"todos": ["pay with hunter2-vault-value"], "n": 1})),
        )
        .unwrap();
    assert!(
        !state.to_string().contains("hunter2-vault-value"),
        "{state}"
    );
    assert_eq!(state["n"], 1);
    let error = rig
        .runtime
        .scrub_answer(
            &rig.session,
            Err(failure("APP_REFUSED", "card hunter2-vault-value declined")),
        )
        .unwrap_err();
    assert_eq!(error.code, "APP_REFUSED");
    assert!(
        !error.message.contains("hunter2-vault-value"),
        "{}",
        error.message
    );
}

// MP-08/MP-10/MP-11: reuse the existing kernel interaction's lasting choice.
#[tokio::test]
async fn an_existing_session_action_approval_skips_the_next_prompt_and_revocation_fences_it() {
    let mut rig = rig();
    rig.grant().await;
    let context = rig.context(declared());
    let action = action("wipe", UiActionEffect::Approval);
    let input = json!({});
    let approve = rig
        .runtime
        .approve_view_action(&rig.run, &rig.agent, &context, &action, &input);
    let (outcome, lasting) = tokio::join!(approve, async {
        let asked = pending_approval(&rig)
            .await
            .expect("the user is asked once");
        let lasting = asked.choices().iter().any(|c| c.id() == "allow_session");
        rig.runtime
            .resolve_runtime_interaction(
                &rig.session,
                asked.id(),
                if lasting { "allow_session" } else { "deny" },
                None,
            )
            .await
            .unwrap();
        lasting
    });
    assert!(
        lasting,
        "offer the existing Allow for session permission choice"
    );
    outcome.unwrap();
    let repeated = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        rig.runtime
            .approve_view_action(&rig.run, &rig.agent, &context, &action, &input),
    )
    .await;
    assert!(
        matches!(repeated, Ok(Ok(()))),
        "an existing approval adds no prompt: {repeated:?}"
    );
    assert!(pending_approval_none(&rig).await);
    // A lasting answer belongs to the Room, not to an open page or its pump.
    rig.runtime
        .app_control()
        .views()
        .forget_session(&rig.session);
    assert!(!rig.runtime.app_control().views().keep_pumping(&rig.session));
    let context = rig.context(declared());
    assert!(tokio::time::timeout(
        std::time::Duration::from_millis(200),
        rig.runtime
            .approve_view_action(&rig.run, &rig.agent, &context, &action, &input)
    )
    .await
    .unwrap()
    .is_ok());

    rig.runtime
        .revoke_agent_extension(rig.agent.id(), ExtensionKind::App, APP, "alice")
        .await
        .unwrap();
    rig.grant().await;
    assert_eq!(
        rig.runtime
            .approve_view_action(&rig.run, &rig.agent, &context, &action, &input)
            .await
            .unwrap_err()
            .code,
        "AGENT_APP_NOT_GRANTED"
    );
    let context = rig.context(declared());
    let approve = rig
        .runtime
        .approve_view_action(&rig.run, &rig.agent, &context, &action, &input);
    let (outcome, ()) = tokio::join!(approve, async {
        let asked = pending_approval(&rig)
            .await
            .expect("regrant must ask again");
        rig.runtime
            .resolve_runtime_interaction(&rig.session, asked.id(), "deny", None)
            .await
            .unwrap();
    });
    assert_eq!(outcome.unwrap_err().code, "approval_denied");
}

async fn decide_action_approval(
    rig: &Rig,
    context: &ViewContext,
    action: &UiActionDeclaration,
    choice: &str,
) -> Result<(), ViewError> {
    let input = json!({});
    let approve = rig
        .runtime
        .approve_view_action(&rig.run, &rig.agent, context, action, &input);
    let (outcome, ()) = tokio::join!(approve, async {
        let asked = pending_approval(rig)
            .await
            .expect("this action needs its own approval");
        rig.runtime
            .resolve_runtime_interaction(&rig.session, asked.id(), choice, None)
            .await
            .unwrap();
    });
    outcome
}

// MP-08/MP-10/MP-11: allow once never becomes lasting authority, and an answer
// for one signed action/generation cannot approve another.
#[tokio::test]
async fn action_approvals_keep_their_choice_action_and_generation_scope() {
    let mut rig = rig();
    rig.grant().await;
    let mut context = rig.context(declared());
    let wipe = action("wipe", UiActionEffect::Approval);
    decide_action_approval(&rig, &context, &wipe, "allow_once")
        .await
        .unwrap();
    assert_eq!(
        decide_action_approval(&rig, &context, &wipe, "deny")
            .await
            .unwrap_err()
            .code,
        "approval_denied"
    );
    decide_action_approval(&rig, &context, &wipe, "allow_session")
        .await
        .unwrap();
    assert_eq!(
        decide_action_approval(
            &rig,
            &context,
            &action("another_wipe", UiActionEffect::Approval),
            "deny"
        )
        .await
        .unwrap_err()
        .code,
        "approval_denied"
    );
    context.binding.generation += 1;
    rig.bind(&rig.session, context.binding.generation);
    assert_eq!(
        decide_action_approval(&rig, &context, &wipe, "deny")
            .await
            .unwrap_err()
            .code,
        "approval_denied"
    );
}
