//! Archive and prompt admission commit under one owner lock, including stale prepared requests.
use super::*;

impl PromptStateOwner {
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
            let state = owner.ensure_agent_state(session, agent_id);
            let busy = state.active_prompt.is_some() || !state.queued_prompts.is_empty();
            if busy
                || owner.profile_transitions.contains_key(&key)
                || owner.queue_promotions.contains(&key)
            {
                return Err(DaemonError::LocalTransport {
                    operation: "archive agent",
                    message: "Stop this agent and clear its queued prompts before archiving it."
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
    fn archive_agent_refuses_admitted_or_reserved_work_without_mutating() {
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
        assert!(owner
            .change_agent_archive(&session, "agent", true, || -> Result<(), DaemonError> {
                panic!("must not mutate")
            })
            .is_err());
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
