//! AV3: an App whose backend serves its own actions and state. There is no page,
//! Tab or browser: the kernel reads the registration and the state from the
//! App's reserved tools and runs a declared action as the backend tool of the
//! same name. The signed declarations still decide what may run.

use super::*;
use crate::runtime::state::app_view_runtime::ViewCaller;

/// The backend lists the actions it registers, in the shape of the signed declarations.
const ACTIONS_TOOL: &str = "view_actions";
/// The backend answers with the view's current state.
const STATE_TOOL: &str = "view_state";

fn serves_view(catalog: &chariox_app_runtime::app_catalog::AppCatalog) -> bool {
    [ACTIONS_TOOL, STATE_TOOL]
        .iter()
        .all(|reserved| catalog.tools().any(|tool| tool.local_name == *reserved))
}

impl KernelRuntimeState {
    /// The backend-served view of an App the agent holds, named by its installation id. `None`
    /// when the App is not installed or its backend serves no view: the page path answers then.
    pub(super) async fn backend_view(
        &self,
        agent: &crate::agent::AgentInstance,
        installation: &str,
    ) -> Result<Option<ViewContext>, ViewError> {
        if !agent.has_extension_grant(ExtensionKind::App, installation) {
            return Ok(None);
        }
        let store = self.owned.durable_state_store.clone();
        let (owner, app) = (agent.owner_user_id().to_owned(), installation.to_owned());
        let Ok(Ok(catalog)) =
            tokio::task::spawn_blocking(move || store.active_app_event_catalog(&owner, &app)).await
        else {
            return Ok(None);
        };
        if !serves_view(catalog.app_catalog()) {
            return Ok(None);
        }
        let authority = AppViewAuthority::new(
            self.owned.agent_store.clone(),
            Some(agent),
            installation,
            None,
            Some(self.app_binding_epoch(agent.id(), installation)),
        );
        authority.check()?;
        Ok(Some(ViewContext {
            agent_id: agent.id().to_owned(),
            session: agent.session_id().to_owned(),
            queue_session: agent.session_id().to_owned(),
            target: installation.to_owned(),
            host: ViewHost::Backend,
            binding: AppViewBinding {
                owner: agent.owner_user_id().to_owned(),
                installation: installation.to_owned(),
                generation: catalog.generation(),
                panel: Default::default(),
                logical_tab: None,
            },
            authority,
            declared: catalog.app_catalog().ui_actions().to_vec(),
        }))
    }

    /// What the page would have answered, answered by the backend.
    pub(super) async fn backend_view_call(
        &self,
        context: &ViewContext,
        kind: ViewAgentCallKind,
        invoke: Option<(&str, Value)>,
    ) -> Result<Value, ViewError> {
        let (tool, input) = match (kind, invoke) {
            (ViewAgentCallKind::Registrations, _) => (ACTIONS_TOOL.to_owned(), json!({})),
            (ViewAgentCallKind::State, _) => (STATE_TOOL.to_owned(), json!({})),
            (ViewAgentCallKind::Invoke, Some((action, input))) => (action.to_owned(), input),
            (ViewAgentCallKind::Invoke, None) => {
                return Err(failure(
                    "invalid_arguments",
                    "an action call names its action",
                ));
            }
            (ViewAgentCallKind::Changes, _) => {
                return Err(failure(
                    "changes_unavailable",
                    "This App's backend keeps no change feed yet; read its state again with chariox.app_view_state",
                ));
            }
        };
        let agent = self
            .owned
            .agent_store
            .get_agent(&context.agent_id)
            .map_err(|_| app_view_authority::not_granted())?;
        let mut call = self.app_control().views().detached_call();
        tokio::select! {
            result = self.invoke_app_view_tool(ViewCaller::Agent(&agent), &context.binding, &tool, input, &mut call) => result,
            () = context.authority.cancelled() => Err(app_view_authority::not_granted()),
        }
    }
}
