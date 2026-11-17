use serde::{Deserialize, Serialize};

use super::{unix_epoch_ms, PromptAttachment};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedMetaagentTask {
    id: String,
    metaagent_id: String,
    source_attachment_id: String,
    task_markdown: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<PromptAttachment>,
    created_at_ms: u64,
    // MP-08 / MP-10 / MP-11: admitted user intent travels privately through
    // the existing prompt journal, never through client-supplied task JSON.
    #[serde(skip)]
    skill_intent: Option<Box<SkillIntent>>,
}

/// The skills a user chose, and whether they typed `$name` syntax, for one prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillIntent {
    pub selections: Vec<crate::skill::v2::SkillSelection>,
    pub user_syntax: bool,
}

impl QueuedMetaagentTask {
    pub fn new(
        id: impl Into<String>,
        metaagent_id: impl Into<String>,
        source_attachment_id: impl Into<String>,
        task_markdown: impl Into<String>,
        attachments: Vec<PromptAttachment>,
    ) -> Self {
        Self {
            id: id.into(),
            metaagent_id: metaagent_id.into(),
            source_attachment_id: source_attachment_id.into(),
            task_markdown: task_markdown.into(),
            attachments,
            created_at_ms: unix_epoch_ms(),
            skill_intent: None,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn metaagent_id(&self) -> &str {
        &self.metaagent_id
    }

    pub fn source_attachment_id(&self) -> &str {
        &self.source_attachment_id
    }

    pub fn task_markdown(&self) -> &str {
        &self.task_markdown
    }

    pub fn attachments(&self) -> &[PromptAttachment] {
        &self.attachments
    }

    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    pub(crate) fn with_skill_intent(mut self, intent: SkillIntent) -> Self {
        self.skill_intent =
            (!intent.selections.is_empty() || intent.user_syntax).then(|| Box::new(intent));
        self
    }
    pub(crate) fn skill_selections(&self) -> &[crate::skill::v2::SkillSelection] {
        self.skill_intent
            .as_ref()
            .map(|intent| intent.selections.as_slice())
            .unwrap_or_default()
    }
    pub(crate) fn user_skill_syntax(&self) -> bool {
        self.skill_intent
            .as_ref()
            .is_some_and(|intent| intent.user_syntax)
    }
    pub(crate) fn durable_skill_intent(
        &self,
        session_id: &str,
    ) -> Option<super::DurablePromptPrivateState> {
        self.skill_intent.as_ref()?;
        let prompt = super::PromptQueueItem::new(
            self.id(),
            self.source_attachment_id(),
            self.metaagent_id(),
            "",
            super::PromptStatus::Queued,
        )
        .with_skill_selections(self.skill_selections().to_vec())
        .with_user_skill_syntax(self.user_skill_syntax());
        super::DurablePromptPrivateState::from_prompt(session_id, &prompt)
    }
    pub(crate) fn restore_skill_intent(&mut self, state: &super::DurablePromptPrivateState) {
        if state.prompt_id != self.id {
            return;
        }
        let mut prompt = super::PromptQueueItem::new(
            self.id(),
            self.source_attachment_id(),
            self.metaagent_id(),
            "",
            super::PromptStatus::Queued,
        );
        prompt.restore_durable_private_state(state);
        // Queued work has no dispatch authority, loaded body or reserved budget.
        self.skill_intent = Some(Box::new(SkillIntent {
            selections: prompt.skill_selections().to_vec(),
            user_syntax: prompt.user_skill_syntax(),
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn skill_intent_survives_the_existing_private_checkpoint_and_rejects_public_forgery() {
        let selection = crate::skill::v2::SkillSelection::Chariox {
            skill_id: "review".into(),
            name: "review".into(),
            catalog_revision: 3,
        };
        let task = QueuedMetaagentTask::new(
            "session-task:1",
            "agent",
            "attachment",
            "Use selected instructions",
            vec![],
        )
        .with_skill_intent(SkillIntent {
            selections: vec![selection.clone()],
            user_syntax: true,
        });
        let mut session = crate::session::RuntimeSession::new(
            "session",
            None,
            "workspace",
            "worktree",
            "machine",
            "kernel",
        );
        session.enqueue_metaagent_task(task);
        let private = session.durable_prompt_private_states();
        assert_eq!(private.len(), 1);
        let mut public = serde_json::to_value(&session).unwrap();
        assert!(public["queued_metaagent_tasks"][0]
            .get("skill_intent")
            .is_none());
        public["queued_metaagent_tasks"][0]["skill_intent"] =
            serde_json::json!({"selections":[selection],"user_syntax":true});
        let mut restored: crate::session::RuntimeSession = serde_json::from_value(public).unwrap();
        assert!(restored.queued_metaagent_tasks()[0]
            .skill_selections()
            .is_empty());
        assert!(!restored.queued_metaagent_tasks()[0].user_skill_syntax());
        restored.restore_durable_prompt_private_states(&private);
        let restored = &restored.queued_metaagent_tasks()[0];
        assert_eq!(restored.skill_selections(), &[selection]);
        assert!(restored.user_skill_syntax());
        assert!(
            private[0].skill_turn_authority.is_none(),
            "queued work has no admitted run/budget"
        );
    }
}
