//! Protocol 394: the passkey prompts one subscription has been sent. Every
//! subscription, session or waiting room, local or relayed, carries the
//! popups of its user when its connection may submit a passkey, so a
//! terminal gets them whether or not it is attached to the prompt's session.
use crate::local::{KernelConnectionClass, PasskeyPrompt};
use crate::runtime::router::CommandRouter;
use crate::transport::kernel_protocol::KernelEvent;

pub(crate) struct PasskeyPromptFeed {
    /// `None` when the connection may not submit a passkey: no popups.
    user_id: Option<String>,
    sequence: u64,
    sent: Option<Vec<PasskeyPrompt>>,
}

impl PasskeyPromptFeed {
    pub(crate) fn new(connection_class: KernelConnectionClass, user_id: &str) -> Self {
        Self {
            user_id: connection_class
                .may_submit_passkey()
                .then(|| user_id.to_owned()),
            sequence: 0,
            sent: None,
        }
    }

    /// The event to send now: the user's current prompts, at the start and
    /// whenever they differ from the last ones sent. They are looked up again
    /// only after a change or once a sent prompt reached its expiry.
    pub(crate) fn next_event(&mut self, router: &CommandRouter) -> Option<KernelEvent> {
        let user_id = self.user_id.as_deref()?;
        let sequence = router.passkey_prompt_change_sequence();
        let now_ms = crate::session::unix_epoch_ms();
        if let Some(sent) = &self.sent {
            if sequence == self.sequence && sent.iter().all(|prompt| now_ms < prompt.expires_at_ms)
            {
                return None;
            }
        }
        self.sequence = sequence;
        let prompts = router.passkey_prompts_for(user_id);
        if self.sent.as_ref() == Some(&prompts) {
            return None;
        }
        self.sent = Some(prompts.clone());
        Some(KernelEvent::PasskeyPromptsChanged { prompts })
    }

    /// Resolves once the prompts may have changed since `next_event`; never
    /// for a subscription that gets no prompts.
    pub(crate) async fn changed(&self, router: &CommandRouter) {
        if self.user_id.is_none() {
            return std::future::pending().await;
        }
        router
            .wait_for_passkey_prompt_change_after(self.sequence)
            .await;
    }
}
