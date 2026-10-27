//! Archive and prompt admission commit under one owner lock, including stale prepared requests.
use super::*;

impl PromptStateOwner {
    pub(crate) fn agent_is_archived(&self, session: &RuntimeSession, agent_id: &str) -> bool {
        let owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        owner
            .archived_agents
            .contains(&PromptStateKey::new(session.id(), agent_id))
            || session
                .agents()
                .iter()
                .any(|agent| agent.id() == agent_id && agent.archived())
    }

    /// The callback must not reenter this owner or await provider I/O.
    pub(crate) fn with_unarchived_agent<T>(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        commit: impl FnOnce() -> Result<T, DaemonError>,
    ) -> Result<T, DaemonError> {
        let owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if owner
            .archived_agents
            .contains(&PromptStateKey::new(session.id(), agent_id))
            || session
                .agents()
                .iter()
                .any(|agent| agent.id() == agent_id && agent.archived())
        {
            return Err(DaemonError::LocalTransport {
                operation: "launch provider",
                message: "Unarchive this agent before starting its provider.".into(),
            });
        }
        commit()
    }

    /// The callback must not reenter this owner or await provider I/O.
    pub(crate) fn change_agent_archive<T>(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
        archived: bool,
        commit: impl FnOnce() -> Result<T, DaemonError>,
    ) -> Result<T, DaemonError> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = PromptStateKey::new(session.id(), agent_id);
        if archived {
            let prompts = owner
                .states
                .get(&key)
                .cloned()
                .unwrap_or_else(|| OwnedAgentPromptState::from_session(session, agent_id));
            if prompts
                .active_prompt
                .iter()
                .chain(prompts.queued_prompts.iter())
                .any(|prompt| prompt.workflow_run_id().is_some())
            {
                return Err(DaemonError::LocalTransport {
                    operation: "archive agent",
                    message: "Pause or rebind this agent's active or queued workflow before archiving it; other agents will keep running.".into(),
                });
            }
            if owner.profile_transitions.contains_key(&key) || owner.queue_promotions.contains(&key)
            {
                return Err(DaemonError::LocalTransport {
                    operation: "archive agent",
                    message: "Agent launch or profile change is committing; retry archive once it completes."
                        .into(),
                });
            }
        }
        let result = commit()?;
        if archived {
            owner.archived_agents.insert(key);
        } else {
            owner.archived_agents.remove(&key);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Barrier};

    fn session() -> RuntimeSession {
        RuntimeSession::new("room", None, "workspace", "worktree", "machine", "kernel")
    }
    fn prompt() -> PromptQueueItem {
        PromptQueueItem::new("prompt", "client", "agent", "work", PromptStatus::Queued)
    }

    #[test]
    fn archive_agent_blocks_concurrent_stale_prepared_admission() {
        let owner = PromptStateOwner::default();
        let session = session();
        let entered = Barrier::new(2);
        let (release, wait) = mpsc::channel();
        std::thread::scope(|scope| {
            let shared_owner = &owner;
            let shared_session = &session;
            let shared_entered = &entered;
            let archive = scope.spawn(move || {
                shared_owner.change_agent_archive(shared_session, "agent", true, || {
                    shared_entered.wait();
                    wait.recv().unwrap();
                    Ok(())
                })
            });
            entered.wait();
            let (started, ready) = mpsc::channel();
            let submit = scope.spawn(move || {
                started.send(()).unwrap();
                shared_owner.submit_prepared_prompt(shared_session, prompt(), false)
            });
            ready.recv().unwrap();
            release.send(()).unwrap();
            archive.join().unwrap().unwrap();
            assert!(submit.join().unwrap().is_err());
        });
        let (active, queued) = owner.state_parts(&session, "agent");
        assert!(active.is_none() && queued.is_empty());
        owner
            .change_agent_archive(&session, "agent", false, || Ok(()))
            .unwrap();
        assert!(owner
            .submit_prepared_prompt(&session, prompt(), false)
            .is_ok());
    }

    #[test]
    fn archive_agent_refuses_reserved_work_and_fences_admitted_work() {
        let owner = PromptStateOwner::default();
        let session = session();
        let profile = owner
            .claim_idle_agent_profile_transition(&session, "agent")
            .unwrap();
        assert!(owner
            .change_agent_archive(&session, "agent", true, || -> Result<(), DaemonError> {
                panic!("must not mutate")
            })
            .is_err());
        drop(profile);
        owner
            .submit_prepared_prompt(&session, prompt(), false)
            .unwrap();
        owner
            .change_agent_archive(&session, "agent", true, || Ok(()))
            .unwrap();
        assert!(owner
            .submit_prepared_prompt(&session, prompt(), false)
            .is_err());
        assert!(
            owner.state_parts(&session, "agent").0.is_some(),
            "the runtime must settle existing work after the fence commits"
        );
    }

    #[test]
    fn archive_agent_refuses_active_or_queued_workflow_without_mutation() {
        for queued in [false, true] {
            let owner = PromptStateOwner::default();
            let session = session();
            if queued {
                owner
                    .submit_prepared_prompt(&session, prompt(), false)
                    .unwrap();
            }
            owner
                .submit_prepared_prompt(
                    &session,
                    PromptQueueItem::new(
                        "workflow-prompt",
                        "client",
                        "agent",
                        "work",
                        PromptStatus::Queued,
                    )
                    .with_workflow_context("run", "node"),
                    false,
                )
                .unwrap();
            assert!(owner
                .change_agent_archive(&session, "agent", true, || -> Result<(), DaemonError> {
                    panic!("workflow participation must be checked before mutation")
                })
                .is_err());
            assert!(owner
                .submit_prepared_prompt(
                    &session,
                    PromptQueueItem::new("after", "client", "agent", "work", PromptStatus::Queued),
                    false
                )
                .is_ok());
        }
    }

    #[test]
    fn archive_agent_restores_idle_archive_without_prompt_state() {
        let owner = PromptStateOwner::default();
        let mut session = session();
        let mut agent = crate::agent::AgentInstance::new(
            "agent",
            "a1",
            "room",
            None,
            "codex",
            None,
            None,
            None,
            crate::agent::GridPosition::new(0, 0, 1, 1),
        );
        agent.set_archived(true);
        session.set_agents(vec![agent]);
        assert!(session.prompt_states().is_empty());
        owner.restore_session_state(&session);
        assert!(owner
            .submit_prepared_prompt(&session, prompt(), false)
            .is_err());
        owner.remove_agent("room", "agent");
        assert!(owner
            .submit_prepared_prompt(&session, prompt(), false)
            .is_ok());
    }
}
