//! Apps v2 A2: the runtime MCP tools through which an agent reads and drives
//! the open App views of its Room. The kernel decides what may run from the
//! signed declarations (`chariox_app_package::v2`); the page only answers.
//! Each call is bound to the acting agent's App grant, runs as that agent in
//! the page, waits its turn in the view's one queue, and reaches the model
//! only after known secret values are scrubbed.

use super::*;
use crate::runtime::{
    app_view_authority::{self, AppViewAuthority},
    app_views::AppViewBinding,
    browser_controller_app_view::{
        BrowserAppViewError as ViewError, BrowserAppViewRequest, ViewAgentAnswer, ViewAgentCallKind,
    },
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
    session: String,
    target: String,
    binding: AppViewBinding,
    authority: AppViewAuthority,
    declared: Vec<UiActionDeclaration>,
}

pub(super) fn specs() -> Vec<RuntimeToolSpec> {
    let view = json!({"type": "string", "description": "A view id from chariox.app_view_catalog"});
    let input =
        json!({"type": "object", "description": "The action's input, as its inputSchema declares"});
    vec![
        RuntimeToolSpec {
            name: CATALOG.into(),
            description: "List the App views open in this Room: each view's declared actions (name, description, effect, input schema) and its current state. Run actions with chariox.app_view_invoke.".into(),
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
            self.app_view_tool(run.id(), &agent, name, arguments).await,
        )))
    }

    async fn app_view_tool(
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

    /// Open views with their declared actions and state. A view that cannot
    /// be read is listed with the reason instead of failing the others.
    async fn view_catalog(&self, agent: &crate::agent::AgentInstance) -> Value {
        let mut views = Vec::new();
        for (target, binding) in self.app_control().views().bound_views(agent.session_id()) {
            if !agent.has_extension_grant(ExtensionKind::App, &binding.installation) {
                continue;
            }
            let entry = match self.view_context(agent, &target).await {
                Ok(context) => {
                    let state = match self.view_turn(&context).await {
                        Ok(_turn) => {
                            self.view_call(&context, ViewAgentCallKind::State, None, None)
                                .await
                        }
                        Err(error) => Err(error),
                    };
                    json!({"view": target, "app": binding.installation, "generation": binding.generation,
                        "actions": context.declared, "state": state.unwrap_or_else(|error| json!({"unavailable": error}))})
                }
                Err(error) => {
                    json!({"view": target, "app": binding.installation, "generation": binding.generation, "unavailable": error})
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
        if action.effect == UiActionEffect::Approval {
            self.approve_view_action(run_id, agent, context, action, &input)
                .await?;
        }
        let _turn = self.view_turn(context).await?;
        self.run_in_view(context, name, input).await
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

    /// An open view of the agent's own Room, for an App the agent holds. The
    /// declarations come next, from the running App.
    fn bound_view(
        &self,
        agent: &crate::agent::AgentInstance,
        target: &str,
    ) -> Result<ViewContext, ViewError> {
        let session = agent.session_id();
        let Some((binding, reloading)) = self.app_control().views().binding_state(session, target)
        else {
            return Err(failure(
                "unknown_view",
                "no such view is open in this Room; list them with chariox.app_view_catalog",
            ));
        };
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
            target: target.to_owned(),
            binding,
            authority,
            declared: Vec::new(),
        })
    }

    /// The view's one queue: two agent calls never run in its page at once.
    async fn view_turn(
        &self,
        context: &ViewContext,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, ViewError> {
        let queue = self
            .app_control()
            .views()
            .agent_queue(&context.session, &context.target)
            .ok_or_else(|| failure("unknown_view", "the view was closed"))?;
        let turn = tokio::select! {
            turn = queue.lock_owned() => turn,
            () = context.authority.cancelled() => return Err(app_view_authority::not_granted()),
        };
        self.view_still_bound(context)?;
        Ok(turn)
    }

    /// The view is still the one the call began on, at the same generation.
    fn view_still_bound(&self, context: &ViewContext) -> Result<(), ViewError> {
        context.authority.check()?;
        match self
            .app_control()
            .views()
            .binding_state(&context.session, &context.target)
        {
            Some((binding, false)) if binding.generation == context.binding.generation => Ok(()),
            Some(_) => Err(failure(
                "APP_VIEW_STALE",
                "The App was updated; reopen its view",
            )),
            None => Err(failure("unknown_view", "the view was closed")),
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
            .controller_call(context, ViewAgentCallKind::Registrations, None, None)
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
        self.scrub_answer(&context.session, answer)
    }

    fn scrub_answer(
        &self,
        session: &str,
        answer: Result<Value, ViewError>,
    ) -> Result<Value, ViewError> {
        let protection = &self.owned.room_secret_observations;
        match answer {
            Ok(value) => protection
                .scrub(session, value)
                .map_err(|_| failure("withheld", "The Room's secret protection is unavailable, so the view's answer was withheld")),
            Err(error) => Err(ViewError {
                message: protection.scrub_text_or_withhold(session, &error.message),
                ..error
            }),
        }
    }

    async fn controller_call(
        &self,
        context: &ViewContext,
        kind: ViewAgentCallKind,
        invoke: Option<(&str, Value)>,
        changes: Option<(u64, u64)>,
    ) -> Result<Value, ViewError> {
        self.view_still_bound(context)?;
        let (name, input) = invoke.map_or((None, None), |(name, input)| {
            (Some(name.to_owned()), Some(input))
        });
        let request = BrowserAppViewRequest::Agent {
            target_id: context.target.clone(),
            agent: context.agent_id.clone(),
            call: kind,
            name,
            input,
            since: changes.map(|(since, _)| since),
            wait_ms: changes.map(|(_, wait)| wait),
            deadline_ms: ACTION_DEADLINE_MS,
        };
        let reply = tokio::select! {
            reply = self.app_view_command::<Value>(&context.session, request) => reply
                .map_err(|error| failure("view_unavailable", error.to_string()))?,
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
            &context.session,
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
                &context.session,
                &context.agent_id,
                &context.binding,
                &action.name,
                context.authority.clone(),
            );
        }
        Ok(())
    }
}

mod remote;

#[cfg(test)]
mod tests;
