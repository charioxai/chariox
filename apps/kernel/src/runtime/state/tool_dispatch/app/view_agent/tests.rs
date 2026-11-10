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
    rig_with(true, false)
}

fn rig_with_installation(install_fixture: bool) -> Rig {
    rig_with(install_fixture, false)
}

fn rig_for_catalog(with_tools: bool) -> Rig {
    rig_with(true, with_tools)
}

fn rig_with(install_fixture: bool, with_tools: bool) -> Rig {
    let root =
        std::env::temp_dir().join(format!("chariox-view-agent-{:016x}", rand::random::<u64>()));
    std::fs::create_dir(&root).unwrap();
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    if with_tools {
        crate::durable_state::app_state::fixture_tool_catalog(&app.durable_state_store());
    } else if install_fixture {
        crate::durable_state::app_state::fixture_catalog(&app.durable_state_store());
    }
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

    fn installation_generation(&self) -> u64 {
        self.runtime
            .owned
            .durable_state_store
            .get_app_installation("alice", APP)
            .unwrap()
            .generation
    }

    fn open_user_view(&self, queue_generation: Option<u64>) -> String {
        self.open_user_view_built_for(self.installation_generation(), queue_generation)
    }

    /// A view whose page was built for `generation`, as one opened before an update.
    fn open_user_view_built_for(&self, generation: u64, queue_generation: Option<u64>) -> String {
        let binding = AppViewBinding {
            owner: "alice".into(),
            installation: APP.into(),
            generation,
            panel: Default::default(),
            logical_tab: None,
        };
        let views = self.runtime.app_control().user_views().clone();
        let view = views.open("alice", binding.clone(), "test").unwrap();
        views
            .bind_browser(
                "alice",
                &view.view_id,
                "registry-bound-browser-target".into(),
                crate::local::UserAppViewBrowser {
                    tab_id: "host-tab-native-test".into(),
                    generation: 1,
                },
            )
            .unwrap();
        let mut queue_binding = binding;
        queue_binding.generation = queue_generation.unwrap_or(generation);
        self.runtime
            .app_control()
            .views()
            .register(&view.view_id, &view.view_id, queue_binding);
        view.view_id
    }

    fn stage_signed_v2_fixture(&self) {
        use chariox_app_package::{verify, VerificationPolicy};
        use chariox_app_runtime::release_store::{ReleaseStore, StageBudget};

        let store = &self.runtime.owned.durable_state_store;
        let (bytes, publisher) = crate::durable_state::app_state::fixture_tool_package();
        let package = verify(
            &bytes,
            &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
        )
        .unwrap();
        ReleaseStore::open_or_create(store.path())
            .unwrap()
            .stage(
                &package,
                &bytes,
                StageBudget {
                    max_stage_bytes: 1024 * 1024,
                    reserved_bytes: 1024 * 1024,
                    host_reserve_bytes: 1024 * 1024,
                },
            )
            .unwrap();
    }

    /// Starts the worker and waits until it can be leased: the start returns
    /// while the worker still comes up on its own thread.
    async fn start_signed_v2_fixture_worker(&self) {
        self.runtime
            .app_control()
            .lifecycle()
            .fixture_inbox_workers_with_mode(
                chariox_app_runtime::worker_process::test_fixture::Mode::ToolEcho,
            );
        self.runtime
            .app_control()
            .lifecycle()
            .start_active_blocking("alice", APP, tokio::runtime::Handle::current())
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while self
                .runtime
                .app_control()
                .active_app_lease("alice", APP)
                .is_none()
            {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the fixture worker became leasable");
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_user_app_view_uses_its_owner_bound_target_and_kernel_browser_host() {
    use std::os::unix::fs::PermissionsExt;

    let mut rig = rig_for_catalog(true);
    rig.grant().await;
    rig.stage_signed_v2_fixture();
    let view_id = rig.open_user_view(None);
    rig.start_signed_v2_fixture_worker().await;

    let controller = rig.root.join("native-app-view-controller.sh");
    let observed = rig.root.join("native-app-view-request.json");
    std::fs::write(
        &controller,
        format!(
            r#"#!/bin/sh
while IFS= read -r request; do
  id=${{request#*:}}; id=${{id%%,*}}
  case "$request" in
    *'"method":"health"'*) printf '{{"id":%s,"ok":true,"result":{{"state":"ready","process_id":%s,"diagnostic_code":null}}}}\n' "$id" "$$" ;;
    *'"method":"browser.app.agent"'*) printf '%s\n' "$request" > '{}'; printf '{{"id":%s,"ok":true,"result":{{"ok":true,"value":{{"native":true}}}}}}\n' "$id" ;;
    *) printf '{{"id":%s,"ok":true,"result":{{}}}}\n' "$id" ;;
  esac
done
"#,
            observed.display()
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

    let context = rig
        .runtime
        .view_context(&rig.agent, "host-tab-native-test")
        .await
        .unwrap();
    assert_eq!(context.queue_session, view_id);
    assert!(
        matches!(&context.host, ViewHost::User { browser_target, .. }
        if browser_target == "registry-bound-browser-target")
    );
    assert_eq!(
        rig.runtime
            .view_call(&context, ViewAgentCallKind::State, None, None)
            .await
            .unwrap(),
        json!({"native":true})
    );
    let request = std::fs::read_to_string(observed).unwrap();
    assert!(
        request.contains("registry-bound-browser-target"),
        "{request}"
    );
    assert!(request.contains(rig.agent.id()), "{request}");
}

#[tokio::test]
async fn native_user_app_view_requires_the_app_grant_and_current_queue_generation() {
    let ungranted_rig = rig_for_catalog(true);
    ungranted_rig.stage_signed_v2_fixture();
    ungranted_rig.open_user_view(None);
    assert_eq!(
        ungranted_rig
            .runtime
            .view_context(&ungranted_rig.agent, "host-tab-native-test")
            .await
            .err()
            .unwrap()
            .code,
        "unknown_view",
        "an owner tab is not an App grant"
    );

    let mut rig = rig_for_catalog(true);
    rig.grant().await;
    rig.stage_signed_v2_fixture();
    let generation = rig
        .runtime
        .owned
        .durable_state_store
        .get_app_installation("alice", APP)
        .unwrap()
        .generation;
    rig.open_user_view(Some(generation + 1));
    assert_eq!(
        rig.runtime
            .view_context(&rig.agent, "host-tab-native-test")
            .await
            .err()
            .unwrap()
            .code,
        "APP_VIEW_STALE",
        "a user view whose registered App generation changed is refused"
    );
}

/// MP-08/MP-11 S1-11: an uninstall forgets the Room-side queue binding of an open
/// user view; the reinstall reloads the same Tab onto the new generation, and the
/// agent must be able to use that view again without opening it a second time.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_user_view_reloaded_after_uninstall_and_reinstall_is_usable_by_the_agent_again() {
    use std::os::unix::fs::PermissionsExt;

    let mut rig = rig_for_catalog(true);
    rig.grant().await;
    rig.stage_signed_v2_fixture();
    let generation = rig.installation_generation();
    let view_id = rig.open_user_view_built_for(generation - 1, None);
    rig.start_signed_v2_fixture_worker().await;
    let controller = rig.root.join("reinstall-view-controller.sh");
    std::fs::write(
        &controller,
        r#"#!/bin/sh
while IFS= read -r request; do
  id=${request#*:}; id=${id%%,*}
  case "$request" in
    *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
    *'"method":"browser.app.agent"'*) printf '{"id":%s,"ok":true,"result":{"ok":true,"value":{"native":true}}}\n' "$id" ;;
    *) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
  esac
done
"#,
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

    // Uninstall: the Room registry forgets the installation's views; the open Tab stays.
    rig.runtime
        .app_control()
        .views()
        .forget_installation("alice", APP);
    // Reinstall: the poll reloads the retained view onto the new generation.
    let listed = rig.runtime.app_control().user_views().list("alice");
    let view = listed.iter().find(|v| v.view_id == view_id).unwrap();
    rig.runtime
        .reload_updated_user_app_view("alice", view)
        .await;
    let reloaded = rig
        .runtime
        .app_control()
        .user_views()
        .get("alice", &view_id);
    assert_eq!(
        reloaded.unwrap().0.generation,
        generation,
        "the Tab moved to the new generation"
    );

    let context = rig
        .runtime
        .view_context(&rig.agent, "host-tab-native-test")
        .await
        .expect("the agent reaches the reloaded view");
    assert_eq!(context.binding.generation, generation);
    assert_eq!(
        rig.runtime
            .view_call(&context, ViewAgentCallKind::State, None, None)
            .await
            .unwrap(),
        json!({"native":true})
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_user_app_view_refuses_actions_absent_from_the_signed_declarations() {
    let mut rig = rig_for_catalog(true);
    rig.grant().await;
    rig.stage_signed_v2_fixture();
    rig.open_user_view(None);
    rig.start_signed_v2_fixture_worker().await;
    let context = rig
        .runtime
        .view_context(&rig.agent, "host-tab-native-test")
        .await
        .unwrap();
    let error = rig
        .runtime
        .invoke_declared(
            rig.run.id(),
            &rig.agent,
            &context,
            "not_declared",
            json!({}),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "unknown_action");
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
    assert_eq!(listed["views"].as_array().unwrap().len(), 1);
    assert_eq!(listed["views"][0]["app"], APP);
    assert!(listed["views"][0].get("unavailable").is_some());
    assert!(
        listed["views"][0].get("state").is_none(),
        "another Room's view is not opened or read"
    );
}

#[tokio::test]
async fn a_room_view_of_another_owners_installation_is_not_agent_authority() {
    let mut rig = rig();
    rig.grant().await;
    rig.runtime.app_control().views().register(
        &rig.session,
        TARGET,
        AppViewBinding {
            owner: "bob".into(),
            installation: APP.into(),
            generation: GENERATION,
            panel: Default::default(),
            logical_tab: None,
        },
    );
    for handle in [TARGET, APP] {
        assert_eq!(
            rig.runtime
                .bound_view(&rig.agent, handle)
                .err()
                .unwrap()
                .code,
            "unknown_view"
        );
    }
}

#[tokio::test]
async fn a_view_that_reloaded_onto_a_newer_generation_stops_the_call_before_the_page() {
    let mut rig = rig();
    rig.grant().await;
    let context = rig.context(declared());
    rig.bind(&rig.session.clone(), GENERATION + 1);
    let refused = rig
        .runtime
        .invoke_declared(rig.run.id(), &rig.agent, &context, "add_todo", json!({}))
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
        (Some(APP), Some(APP), Some(GENERATION))
    );
    assert_eq!(
        view["unavailable"]["code"], "view_unavailable",
        "a cached binding does not prove this rig has a running Room controller"
    );
}

#[tokio::test]
async fn the_catalog_lists_an_unopened_view_as_openable() {
    let mut rig = rig_for_catalog(true);
    rig.grant().await;
    rig.stage_signed_v2_fixture();
    let listed = rig.runtime.view_catalog(&rig.agent).await;
    let view = &listed["views"][0];
    assert_eq!(view["app"], APP);
    assert_eq!(view["openable"], true, "{view}");
    assert!(
        view.get("unavailable").is_none(),
        "an unopened view is not unavailable: {view}"
    );
}

#[tokio::test]
async fn an_agent_whose_app_was_uninstalled_gets_a_definitive_tool_error() {
    let mut rig = rig();
    rig.grant().await;
    let generation = rig.installation_generation();
    rig.runtime
        .owned
        .durable_state_store
        .mutate_app_installation(
            "alice",
            crate::durable_state::apps::AppRegistryMutation::Uninstall {
                installation_id: APP.into(),
                expected_generation: generation,
                now_ms: 5,
            },
        )
        .unwrap();
    assert_eq!(
        rig.tool_names(),
        TOOLS,
        "the kept grant keeps the view tools, so the agent can be told"
    );
    let call = |name: &'static str, arguments: Value| {
        rig.runtime
            .app_view_tool_for_run_id(rig.run.id(), &rig.agent, name, arguments)
    };
    let refused = call(STATE, json!({"view": APP})).await.unwrap_err();
    assert_eq!(refused.code, "APP_NOT_INSTALLED");
    let listed = call(CATALOG, json!({})).await.unwrap();
    assert_eq!(
        listed["views"][0]["unavailable"]["code"],
        "APP_NOT_INSTALLED"
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
                .invoke_declared(run.id(), agent, context, name, input)
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
        .invoke_declared(rig.run.id(), &rig.agent, &context, "wipe", json!({}));
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

/// An App view the agent holds but has not claimed cannot run the action, so
/// the owner is not asked to approve it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unclaimed_user_view_is_refused_before_any_approval_card() {
    let mut rig = rig_for_catalog(true);
    rig.grant().await;
    rig.stage_signed_v2_fixture();
    let view_id = rig.open_user_view(None);
    rig.start_signed_v2_fixture_worker().await;
    let host = rig.runtime.kernel_browser_host_for_test();
    host.set_focus("alice", Some(rig.agent.id()));
    host.load("alice", rig.agent.id()).unwrap();
    let mut context = rig
        .runtime
        .view_context(&rig.agent, "host-tab-native-test")
        .await
        .unwrap();
    context.declared = declared();

    let invoke = rig
        .runtime
        .invoke_declared(rig.run.id(), &rig.agent, &context, "wipe", json!({}));
    match tokio::time::timeout(std::time::Duration::from_secs(3), invoke).await {
        Ok(outcome) => {
            let refused = outcome.unwrap_err();
            assert_eq!(refused.code, "AGENT_APP_NOT_GRANTED");
            assert!(refused.message.contains("claim"), "{}", refused.message);
            assert!(pending_approval_none(&rig).await);
        }
        Err(_) => panic!(
            "the owner was asked to approve an action on an unclaimed view (card raised: {})",
            !pending_approval_none(&rig).await
        ),
    }

    // Claiming the view, as a click on its Tab does, lets the one card through.
    let admission = host.admit("alice", rig.agent.id()).unwrap();
    host.claim_resource(
        Some(&admission),
        crate::local::UserDomainResource::AppView { view_id },
        false,
    )
    .unwrap();
    let invoke = rig
        .runtime
        .invoke_declared(rig.run.id(), &rig.agent, &context, "wipe", json!({}));
    let (outcome, ()) = tokio::join!(invoke, async {
        let asked = pending_approval(&rig)
            .await
            .expect("the claimed view asks the user");
        rig.runtime
            .resolve_runtime_interaction(&rig.session, asked.id(), "allow_once", None)
            .await
            .unwrap();
    });
    assert_ne!(outcome.unwrap_err().code, "AGENT_APP_NOT_GRANTED");
}

#[tokio::test]
async fn an_approved_action_goes_on_to_the_page() {
    let mut rig = rig();
    rig.grant().await;
    let context = rig.context(declared());
    let invoke = rig
        .runtime
        .invoke_declared(rig.run.id(), &rig.agent, &context, "wipe", json!({}));
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
        .invoke_declared(rig.run.id(), &rig.agent, &context, "add_todo", json!({}))
        .await
        .unwrap_err();
    assert_eq!(refused.code, "AGENT_APP_NOT_GRANTED");
}

#[tokio::test]
async fn known_secret_values_never_reach_the_model_in_a_state_a_result_or_an_error() {
    let mut rig = rig();
    rig.grant().await;
    let context = rig.context(declared());
    rig.runtime
        .owned
        .room_secret_observations
        .register(&rig.session, "hunter2-vault-value")
        .unwrap();
    let state = rig
        .runtime
        .scrub_answer(
            &context,
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
            &context,
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
    let approve =
        rig.runtime
            .approve_view_action(rig.run.id(), &rig.agent, &context, &action, &input);
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
            .approve_view_action(rig.run.id(), &rig.agent, &context, &action, &input),
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
            .approve_view_action(rig.run.id(), &rig.agent, &context, &action, &input)
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
            .approve_view_action(rig.run.id(), &rig.agent, &context, &action, &input)
            .await
            .unwrap_err()
            .code,
        "AGENT_APP_NOT_GRANTED"
    );
    let context = rig.context(declared());
    let approve =
        rig.runtime
            .approve_view_action(rig.run.id(), &rig.agent, &context, &action, &input);
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
    let approve =
        rig.runtime
            .approve_view_action(rig.run.id(), &rig.agent, context, action, &input);
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

#[path = "installed.rs"]
mod installed;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mp08_a3_catalog_opens_granted_app_without_a_human_view() {
    check_live_binding(LiveBindingCheck::Catalog).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mp08_a3_page_panel_survives_agent_state_on_an_adopted_tab() {
    check_live_binding(LiveBindingCheck::Panel).await;
}

// MP-08/MP-10/MP-11: page-controlled registration failures cross the same
// observation boundary as action results, including the stable-handle probe.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mp08_a3_registration_errors_are_scrubbed_on_every_agent_path() {
    check_live_binding(LiveBindingCheck::RegistrationErrors).await;
}

// MP-08/MP-09/MP-10/MP-11: a failed idle close keeps the person's restore intent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mp08_a3_failed_retirement_keeps_the_adopted_tabs_restore_intent() {
    check_live_binding(LiveBindingCheck::FailedRetirement).await;
}

// MP-08/MP-09/MP-10/MP-11: retirement must settle bindings before the drain.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mp08_a3_successful_retirement_unbinds_the_old_adopted_tab_on_reopen() {
    check_live_binding(LiveBindingCheck::SuccessfulRetirement).await;
}

#[derive(PartialEq)]
enum LiveBindingCheck {
    Catalog,
    Panel,
    RegistrationErrors,
    FailedRetirement,
    SuccessfulRetirement,
}

async fn check_live_binding(check: LiveBindingCheck) {
    use crate::runtime::browser_controller_process::BrowserControllerProcessStore;
    use chariox_app_package::{verify, VerificationPolicy};
    use chariox_app_runtime::release_store::{ReleaseStore, StageBudget};
    let mut rig = rig_for_catalog(true);
    rig.grant().await;
    let store = rig.runtime.owned.durable_state_store.clone();
    let (bytes, publisher) = crate::durable_state::app_state::fixture_tool_package();
    let package = verify(
        &bytes,
        &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
    )
    .unwrap();
    ReleaseStore::open_or_create(store.path())
        .unwrap()
        .stage(
            &package,
            &bytes,
            StageBudget {
                max_stage_bytes: 1024 * 1024,
                reserved_bytes: 1024 * 1024,
                host_reserve_bytes: 1024 * 1024,
            },
        )
        .unwrap();
    rig.runtime
        .app_control()
        .lifecycle()
        .fixture_inbox_workers_with_mode(
            chariox_app_runtime::worker_process::test_fixture::Mode::ToolEcho,
        );
    rig.runtime
        .app_control()
        .lifecycle()
        .start_active_blocking("alice", APP, tokio::runtime::Handle::current())
        .unwrap();
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("slice-linux-docker/docker");
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/runtime/state/app-view-actor.fixture.mjs");
    let controllers = BrowserControllerProcessStore::new(
        "node",
        vec![
            fixture.display().to_string(),
            directory.display().to_string(),
            rig.root.display().to_string(),
        ],
        std::time::Duration::from_secs(3),
    );
    controllers.acquire(&rig.session).unwrap();
    rig.runtime
        .set_browser_controller_process_store_for_test(controllers.clone());
    let result = std::panic::AssertUnwindSafe(async {
        let listed = rig.runtime.view_catalog(&rig.agent).await;
        assert_eq!(
            listed["views"].as_array().unwrap().len(),
            1,
            "a granted App opens with every viewer closed: {listed}"
        );
        let view = &listed["views"][0];
        assert_eq!(view["app"], APP);
        assert_eq!(
            view["view"], APP,
            "the installation is a stable handle: {listed}"
        );
        assert_eq!(
            rig.runtime.app_control().views().bound_views(&rig.session)[0].0,
            "actor-target",
            "the production controller was reached"
        );
        assert!(
            view.get("state").is_some(),
            "the state bridge answered: {listed}"
        );
        assert!(
            view["state"].get("unavailable").is_none(),
            "the controller answered state: {listed}"
        );
        if check == LiveBindingCheck::RegistrationErrors {
            const SECRET: &str = "registration-vault-value";
            rig.runtime
                .owned
                .room_secret_observations
                .register(&rig.session, SECRET)
                .unwrap();
            std::fs::write(rig.root.join("registration-error"), "").unwrap();
            for (tool, arguments) in [
                (STATE, json!({"view": APP})),
                (CHANGES, json!({"view": APP})),
                (BATCH, json!({"view": APP, "steps": []})),
            ] {
                let error = rig
                    .runtime
                    .app_view_tool(&rig.run, &rig.agent, tool, arguments)
                    .await
                    .unwrap_err();
                assert_eq!(error.code, "view_unavailable");
                assert!(
                    !error.message.contains(SECRET),
                    "registration probe leaked a known secret through {tool}"
                );
            }
            let catalog = rig
                .runtime
                .app_view_tool(&rig.run, &rig.agent, CATALOG, json!({}))
                .await
                .unwrap();
            assert!(
                !catalog.to_string().contains(SECRET),
                "catalog probe leaked a known secret"
            );
            let context = rig.runtime.bound_view(&rig.agent, "actor-target").unwrap();
            let error = rig
                .runtime
                .run_in_view(&context, "roundtrip", json!({}))
                .await
                .unwrap_err();
            assert_eq!(error.code, "view_unavailable");
            assert!(
                !error.message.contains(SECRET),
                "action registration check leaked a known secret"
            );
            return;
        }
        if matches!(
            check,
            LiveBindingCheck::Panel
                | LiveBindingCheck::FailedRetirement
                | LiveBindingCheck::SuccessfulRetirement
        ) {
            rig.runtime
                .ensure_room_app_view(
                    "alice",
                    &rig.session,
                    APP,
                    crate::runtime::browser_controller_app_view::AppViewProjection::Human,
                )
                .await
                .unwrap();
            let views = rig.runtime.app_control().views();
            assert!(views.request_panel(
                &rig.session,
                "actor-target",
                crate::runtime::app_views::PanelRequest {
                    placement: Some(crate::session::AppPanelPlacement::Bottom),
                    size: Some(250),
                }
            ));
            let before = views.layouts(&rig.session);
            let state = rig
                .runtime
                .app_view_tool(&rig.run, &rig.agent, STATE, json!({"view": APP}))
                .await
                .unwrap();
            assert_eq!(state, json!({"ready": true}));
            assert_eq!(
                views.layouts(&rig.session),
                before,
                "agent state preserves the adopted page's panel request"
            );
        }
        if check == LiveBindingCheck::FailedRetirement {
            std::fs::write(rig.root.join("fail-idle-close"), "").unwrap();
            let context = rig.runtime.view_context(&rig.agent, APP).await.unwrap();
            assert_eq!(
                context.target, "actor-target",
                "the failed close reuses the adopted document"
            );
            let views = rig.runtime.app_control().views();
            views.suspend_for_cold_start(&rig.session);
            let restoring = views.cold_start_views(&rig.session);
            assert_eq!(
                restoring.len(),
                1,
                "failed retirement must preserve the adopted Tab's restore intent"
            );
            assert_eq!(restoring[0].installation, APP);
            return;
        }
        if check == LiveBindingCheck::SuccessfulRetirement {
            let views = rig.runtime.app_control().views();
            let old_tab = crate::session::EnvironmentTab {
                tab_id: "retired-person-tab".into(),
                url: "https://app.invalid/".into(),
                title: "Old App".into(),
                document_revision: 1,
                focused: true,
                app: None,
            };
            views.remember_logical_tab(&rig.session, "actor-target", old_tab.clone());
            std::fs::write(rig.root.join("successful-idle-close"), "").unwrap();
            let context = rig.runtime.view_context(&rig.agent, APP).await.unwrap();
            assert_eq!(context.target, "actor-target-2");
            assert!(
                views.binding_state(&rig.session, "actor-target").is_none(),
                "a successful close must unbind the old adopted Tab before the drain"
            );
            assert_eq!(views.bound_views(&rig.session).len(), 1);
            let opened = || std::fs::read_to_string(rig.root.join("app-opens")).unwrap();
            let before = opened();
            assert!(views.request_panel(
                &rig.session,
                &context.target,
                crate::runtime::app_views::PanelRequest {
                    placement: Some(crate::session::AppPanelPlacement::Bottom),
                    size: Some(275),
                }
            ));
            assert!(views.first_call(&rig.session, &context.target));
            let layouts = views.layouts(&rig.session);
            let state = rig
                .runtime
                .app_view_tool(&rig.run, &rig.agent, STATE, json!({"view": APP}))
                .await
                .unwrap();
            assert_eq!(state, json!({"ready": true}));
            assert_eq!(opened(), before, "the next agent call sends no second Open");
            assert_eq!(views.layouts(&rig.session), layouts);
            assert!(!views.first_call(&rig.session, &context.target));
            // The person adopts the replacement before the next drain. Browser
            // loss must restore this document, never the successfully closed one.
            rig.runtime
                .ensure_room_app_view(
                    "alice",
                    &rig.session,
                    APP,
                    crate::runtime::browser_controller_app_view::AppViewProjection::Human,
                )
                .await
                .unwrap();
            let new_tab = crate::session::EnvironmentTab {
                tab_id: "replacement-person-tab".into(),
                ..old_tab
            };
            views.remember_logical_tab(&rig.session, &context.target, new_tab.clone());
            views.suspend_for_cold_start(&rig.session);
            let restoring = views.cold_start_views(&rig.session);
            assert_eq!(restoring.len(), 1);
            assert_eq!(restoring[0].logical_tab, Some(new_tab));
            return;
        }
        let opens = || {
            std::fs::read_to_string(rig.root.join("app-opens"))
                .unwrap()
                .lines()
                .count()
        };
        let before = opens();
        let repeated = rig.runtime.view_catalog(&rig.agent).await;
        assert_eq!(opens(), before, "a live binding sends no second Open");
        assert_eq!(repeated["views"][0]["view"], view["view"]);
        assert_eq!(
            rig.runtime
                .app_control()
                .views()
                .bound_views(&rig.session)
                .len(),
            1
        );
        // The binding grants declared App work in this Room, never another Room's Tab.
        assert!(rig
            .runtime
            .app_control()
            .views()
            .bound_views("unrelated-room")
            .is_empty());
        rig.runtime
            .app_control()
            .views()
            .unbind(&rig.session, "actor-target");
        // The controller retired a Tab; its drain has not pruned Rust's old
        // binding yet. A stable handle must still resolve the actual target.
        rig.runtime.app_control().views().register_projection(
            &rig.session,
            "retired-target",
            AppViewBinding {
                owner: "alice".into(),
                installation: APP.into(),
                generation: 1,
                panel: Default::default(),
                logical_tab: None,
            },
            true,
        );
        let reopened = rig.runtime.view_context(&rig.agent, APP).await.unwrap();
        assert_eq!(reopened.target, "actor-target");
        assert_eq!(reopened.binding.installation, APP);
        assert_eq!(reopened.binding.generation, 1);
    });
    use futures_util::FutureExt;
    let outcome = result.catch_unwind().await;
    rig.runtime
        .app_control()
        .views()
        .forget_session(&rig.session);
    controllers.release(&rig.session).unwrap();
    rig.runtime
        .app_control()
        .lifecycle()
        .stop_blocking("alice", APP)
        .unwrap();
    if let Err(error) = outcome {
        std::panic::resume_unwind(error);
    }
}

#[tokio::test]
async fn remote_view_batches_keep_the_local_result_and_deadline_contract() {
    let mut rig = rig();
    rig.grant().await;
    let tool = KernelRuntimeState::remote_app_view_tools(&rig.agent)
        .into_iter()
        .find(|tool| tool.tool_name == BATCH)
        .unwrap();
    assert!(
        tool.timeout_sec.is_none(),
        "each action owns its deadline; a batch has no extra proxy deadline"
    );
    let result = RuntimeToolResult {
        ok: true,
        payload: json!({"completed": (0..8).map(|_| json!({"action":"read", "value":"x".repeat(256 * 1024 - 100)})).collect::<Vec<_>>(), "stopped":null}),
    };
    let expected = result.payload.clone();
    let checked = super::super::super::home_extension_execution_policy::enforce_home_app_runtime_result_limit(&tool, result.clone()).unwrap();
    assert_eq!(checked.payload, expected);
    let mut backend = tool;
    backend.name = APP.into();
    backend.tool_name = "app_installed_read".into();
    assert!(super::super::super::home_extension_execution_policy::enforce_home_app_runtime_result_limit(&backend, result).is_err(), "backend App tools keep their existing result limit");
}
