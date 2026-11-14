//! Apps v2 A2: the runtime MCP tools through which an agent reads and drives
//! its Room views and the owner's open App Tabs. The kernel decides what may
//! run from the signed declarations (`chariox_app_package::v2`); the page only answers.
//! Each call is bound to the acting agent's App grant, runs as that agent in
//! the page, waits its turn in the view's one queue, and reaches the model
//! only after known secret values are scrubbed.

use super::*;
use crate::runtime::{
    app_view_authority::{self, AppViewAuthority},
    app_views::AppViewBinding,
    browser_controller_app_view::{
        BrowserAppViewActor, BrowserAppViewError as ViewError, BrowserAppViewRequest,
        ViewAgentAnswer, ViewAgentCallKind,
    },
    kernel_browser_host::KernelBrowserHost,
};
use crate::session::{
    RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionChoiceStyle,
    RuntimeInteractionKind, RuntimeInteractionLevel,
};
use chariox_app_package::v2::{self as declared_actions, UiActionDeclaration, UiActionEffect};
use serde::Deserialize;
use serde_json::{json, Value};

pub(in crate::runtime::state) const CATALOG: &str = "chariox.app_view_catalog";
const INVOKE: &str = "chariox.app_view_invoke";
const STATE: &str = "chariox.app_view_state";
const CHANGES: &str = "chariox.app_view_changes";
const BATCH: &str = "chariox.app_view_batch";
const TOOLS: [&str; 5] = [CATALOG, INVOKE, STATE, CHANGES, BATCH];

