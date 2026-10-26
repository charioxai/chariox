//! MP-08 / MP-11, A01: durable registration; lifecycle/settlement follows in PR2.
use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn register_room_dispatch_obligation(
        &self,
        actor_id: &str,
        kind: &str,
        resource: Option<&str>,
    ) -> Result<Option<String>, DaemonError> {
        if !self.config_projection.snapshot().room_agent_tools {
            return Ok(None);
        }
        let actor = self.agent_store.get_agent(actor_id)?;
        let run = self
            .provider_store
            .get_run_for_agent(actor.session_id(), actor.id());
        let session = self.session_store.get_session(actor.session_id())?;
        crate::runtime::room_dispatch_registration::register(
            &self.durable_state_store,
            &actor,
            run.as_ref().map(|r| r.id()),
            session.active_prompt_for_agent(actor.id()).map(|p| p.id()),
            kind,
            resource,
        )
        .map(Some)
    }
}

impl KernelRuntimeState {
    pub(crate) fn register_room_dispatch_obligation(
        &self,
        actor: &crate::agent::AgentInstance,
        kind: &str,
        resource: Option<&str>,
    ) -> Result<Option<String>, DaemonError> {
        self.owned
            .register_room_dispatch_obligation(actor.id(), kind, resource)
    }
    pub(crate) fn record_room_dispatch_receipt(
        &self,
        id: Option<&str>,
        accepted: bool,
        resource: Option<&str>,
    ) -> Result<(), DaemonError> {
        crate::runtime::room_dispatch_registration::receipt(
            &self.owned.durable_state_store,
            id,
            accepted,
            resource,
        )
    }

    /// Finish the receipt transaction before provider startup writes can be batched
    /// with it. Already-admitted dispatches still start if this receipt fails.
    pub(crate) fn finish_room_dispatch(
        &self,
        id: Option<&str>,
        resource: Option<&str>,
        dispatch: impl FnOnce(),
    ) -> Result<(), DaemonError> {
        let receipt = self.record_room_dispatch_receipt(id, true, resource);
        dispatch();
        receipt
    }
}
