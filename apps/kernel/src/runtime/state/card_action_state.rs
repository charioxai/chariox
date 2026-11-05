//! Person cards: a card that carries one pre-bound action. The kernel describes the action,
//! asks the owner for the passkey and hands the decision back; the router runs the action.
use super::*;

pub(crate) const APPROVE_CHOICE_ID: &str = "approve";

/// An open action card. Dropping it before the person decides withdraws the card, so an
/// agent that stopped waiting never leaves a card whose approval would run nothing.
pub(crate) struct ActionCard {
    pub(crate) interaction_id: String,
    pub(crate) owner_user_id: String,
    session_id: String,
    owned: KernelRuntimeOwnedState,
    lifetime: super::PendingInteraction,
    decided: Option<tokio::sync::oneshot::Receiver<PendingInteractionResolution>>,
}

impl ActionCard {
    pub(in crate::runtime) fn is_live(&self) -> bool {
        self.owned.agent_interaction_is_live(&self.lifetime)
    }

    pub(in crate::runtime) async fn decision(
        &mut self,
    ) -> Result<PendingInteractionResolution, DaemonError> {
        let receiver = self
            .decided
            .as_mut()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "runtime_tool_request_popup",
                message: "the card was already decided".into(),
            })?;
        let resolution = receiver
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "runtime_tool_request_popup",
                message: format!("action card dropped before a decision: {error}"),
            })?;
        self.decided = None;
        Ok(resolution)
    }
}

impl Drop for ActionCard {
    fn drop(&mut self) {
        if self.decided.is_some() {
            let _ = self
                .owned
                .timeout_runtime_interaction(&self.session_id, &self.interaction_id);
        }
    }
}

impl KernelRuntimeState {
    pub(crate) async fn raise_action_card(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        request: &crate::local::LocalDaemonRequest,
        timeout_sec: Option<u64>,
    ) -> Result<ActionCard, DaemonError> {
        let refused = |message: &str| DaemonError::LocalTransport {
            operation: "runtime_tool_request_popup",
            message: message.into(),
        };
        if timeout_sec == Some(0) {
            return Err(refused("timeout_sec must be at least 1"));
        }
        let agent_id = run
            .agent_instance_id()
            .ok_or_else(|| refused("provider run is not bound to an agent"))?;
        let current = self
            .owned
            .provider_store
            .get_run_for_agent(run.session_id(), agent_id)
            .ok_or_else(|| refused("asking provider run is unavailable"))?;
        if current.id() != run.id() {
            return Err(refused("stale provider tool authority"));
        }
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let interaction_id = format!("card-action-{:032x}", rand::random::<u128>());
        let interaction = crate::session::RuntimeInteraction::for_kernel_operation(
            &interaction_id,
            format!("card-action:{interaction_id}"),
            "Approve one action",
            crate::runtime::card_action::card_action_summary(
                agent.alias().unwrap_or_else(|| agent.agent_ref()),
                request,
            ),
            vec![
                crate::session::RuntimeInteractionChoice::new(
                    APPROVE_CHOICE_ID,
                    "Approve",
                    APPROVE_CHOICE_ID,
                    Some(crate::session::RuntimeInteractionChoiceStyle::Primary),
                )
                .requiring_passkey(),
                crate::session::RuntimeInteractionChoice::new(
                    super::tool_dispatch::CANCEL_CHOICE_ID,
                    "Cancel",
                    super::tool_dispatch::CANCEL_CHOICE_ID,
                    Some(crate::session::RuntimeInteractionChoiceStyle::Secondary),
                ),
            ],
        )
        .with_related_agent_id(agent_id.to_owned())
        .with_native_origin(Some(
            self.capture_native_interaction_origin(run.session_id(), agent_id, run.id())
                .unwrap_or_else(
                    || crate::session::NativeInteractionOrigin::ProviderStartup {
                        provider_run_id: run.id().into(),
                    },
                ),
        ))
        .with_timeout_sec(timeout_sec.unwrap_or(300));
        let receiver = self
            .create_kernel_operation_interaction(
                run.session_id(),
                agent.owner_user_id(),
                interaction,
            )
            .await?;
        let lifetime = self
            .owned
            .pending_interactions
            .write()
            .get(&interaction_id)
            .cloned()
            .ok_or_else(|| refused("asking prompt ended before the action card was registered"))?;
        if let Some(seconds) = timeout_sec.or(Some(300)) {
            let state = self.clone();
            let (session, id) = (run.session_id().to_owned(), interaction_id.clone());
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(seconds)).await;
                let _ = state.timeout_runtime_interaction(&session, &id).await;
            });
        }
        Ok(ActionCard {
            interaction_id,
            owner_user_id: agent.owner_user_id().to_owned(),
            session_id: run.session_id().to_owned(),
            owned: self.owned.clone(),
            lifetime,
            decided: Some(receiver),
        })
    }
}