/// An action is bounded by the page's own deadline: past it the view refuses
/// the next call until the handler settles (the bridge cannot interrupt it).
const ACTION_DEADLINE_MS: u64 = 20_000;
const CHANGES_MAX_WAIT_MS: u64 = 10_000;
const MAX_BATCH_STEPS: usize = 32;
const APPROVAL_TIMEOUT_SEC: u64 = 300;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InvokeArguments {
    view: String,
    action: String,
    #[serde(default = "empty_input")]
    input: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ViewArguments {
    view: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChangesArguments {
    view: String,
    #[serde(default)]
    since: u64,
    #[serde(default)]
    wait_ms: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchArguments {
    view: String,
    steps: Vec<BatchStep>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchStep {
    action: String,
    #[serde(default = "empty_input")]
    input: Value,
}

fn empty_input() -> Value {
    json!({})
}

fn failure(code: &str, message: impl Into<String>) -> ViewError {
    ViewError {
        code: code.into(),
        message: message.into(),
    }
}

fn action_error(error: declared_actions::ViewActionError) -> ViewError {
    failure(error.code, error.message)
}

/// Everything one call needs from a view, fixed when the call begins.
struct ViewContext {
    agent_id: String,
    /// The agent's Room, retained for approvals and Room-view secret scope.
    session: String,
    /// Queue identity. User-domain views are registered under their opaque view ID.
    queue_session: String,
    /// Room target or opaque user-view ID used by the in-kernel view queue.
    target: String,
    /// Room controller or an owner-registry-bound user App Tab.
    host: ViewHost,
    binding: AppViewBinding,
    authority: AppViewAuthority,
    declared: Vec<UiActionDeclaration>,
}

enum ViewHost {
    Room,
    /// The App's backend serves its actions and state: no page, Tab or browser.
    Backend,
    User {
        owner: String,
        view_id: String,
        browser_target: String,
        browser_generation: u64,
        retained: Box<(AppViewBinding, crate::local::UserAppView)>,
    },
}

pub(super) fn specs() -> Vec<RuntimeToolSpec> {
    let view = json!({"type": "string", "description": "A view id from chariox.app_view_catalog"});
    let input =
        json!({"type": "object", "description": "The action's input, as its inputSchema declares"});
    vec![
        RuntimeToolSpec {
            name: CATALOG.into(),
            description: "Open or reuse granted Apps in this Room or the owner's open App Tabs: each view's declared actions (name, description, effect, input schema) and current state. Run actions with chariox.app_view_invoke.".into(),
            input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
        },
        RuntimeToolSpec {
            name: INVOKE.into(),
            description: "Run one declared action in an open App view, as you, through the same function the App's buttons use. Read and write actions run at once; an approval action asks the user first (unless /sudo or an existing approval covers it). Read the changed state with chariox.app_view_state.".into(),
            input_schema: json!({"type": "object", "properties": {"view": view, "action": {"type": "string"}, "input": input}, "required": ["view", "action"], "additionalProperties": false}),
        },
        RuntimeToolSpec {
            name: STATE.into(),
            description: "Read an open App view's current state snapshot.".into(),
            input_schema: json!({"type": "object", "properties": {"view": view}, "required": ["view"], "additionalProperties": false}),
        },
        RuntimeToolSpec {
            name: CHANGES.into(),
            description: "Read the changes an open App view announced after `since` (0 for all buffered). With wait_ms, wait for the first one. `dropped: true` means some were lost: read the state again.".into(),
            input_schema: json!({"type": "object", "properties": {"view": view, "since": {"type": "integer", "minimum": 0},
                "wait_ms": {"type": "integer", "minimum": 0, "maximum": CHANGES_MAX_WAIT_MS}}, "required": ["view"], "additionalProperties": false}),
        },
        RuntimeToolSpec {
            name: BATCH.into(),
            description: format!("Run several declared actions in order in one view. Every step is checked before the first runs. It stops at the first error and at the first approval action, which it does not run (run that one with chariox.app_view_invoke); it reports the steps that completed. Nothing is rolled back. At most {MAX_BATCH_STEPS} steps."),
            input_schema: json!({"type": "object", "properties": {"view": view, "steps": {"type": "array", "maxItems": MAX_BATCH_STEPS,
                "items": {"type": "object", "properties": {"action": {"type": "string"}, "input": input}, "required": ["action"], "additionalProperties": false}}},
                "required": ["view", "steps"], "additionalProperties": false}),
        },
    ]
}

fn view_result(result: Result<Value, ViewError>) -> RuntimeToolResult {
    match result {
        Ok(payload) => RuntimeToolResult { ok: true, payload },
        Err(error) => RuntimeToolResult {
            ok: false,
            payload: json!({"code": error.code, "message": error.message}),
        },
    }
}

impl KernelRuntimeState {
    /// The view tools of an agent that holds at least one App grant.
    pub(in crate::runtime::state) fn app_view_tool_specs(
        &self,
        runs: &[crate::provider::RuntimeProviderRun],
    ) -> Vec<RuntimeToolSpec> {
        let [run] = runs else {
            return Vec::new();
        };
        let granted = self.app_agent_for_provider_run(run).is_ok_and(|agent| {
            agent
                .extension_grants()
                .iter()
                .any(|grant| grant.kind == ExtensionKind::App)
        });
        if granted {
            specs()
        } else {
            Vec::new()
        }
    }

    pub(in crate::runtime::state) async fn try_app_view_tool(
        &self,
        auth_token: &str,
        name: &str,
        arguments: Value,
    ) -> Option<Result<RuntimeToolResult, DaemonError>> {
        if !TOOLS.contains(&name) {
            return None;
        }
        let runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(auth_token);
        let [run] = runs.as_slice() else {
            return Some(Err(unavailable()));
        };
        if run
            .remote_extension_manifest()
            .home_proxy_tool(name)
            .is_some_and(|tool| tool.kind == ExtensionKind::App)
        {
            return Some(
                self.try_dispatch_remote_home_extension_runtime_tool_call(run, name, arguments)
                    .await
                    .and_then(|result| result.ok_or_else(unavailable)),
            );
        }
        let agent = match self.app_agent_for_provider_run(run) {
            Ok(agent) => agent,
            Err(error) => return Some(Err(error)),
        };
        Some(Ok(view_result(
            self.app_view_tool(run, &agent, name, arguments).await,
        )))
    }

    async fn app_view_tool(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        agent: &crate::agent::AgentInstance,
        name: &str,
        arguments: Value,
    ) -> Result<Value, ViewError> {
        self.app_view_tool_for_run_id(run.id(), agent, name, arguments)
            .await
    }

    async fn app_view_tool_for_run_id(
        &self,
        run_id: &str,
        agent: &crate::agent::AgentInstance,
        name: &str,
        arguments: Value,
    ) -> Result<Value, ViewError> {
        let malformed = |error: serde_json::Error| failure("invalid_arguments", error.to_string());
        match name {
            CATALOG => Ok(self.view_catalog(agent).await),
            INVOKE => {
                let call: InvokeArguments = serde_json::from_value(arguments).map_err(malformed)?;
                self.view_invoke(run_id, agent, &call.view, &call.action, call.input)
                    .await
            }
            STATE => {
                let call: ViewArguments = serde_json::from_value(arguments).map_err(malformed)?;
                let context = self.view_context(agent, &call.view).await?;
                let _turn = self.view_turn(&context).await?;
                self.view_call(&context, ViewAgentCallKind::State, None, None)
                    .await
            }
            CHANGES => {
                let call: ChangesArguments =
                    serde_json::from_value(arguments).map_err(malformed)?;
                // The change log runs no App code, so a wait for it holds
                // no turn: other actions on the view go on meanwhile.
                let context = self.view_context(agent, &call.view).await?;
                let wait = call.wait_ms.min(CHANGES_MAX_WAIT_MS);
                self.view_call(
                    &context,
                    ViewAgentCallKind::Changes,
                    None,
                    Some((call.since, wait)),
                )
                .await
            }
            _ => {
                let call: BatchArguments = serde_json::from_value(arguments).map_err(malformed)?;
                self.view_batch(agent, call).await
            }
        }
    }

    /// Open granted Apps without a viewer, then read their actions and state. A view that cannot
    /// be read is listed with the reason instead of failing the others.
    async fn view_catalog(&self, agent: &crate::agent::AgentInstance) -> Value {
        let mut views = Vec::new();
        for installation in agent.granted_extension_names(ExtensionKind::App) {
            let entry = match self.view_context(agent, &installation).await {
                Ok(context) => {
                    let state = match self.view_turn(&context).await {
                        Ok(_turn) => {
                            self.view_call(&context, ViewAgentCallKind::State, None, None)
                                .await
                        }
                        Err(error) => Err(error),
                    };
                    json!({"view": installation, "app": installation, "generation": context.binding.generation,
                        "actions": context.declared, "state": state.unwrap_or_else(|error| json!({"unavailable": error}))})
                }
                // Opening reads the view's actions and state; with no Room
                // browser to open it in, it stays a view the agent can open.
                Err(error) if error.code == "no_browser_controller" => {
                    json!({"view": installation, "app": installation, "openable": true})
                }
                Err(error) => {
                    let mut entry =
                        json!({"view": installation, "app": installation, "unavailable": error});
                    if let Ok(context) = self.bound_view(agent, &installation) {
                        entry["generation"] = json!(context.binding.generation);
                    }
                    entry
                }
            };
            views.push(entry);
        }
        json!({"views": views})
    }

    async fn view_invoke(
        &self,
        run_id: &str,
        agent: &crate::agent::AgentInstance,
        view: &str,
        name: &str,
        input: Value,
    ) -> Result<Value, ViewError> {
        let context = self.view_context(agent, view).await?;
        self.invoke_declared(run_id, agent, &context, name, input)
            .await
    }

    /// Declared, well-formed, approved when it must be, then run in the
    /// view's turn: every gate comes before the page sees the call.
    async fn invoke_declared(
        &self,
        run_id: &str,
        agent: &crate::agent::AgentInstance,
        context: &ViewContext,
        name: &str,
        input: Value,
    ) -> Result<Value, ViewError> {
        let action =
            declared_actions::find_declared(&context.declared, name).map_err(action_error)?;
        declared_actions::check_input(action, &input).map_err(action_error)?;
        self.check_view_claimed(agent, context)?;
        if action.effect == UiActionEffect::Approval {
            self.approve_view_action(run_id, agent, context, action, &input)
                .await?;
        }
        let _turn = self.view_turn(context).await?;
        self.run_in_view(context, name, input).await
    }

    /// An owner's App Tab is driven only by an agent that claimed it. Checked
    /// before any approval card, which would ask for an action that cannot run.
    fn check_view_claimed(
        &self,
        agent: &crate::agent::AgentInstance,
        context: &ViewContext,
    ) -> Result<(), ViewError> {
        let ViewHost::User { owner, view_id, .. } = &context.host else {
            return Ok(());
        };
        let actor = BrowserAppViewActor {
            kind: "agent".into(),
            id: agent.id().into(),
        };
        self.user_app_view_agent(owner, view_id, &actor).map(|_| ())
    }

    async fn view_batch(
        &self,
        agent: &crate::agent::AgentInstance,
        call: BatchArguments,
    ) -> Result<Value, ViewError> {
        if call.steps.len() > MAX_BATCH_STEPS {
            return Err(failure(
                "invalid_arguments",
                format!("a batch has at most {MAX_BATCH_STEPS} steps"),
            ));
        }
        let context = self.view_context(agent, &call.view).await?;
        let steps: Vec<(&str, &Value)> = call
            .steps
            .iter()
            .map(|step| (step.action.as_str(), &step.input))
            .collect();
        let plan = match declared_actions::plan_batch(&context.declared, &steps) {
            Ok(plan) => plan,
            Err((at, error)) => {
                return Ok(
                    json!({"completed": [], "stopped": {"at": at, "reason": "refused", "error": action_error(error)}}),
                );
            }
        };
        let mut completed = Vec::new();
        let _turn = self.view_turn(&context).await?;
        for (at, step) in call.steps[..plan.run].iter().enumerate() {
            match self
                .run_in_view(&context, &step.action, step.input.clone())
                .await
            {
                Ok(value) => completed.push(json!({"action": step.action, "value": value})),
                Err(error) => {
                    return Ok(json!({"completed": completed,
                        "stopped": {"at": at, "action": step.action, "reason": "error", "error": error}}));
                }
            }
        }
        let stopped = plan.approval.map(|action| {
            json!({"at": plan.run, "action": action.name, "reason": "approval",
            "approvalReason": action.approval_reason})
        });
        Ok(json!({"completed": completed, "stopped": stopped}))
    }

    /// The view, the agent's grant on its App, and the signed declarations of
    /// the generation the view runs.
    async fn view_context(
        &self,
        agent: &crate::agent::AgentInstance,
        target: &str,
    ) -> Result<ViewContext, ViewError> {
        if let Some(context) = self.backend_view(agent, target).await? {
            return Ok(context);
        }
        // Installation IDs are stable handles in the agent's own Room. After
        // idle retirement they open a fresh Tab from the current verified release.
        // Physical targets from the earlier catalog remain accepted while live.
        if let Some(context) = self.bound_user_view(agent, target).await? {
            return Ok(context);
        }
        let opened_target;
        let target = if agent.has_extension_grant(ExtensionKind::App, target) {
            // The grant outlives an uninstall, so the agent still has the view
            // tools and learns here that the App is gone.
            if self.app_not_installed(agent.owner_user_id(), target).await {
                return Err(failure(
                    "APP_NOT_INSTALLED",
                    "The App is not installed. Reinstall it to use it again.",
                ));
            }
            let authority = AppViewAuthority::new(
                self.owned.agent_store.clone(),
                Some(agent),
                target,
                None,
                Some(self.app_binding_epoch(agent.id(), target)),
            );
            authority.check()?;
            let mut retired = None;
            let existing = match self.bound_view(agent, target) {
                Ok(context) => {
                    // The drain can lag idle retirement. Registrations run no App
                    // code and prove liveness without resetting an adopted layout.
                    match self
                        .view_call(&context, ViewAgentCallKind::Registrations, None, None)
                        .await
                    {
                        Ok(_) => Some(context.target),
                        // Open waits for retirement; a failed close leaves the
                        // adopted Tab live, including its human restore intent.
                        Err(error) if error.code == "view_retired" => {
                            retired = Some(context);
                            None
                        }
                        Err(error)
                            if error.code == "view_unavailable"
                                && error.message.contains("unknown App tab") =>
                        {
                            self.app_control()
                                .views()
                                .unbind(&context.session, &context.target);
                            None
                        }
                        Err(error) => return Err(error),
                    }
                }
                Err(error) if error.code == "unknown_view" => None,
                Err(error) => return Err(error),
            };
            opened_target = if let Some(target) = existing {
                target
            } else {
                // Finish registration even if the grant is revoked during Open,
                // so the pump can retire the Tab. Reopen a retired binding once.
                let opened = self
                    .ensure_room_app_view(
                        agent.owner_user_id(),
                        agent.session_id(),
                        target,
                        crate::runtime::browser_controller_app_view::AppViewProjection::Agent {
                            agent: agent.id().into(),
                        },
                    )
                    .await
                    .map_err(|error| {
                        let code =
                            if crate::runtime::state::app_view_runtime::is_no_browser_controller(
                                &error,
                            ) {
                                "no_browser_controller"
                            } else {
                                "view_unavailable"
                            };
                        failure(code, error.to_string())
                    })?;
                if let Some(context) = retired {
                    // A successful close must not leave the old binding for
                    // the next call or browser-loss restore before the drain.
                    if opened.target_id != context.target {
                        self.app_control()
                            .views()
                            .unbind(&context.session, &context.target);
                    }
                }
                opened.target_id
            };
            authority.check()?;
            opened_target.as_str()
        } else {
            target
        };
        let mut context = self.bound_view(agent, target)?;
        let lease = tokio::select! {
            result = self.app_lease_on_demand(&context.binding.owner, &context.binding.installation) => {
                result.map_err(|_| failure("APP_UNAVAILABLE", "The App is not running"))?
            }
            () = context.authority.cancelled() => return Err(app_view_authority::not_granted()),
        };
        if lease.catalog().generation() != context.binding.generation {
            return Err(failure(
                "APP_VIEW_STALE",
                "The App was updated; reopen its view",
            ));
        }
        context.declared = lease.catalog().app_catalog().ui_actions().to_vec();
        Ok(context)
    }

    async fn app_not_installed(&self, owner: &str, installation: &str) -> bool {
        let store = self.owned.durable_state_store.clone();
        let (owner, installation) = (owner.to_owned(), installation.to_owned());
        tokio::task::spawn_blocking(move || {
            store
                .get_app_installation(&owner, &installation)
                .is_ok_and(|installation| installation.active.is_none())
        })
        .await
        .unwrap_or(false)
    }

    /// A user-domain App is reachable only through its owner's retained view
    /// record and an exact grant on that record's installation.
    async fn bound_user_view(
        &self,
        agent: &crate::agent::AgentInstance,
        requested: &str,
    ) -> Result<Option<ViewContext>, ViewError> {
        let canonical_owner = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        let views = self.app_control().user_views().clone();
        for owner in self.user_domain_owner_aliases(&canonical_owner) {
            for listed in views.list(&owner) {
                let Some(browser) = listed.browser.as_ref() else {
                    continue;
                };
                if listed.installation_id != requested
                    && listed.view_id != requested
                    && browser.tab_id != requested
                {
                    continue;
                }
                if !agent.has_extension_grant(ExtensionKind::App, &listed.installation_id) {
                    continue;
                }
                let Some(retained) = views.get(&owner, &listed.view_id) else {
                    continue;
                };
                let (binding, view) = &retained;
                let Some(browser) = view.browser.as_ref() else {
                    continue;
                };
                if binding.owner != owner
                    || binding.installation != view.installation_id
                    || view.generation.parse::<u64>().ok() != Some(binding.generation)
                {
                    return Err(failure(
                        "APP_VIEW_STALE",
                        "The App view binding changed; reopen it before using App actions",
                    ));
                }
                let Some(browser_target) = views.browser_target(&owner, &view.view_id) else {
                    return Err(failure(
                        "APP_VIEW_STALE",
                        "The App view is reloading; retry after opening it",
                    ));
                };
                let queue_session = view.view_id.clone();
                let queue_target = view.view_id.clone();
                let Some((queue_binding, reloading)) = self
                    .app_control()
                    .views()
                    .binding_state(&queue_session, &queue_target)
                else {
                    return Err(failure("unknown_view", "The App view closed; reopen it"));
                };
                if reloading || queue_binding != *binding {
                    return Err(failure(
                        "APP_VIEW_STALE",
                        "The App view is reloading; retry after opening it",
                    ));
                }
                let authority = AppViewAuthority::new(
                    self.owned.agent_store.clone(),
                    Some(agent),
                    &binding.installation,
                    None,
                    Some(self.app_binding_epoch(agent.id(), &binding.installation)),
                );
                authority.check()?;
                let (_, signed_actions) = self
                    .check_inspected_release(None, binding)
                    .await
                    .map_err(|_| {
                        failure(
                            "APP_VIEW_STALE",
                            "The signed App release changed; reopen the view",
                        )
                    })?;
                let lease = tokio::select! {
                    result = self.app_lease_on_demand(&binding.owner, &binding.installation) => {
                        result.map_err(|_| failure("APP_UNAVAILABLE", "The App is not running"))?
                    }
                    () = authority.cancelled() => return Err(app_view_authority::not_granted()),
                };
                let declared = lease.catalog().app_catalog().ui_actions().to_vec();
                if lease.catalog().generation() != binding.generation || declared != signed_actions
                {
                    return Err(failure(
                        "APP_VIEW_STALE",
                        "The App release changed; reopen its view",
                    ));
                }
                if views.get(&owner, &view.view_id).as_ref() != Some(&retained)
                    || views.browser_target(&owner, &view.view_id).as_deref()
                        != Some(browser_target.as_str())
                {
                    return Err(failure(
                        "APP_VIEW_STALE",
                        "The App view changed while it was being checked; retry",
                    ));
                }
                authority.check()?;
                let view_id = view.view_id.clone();
                let browser_generation = browser.generation;
                let context_binding = binding.clone();
                return Ok(Some(ViewContext {
                    agent_id: agent.id().to_owned(),
                    session: agent.session_id().to_owned(),
                    queue_session,
                    target: queue_target,
                    host: ViewHost::User {
                        owner,
                        view_id,
                        browser_target,
                        browser_generation,
                        retained: Box::new(retained),
                    },
                    binding: context_binding,
                    authority,
                    declared,
                }));
            }
        }
        Ok(None)
    }

    /// An open view of the agent's own Room, for an App the agent holds. The
    /// declarations come next, from the running App.
    fn bound_view(
        &self,
        agent: &crate::agent::AgentInstance,
        target: &str,
    ) -> Result<ViewContext, ViewError> {
        let session = agent.session_id();
        let views = self.app_control().views();
        let target = views
            .binding_state(session, target)
            .map(|_| target.to_owned())
            .or_else(|| {
                views
                    .bound_views(session)
                    .into_iter()
                    .find(|(_, binding)| {
                        binding.installation == target && binding.owner == agent.owner_user_id()
                    })
                    .map(|(target, _)| target)
            })
            .ok_or_else(|| {
                failure(
                    "unknown_view",
                    "no such view is open in this Room; list them with chariox.app_view_catalog",
                )
            })?;
        let (binding, reloading) = views
            .binding_state(session, &target)
            .ok_or_else(|| failure("unknown_view", "the view was closed"))?;
        if binding.owner != agent.owner_user_id() {
            return Err(failure(
                "unknown_view",
                "no such view is open for your App authority in this Room",
            ));
        }
        if reloading {
            return Err(failure(
                "APP_VIEW_STALE",
                "The view is reloading onto an updated App; try again shortly",
            ));
        }
        let authority = AppViewAuthority::new(
            self.owned.agent_store.clone(),
            Some(agent),
            &binding.installation,
            None,
            Some(self.app_binding_epoch(agent.id(), &binding.installation)),
        );
        authority.check()?;
        Ok(ViewContext {
            agent_id: agent.id().to_owned(),
            session: session.to_owned(),
            queue_session: session.to_owned(),
            target,
            host: ViewHost::Room,
            binding,
            authority,
            declared: Vec::new(),
        })
    }

    /// The view's one queue: two agent calls never run in its page at once.
    async fn view_turn(
        &self,
        context: &ViewContext,
    ) -> Result<Option<tokio::sync::OwnedMutexGuard<()>>, ViewError> {
        // A backend serves its own calls in order; there is no page to take turns on.
        if matches!(context.host, ViewHost::Backend) {
            context.authority.check()?;
            return Ok(None);
        }
        let queue = self
            .app_control()
            .views()
            .agent_queue(&context.queue_session, &context.target)
            .ok_or_else(|| failure("unknown_view", "the view was closed"))?;
        let turn = tokio::select! {
            turn = queue.lock_owned() => turn,
            () = context.authority.cancelled() => return Err(app_view_authority::not_granted()),
        };
        self.view_still_bound(context)?;
        Ok(Some(turn))
    }

    /// The view is still the one the call began on, at the same generation.
    fn view_still_bound(&self, context: &ViewContext) -> Result<(), ViewError> {
        context.authority.check()?;
        match &context.host {
            ViewHost::Backend => Ok(()),
            ViewHost::Room => match self
                .app_control()
                .views()
                .binding_state(&context.queue_session, &context.target)
            {
                Some((binding, false)) if binding == context.binding => Ok(()),
                Some(_) => Err(failure(
                    "APP_VIEW_STALE",
                    "The App was updated; reopen its view",
                )),
                None => Err(failure("unknown_view", "the view was closed")),
            },
            ViewHost::User {
                owner,
                view_id,
                browser_target,
                browser_generation,
                retained,
            } => {
                let user_views = self.app_control().user_views();
                if user_views.get(owner, view_id).as_ref() != Some(&**retained)
                    || user_views.browser_target(owner, view_id).as_deref()
                        != Some(browser_target.as_str())
                {
                    return Err(failure("unknown_view", "The App view closed; reopen it"));
                }
                let (binding, view) = &**retained;
                let Some(browser) = view.browser.as_ref() else {
                    return Err(failure("unknown_view", "The App view closed; reopen it"));
                };
                if binding != &context.binding
                    || view_id != &view.view_id
                    || browser.generation != *browser_generation
                    || view.generation.parse::<u64>().ok() != Some(binding.generation)
                {
                    return Err(failure(
                        "APP_VIEW_STALE",
                        "The App was updated; reopen its view",
                    ));
                }
                match self
                    .app_control()
                    .views()
                    .binding_state(&context.queue_session, &context.target)
                {
                    Some((queue_binding, false)) if queue_binding == context.binding => Ok(()),
                    Some(_) => Err(failure(
                        "APP_VIEW_STALE",
                        "The App was updated; reopen its view",
                    )),
                    None => Err(failure("unknown_view", "The App view closed; reopen it")),
                }
            }
        }
    }

    /// One action, with the page's registration checked against the signed
    /// declaration first. The caller holds the view's turn.
    async fn run_in_view(
        &self,
        context: &ViewContext,
        name: &str,
        input: Value,
    ) -> Result<Value, ViewError> {
        let registrations = self
            .view_call(context, ViewAgentCallKind::Registrations, None, None)
            .await?;
        declared_actions::check_registered(&context.declared, &registrations, name)
            .map_err(action_error)?;
        self.view_call(
            context,
            ViewAgentCallKind::Invoke,
            Some((name, input)),
            None,
        )
        .await
    }

    /// A page answer on its way to the model: known secret values are
    /// scrubbed from the value and from an App's error message alike.
    async fn view_call(
        &self,
        context: &ViewContext,
        kind: ViewAgentCallKind,
        invoke: Option<(&str, Value)>,
        changes: Option<(u64, u64)>,
    ) -> Result<Value, ViewError> {
        let answer = self.controller_call(context, kind, invoke, changes).await;
        self.ensure_context_current(context).await?;
        self.scrub_answer(context, answer)
    }

    fn scrub_answer(
        &self,
        context: &ViewContext,
        answer: Result<Value, ViewError>,
    ) -> Result<Value, ViewError> {
        let (protection, scope) = match &context.host {
            ViewHost::Room | ViewHost::Backend => (
                &self.owned.room_secret_observations,
                context.session.clone(),
            ),
            ViewHost::User { owner, .. } => (
                &self.owned.kernel_browser_secret_observations,
                KernelBrowserHost::profile_key(
                    &self.provider_account_authority_owner_user_id(owner),
                ),
            ),
        };
        match answer {
            Ok(value) => protection.scrub(&scope, value).map_err(|_| {
                failure(
                    "withheld",
                    "Secret protection is unavailable, so the App view's answer was withheld",
                )
            }),
            Err(error) => Err(ViewError {
                message: protection.scrub_text_or_withhold(&scope, &error.message),
                ..error
            }),
        }
    }

    async fn ensure_context_current(&self, context: &ViewContext) -> Result<(), ViewError> {
        self.view_still_bound(context)?;
        if matches!(&context.host, ViewHost::User { .. } | ViewHost::Backend) {
            let (_, signed_actions) = self
                .check_inspected_release(None, &context.binding)
                .await
                .map_err(|_| {
                    failure(
                        "APP_VIEW_STALE",
                        "The signed App release changed; reopen the view",
                    )
                })?;
            if signed_actions != context.declared {
                return Err(failure(
                    "APP_VIEW_STALE",
                    "The signed App actions changed; reopen the view",
                ));
            }
            self.view_still_bound(context)?;
        }
        Ok(())
    }

    async fn controller_call(
        &self,
        context: &ViewContext,
        kind: ViewAgentCallKind,
        invoke: Option<(&str, Value)>,
        changes: Option<(u64, u64)>,
    ) -> Result<Value, ViewError> {
        self.ensure_context_current(context).await?;
        if matches!(context.host, ViewHost::Backend) {
            return self.backend_view_call(context, kind, invoke).await;
        }
        let (name, input) = invoke.map_or((None, None), |(name, input)| {
            (Some(name.to_owned()), Some(input))
        });
        let request = BrowserAppViewRequest::Agent {
            target_id: match &context.host {
                ViewHost::Room => context.target.clone(),
                ViewHost::User { browser_target, .. } => browser_target.clone(),
                ViewHost::Backend => unreachable!("a backend view answers before the controller"),
            },
            agent: context.agent_id.clone(),
            call: kind,
            name,
            input,
            since: changes.map(|(since, _)| since),
            wait_ms: changes.map(|(_, wait)| wait),
            deadline_ms: ACTION_DEADLINE_MS,
        };
        let reply = tokio::select! {
            reply = async {
                match &context.host {
                    ViewHost::Backend => unreachable!("a backend view answers before the controller"),
                    ViewHost::Room => self.app_view_command::<Value>(&context.session, request).await,
                    ViewHost::User { owner, browser_generation, .. } => {
                        use crate::runtime::state::app_view_host::{
                            AppViewHost, KernelBrowserAppViewHost,
                        };
                        KernelBrowserAppViewHost {
                            state: self,
                            owner,
                            generation: Some(*browser_generation),
                        }
                        .command(request)
                        .await
                        .map(|value| value.unwrap_or(Value::Null))
                    }
                }
            } => reply.map_err(|error| failure("view_unavailable", error.to_string()))?,
            () = context.authority.cancelled() => return Err(app_view_authority::not_granted()),
        };
        match ViewAgentAnswer::from_controller(reply) {
            Some(ViewAgentAnswer::Value(value)) => Ok(value),
            Some(ViewAgentAnswer::Error(error)) => Err(error),
            None => Err(failure(
                "view_error",
                "The view answered in an unknown shape",
            )),
        }
    }

    /// An approval action asks through the existing kernel interaction, shown
    /// on every client of the Room; /sudo answers it like any provider permission.
    async fn approve_view_action(
        &self,
        run_id: &str,
        agent: &crate::agent::AgentInstance,
        context: &ViewContext,
        action: &UiActionDeclaration,
        input: &Value,
    ) -> Result<(), ViewError> {
        let current_turn = || {
            self.authorize_room_provider_epoch(Some(agent.id()), Some(run_id))
                .map_err(|_| {
                    failure(
                        "approval_expired",
                        "The approval belongs to a turn that has ended",
                    )
                })
        };
        current_turn()?;
        self.view_still_bound(context)?;
        if self.app_control().views().action_approved(
            &context.queue_session,
            &context.agent_id,
            &context.binding,
            &action.name,
        ) {
            return Ok(());
        }
        let shown = serde_json::to_string(input).unwrap_or_default();
        let shown: String = shown.chars().take(2000).collect();
        let interaction = RuntimeInteraction::new(
            format!("app-view-action-{:032x}", rand::random::<u128>()),
            agent.id(),
            RuntimeInteractionKind::Permission,
            RuntimeInteractionLevel::Warning,
            Some(format!("Approve {} in an App view?", action.name)),
            format!(
                "Agent {} asks to run `{}` in the {} view.\nWhy it needs approval: {}\nInput: {shown}",
                agent.agent_ref(),
                action.name,
                context.binding.installation,
                action.approval_reason.as_deref().unwrap_or("the App marked it as needing approval"),
            ),
            vec![
                RuntimeInteractionChoice::new("allow_once", "Allow once", "allow", Some(RuntimeInteractionChoiceStyle::Primary)),
                RuntimeInteractionChoice::new("allow_session", "Allow for session", "allow_session", Some(RuntimeInteractionChoiceStyle::Secondary)),
                RuntimeInteractionChoice::new("deny", "Deny", "deny", Some(RuntimeInteractionChoiceStyle::Danger)),
            ],
            None,
            Some(APPROVAL_TIMEOUT_SEC),
            Some("deny".into()),
        )
        .with_native_origin(self.owned.capture_native_interaction_origin(agent.session_id(), agent.id(), run_id));
        let answer = tokio::select! {
            answer = crate::runtime::native_interaction_bridge::request_provider_native_interaction(
                self, agent.session_id(), interaction, "app_view_action_approval") => {
                answer.map_err(|error| failure("approval_unavailable", error.to_string()))?
            }
            () = context.authority.cancelled() => return Err(app_view_authority::not_granted()),
        };
        if answer.status != "answered"
            || !matches!(
                answer.choice_id.as_deref(),
                Some("allow_once" | "allow_session")
            )
        {
            return Err(failure(
                "approval_denied",
                format!(
                    "The user did not approve {}. Do not retry it unless they ask.",
                    action.name
                ),
            ));
        }
        current_turn()?;
        self.view_still_bound(context)?;
        if answer.choice_id.as_deref() == Some("allow_session") {
            self.app_control().views().approve_action(
                &context.queue_session,
                &context.agent_id,
                &context.binding,
                &action.name,
                context.authority.clone(),
            );
        }
        Ok(())
    }
}

mod backend;
mod remote;

#[cfg(test)]
mod tests;
