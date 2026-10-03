use super::*;
use crate::agent::AgentServiceStore;
use crate::runtime::prompt_state::PromptStateOwner;
use crate::session::{PromptStatus, RuntimeSession};

/// Retains the submission's caller, without retaining an agent-store guard.
/// A turn-less call still belongs to its agent; a new turn cannot revive an
/// older call. This is cancellation only, never an authorization grant.
pub(super) struct AppCallerLifetime {
    agents: AgentServiceStore,
    prompts: PromptStateOwner,
    session: RuntimeSession,
    agent_id: String,
    turn_id: Option<String>,
}

impl AppCallerLifetime {
    pub(super) fn capture(
        state: &KernelRuntimeState,
        agent: &crate::agent::AgentInstance,
    ) -> Result<Self, DaemonError> {
        Ok(Self::new(
            state.owned.agent_store.clone(),
            state.owned.prompt_state_owner.clone(),
            state.owned.session_store.get_session(agent.session_id())?,
            agent.id().to_owned(),
        ))
    }

    fn new(
        agents: AgentServiceStore,
        prompts: PromptStateOwner,
        session: RuntimeSession,
        agent_id: String,
    ) -> Self {
        let turn_id = prompts
            .active_prompt_for_agent_snapshot(&session, &agent_id)
            .map(|prompt| prompt.id().to_owned());
        Self {
            agents,
            prompts,
            session,
            agent_id,
            turn_id,
        }
    }

    pub(super) fn cancelled(&self) -> bool {
        match self
            .agents
            .agent_in_session_if_available(&self.agent_id, self.session.id())
        {
            Some(false) => return true,
            // Enqueue may hold this same store's binding guard. Defer the
            // lifetime poll, without extending the operation's fixed deadline.
            None => return false,
            Some(true) => {}
        }
        self.turn_id.as_ref().is_some_and(|expected| {
            !self
                .prompts
                .active_prompt_for_agent_snapshot(&self.session, &self.agent_id)
                .is_some_and(|prompt| {
                    prompt.id() == expected && prompt.status() == PromptStatus::Running
                })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentService, CreateAgentRequest};
    use crate::config::DaemonConfig;
    use crate::runtime::app_operation_budget::{AppOperationBudget, AppOperationStopped};
    use crate::session::{CreateSessionRequest, PromptQueueItem, SessionService};

    struct Fixture {
        agents: AgentServiceStore,
        prompts: PromptStateOwner,
        sessions: SessionService,
        session: RuntimeSession,
        agent_id: String,
    }

    impl Fixture {
        fn new() -> Self {
            let mut sessions = SessionService::new(&DaemonConfig::for_tests());
            let session = sessions
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .unwrap();
            let agents = AgentServiceStore::new(AgentService::new());
            let agent = agents
                .create_agent(
                    CreateAgentRequest::new(session.id(), "dev-stub").with_worktree("worktree"),
                    &mut sessions,
                )
                .unwrap();
            Self {
                agents,
                prompts: PromptStateOwner::default(),
                sessions,
                session,
                agent_id: agent.id().to_owned(),
            }
        }

        fn submit(&self, id: &str) {
            self.prompts
                .submit_prepared_prompt(
                    &self.session,
                    PromptQueueItem::new(
                        id,
                        "attachment",
                        &self.agent_id,
                        "bounded App caller fixture",
                        PromptStatus::Queued,
                    ),
                    false,
                )
                .unwrap();
        }

        fn lifetime(&self) -> AppCallerLifetime {
            AppCallerLifetime::new(
                self.agents.clone(),
                self.prompts.clone(),
                self.session.clone(),
                self.agent_id.clone(),
            )
        }
    }

    #[test]
    fn app_caller_lifetime_cancelling_turn_stops_retained_budget() {
        let f = Fixture::new();
        f.submit("turn-one");
        let lifetime = f.lifetime();
        let budget = AppOperationBudget::from_supervisor(move || lifetime.cancelled());
        assert_eq!(budget.check(), Ok(()));
        f.prompts
            .begin_cancelling_active_prompt(&f.session, &f.agent_id)
            .unwrap();
        assert_eq!(budget.check(), Err(AppOperationStopped::Cancelled));
    }

    #[test]
    fn app_caller_lifetime_destroyed_agent_stops_turnless_call() {
        let mut f = Fixture::new();
        let lifetime = f.lifetime();
        assert!(!lifetime.cancelled());
        f.agents
            .destroy_agent(&f.agent_id, &mut f.sessions)
            .unwrap();
        assert!(lifetime.cancelled());
    }

    #[test]
    fn app_caller_lifetime_new_turn_does_not_revive_old_call() {
        let f = Fixture::new();
        f.submit("turn-one");
        let old = f.lifetime();
        f.prompts
            .cancel_active_prompt_only(&f.session, &f.agent_id)
            .unwrap();
        f.submit("turn-two");
        assert!(old.cancelled());
        assert!(!f.lifetime().cancelled());
    }

    #[test]
    fn app_caller_lifetime_other_agent_cancellation_is_independent() {
        let f = Fixture::new();
        f.submit("turn-one");
        let lifetime = f.lifetime();
        f.prompts
            .submit_prepared_prompt(
                &f.session,
                PromptQueueItem::new(
                    "other-turn",
                    "attachment",
                    "other-agent",
                    "neighbour",
                    PromptStatus::Queued,
                ),
                false,
            )
            .unwrap();
        f.prompts
            .begin_cancelling_active_prompt(&f.session, "other-agent")
            .unwrap();
        assert!(!lifetime.cancelled());
    }

    #[test]
    fn app_caller_lifetime_binding_guard_does_not_deadlock_poll() {
        let f = Fixture::new();
        f.submit("turn-one");
        let lifetime = f.lifetime();
        let guard = f.agents.read();
        assert!(!lifetime.cancelled());
        drop(guard);
        f.prompts
            .begin_cancelling_active_prompt(&f.session, &f.agent_id)
            .unwrap();
        assert!(lifetime.cancelled());
    }
}
