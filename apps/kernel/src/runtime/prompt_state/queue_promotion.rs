//! Deduplicate asynchronous preparation of an idle agent's queued prompt.
use super::*;

pub(crate) struct PromptQueuePromotionClaim {
    state: Arc<StdMutex<PromptStateOwnerState>>,
    key: PromptStateKey,
}

impl Drop for PromptQueuePromotionClaim {
    fn drop(&mut self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .queue_promotions
            .remove(&self.key);
    }
}

impl PromptStateOwner {
    pub(crate) fn try_claim_idle_queue_promotion(
        &self,
        session: &RuntimeSession,
        agent_id: &str,
    ) -> Option<PromptQueuePromotionClaim> {
        let mut owner = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = PromptStateKey::new(session.id(), agent_id);
        if owner.profile_transitions.contains_key(&key) || owner.queue_promotions.contains(&key) {
            return None;
        }
        let state = owner.ensure_agent_state(session, agent_id);
        if state.active_prompt.is_some()
            || state
                .queued_prompts
                .front()
                .is_none_or(|prompt| prompt.remote_steer_reserved())
        {
            return None;
        }
        owner.queue_promotions.insert(key.clone());
        Some(PromptQueuePromotionClaim {
            state: self.state.clone(),
            key,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_promotion_claim_deduplicates_preparation_and_releases_for_retry() {
        let owner = PromptStateOwner::default();
        let session =
            RuntimeSession::new("room", None, "workspace", "worktree", "machine", "kernel");
        assert!(owner
            .try_claim_idle_queue_promotion(&session, "agent")
            .is_none());
        let transition = owner
            .claim_idle_agent_profile_transition(&session, "agent")
            .unwrap();
        owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "older",
                    "client",
                    "agent",
                    "older request",
                    PromptStatus::Queued,
                ),
                false,
            )
            .unwrap();
        assert!(owner
            .try_claim_idle_queue_promotion(&session, "agent")
            .is_none());
        drop(transition);
        let preparing = owner
            .try_claim_idle_queue_promotion(&session, "agent")
            .unwrap();
        assert!(owner
            .clone()
            .try_claim_idle_queue_promotion(&session, "agent")
            .is_none());
        assert!(
            owner
                .claim_idle_agent_profile_transition(&session, "agent")
                .is_err(),
            "profile reservation must exclude an already-held preparation claim"
        );
        // Failure/cancellation releases only the ephemeral guard; the queue survives.
        drop(preparing);
        let transition = owner
            .claim_idle_agent_profile_transition(&session, "agent")
            .unwrap();
        assert!(owner
            .try_claim_idle_queue_promotion(&session, "agent")
            .is_none());
        drop(transition);
        let retry = owner
            .try_claim_idle_queue_promotion(&session, "agent")
            .unwrap();
        owner
            .activate_next_queued_prompt_with_prompt_id(
                &session,
                "agent",
                None,
                "active-older".into(),
            )
            .unwrap();
        drop(retry);
        owner
            .submit_prepared_prompt(
                &session,
                PromptQueueItem::new(
                    "newer",
                    "client",
                    "agent",
                    "newer request",
                    PromptStatus::Queued,
                ),
                false,
            )
            .unwrap();
        assert!(owner
            .try_claim_idle_queue_promotion(&session, "agent")
            .is_none());
    }
}
