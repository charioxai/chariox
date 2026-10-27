use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::session::{AgentPromptState, PromptOrigin, PromptStatus};

pub(crate) const QUEUED_PROMPT_STEER_EXTERNAL_REASON: &str =
    "Steering is unavailable while the active provider turn was started outside Chariox.";
const QUEUED_PROMPT_STALE_REASON: &str = "This prompt is no longer waiting in the queue.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentQueuedPromptControlProjection {
    pub prompt_id: String,
    pub status: String,
    pub can_steer: bool,
    pub can_cancel: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steer_disabled_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_disabled_reason: Option<String>,
}

pub(crate) fn queued_prompt_controls_projection(
    prompt_state: Option<&AgentPromptState>,
    active_turn_prompt_origin: Option<PromptOrigin>,
) -> BTreeMap<String, AgentQueuedPromptControlProjection> {
    let Some(prompt_state) = prompt_state else {
        return BTreeMap::new();
    };
    let active_prompt_is_external = active_turn_prompt_origin
        .is_some_and(|origin| origin == PromptOrigin::External)
        || prompt_state
            .active_prompt()
            .is_some_and(|prompt| prompt.prompt_origin() == PromptOrigin::External);
    prompt_state
        .queued_prompts()
        .iter()
        .map(|prompt| {
            let queued = prompt.status() == PromptStatus::Queued;
            let can_steer = queued && !active_prompt_is_external;
            let can_cancel = queued;
            let steer_disabled_reason = if !queued {
                Some(QUEUED_PROMPT_STALE_REASON.to_string())
            } else if active_prompt_is_external {
                Some(QUEUED_PROMPT_STEER_EXTERNAL_REASON.to_string())
            } else {
                None
            };
            let cancel_disabled_reason = (!queued).then(|| QUEUED_PROMPT_STALE_REASON.to_string());
            (
                prompt.id().to_string(),
                AgentQueuedPromptControlProjection {
                    prompt_id: prompt.id().to_string(),
                    status: queued_prompt_status_label(prompt.status()).to_string(),
                    can_steer,
                    can_cancel,
                    steer_disabled_reason,
                    cancel_disabled_reason,
                },
            )
        })
        .collect()
}

fn queued_prompt_status_label(status: PromptStatus) -> &'static str {
    match status {
        PromptStatus::Queued => "queued",
        PromptStatus::Dispatching => "dispatching",
        PromptStatus::Running => "running",
        PromptStatus::Cancelling => "cancelling",
        PromptStatus::Completed => "completed",
        PromptStatus::Cancelled => "cancelled",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::PromptQueueItem;

    #[test]
    fn queued_prompt_controls_allow_steering_when_chariox_turn_is_active() {
        let prompt_state = prompt_state(
            Some(PromptQueueItem::new(
                "active-1",
                "attach-1",
                "agent-1",
                "active",
                PromptStatus::Running,
            )),
            vec![PromptQueueItem::new(
                "queued-1",
                "attach-1",
                "agent-1",
                "queued",
                PromptStatus::Queued,
            )],
        );

        let controls =
            queued_prompt_controls_projection(Some(&prompt_state), Some(PromptOrigin::Chariox));
        let control = controls
            .get("queued-1")
            .expect("queued prompt control should exist");

        assert_eq!(control.status, "queued");
        assert!(control.can_steer);
        assert!(control.can_cancel);
        assert!(control.steer_disabled_reason.is_none());
        assert!(control.cancel_disabled_reason.is_none());
    }

    #[test]
    fn queued_prompt_controls_block_steering_behind_external_turn() {
        let prompt_state = prompt_state(
            Some(
                PromptQueueItem::new(
                    "external:codex:thread-1:user-1",
                    "external:codex",
                    "agent-1",
                    "external",
                    PromptStatus::Running,
                )
                .with_prompt_origin(PromptOrigin::External),
            ),
            vec![PromptQueueItem::new(
                "queued-1",
                "attach-1",
                "agent-1",
                "queued",
                PromptStatus::Queued,
            )],
        );

        let controls = queued_prompt_controls_projection(Some(&prompt_state), None);
        let control = controls
            .get("queued-1")
            .expect("queued prompt control should exist");

        assert!(!control.can_steer);
        assert!(control.can_cancel);
        assert_eq!(
            control.steer_disabled_reason.as_deref(),
            Some(QUEUED_PROMPT_STEER_EXTERNAL_REASON)
        );
        assert!(control.cancel_disabled_reason.is_none());
    }

    #[test]
    fn queued_prompt_controls_block_steering_from_external_active_turn_origin() {
        let prompt_state = prompt_state(
            None,
            vec![PromptQueueItem::new(
                "queued-1",
                "attach-1",
                "agent-1",
                "queued",
                PromptStatus::Queued,
            )],
        );

        let controls =
            queued_prompt_controls_projection(Some(&prompt_state), Some(PromptOrigin::External));
        let control = controls
            .get("queued-1")
            .expect("queued prompt control should exist");

        assert_eq!(control.status, "queued");
        assert!(!control.can_steer);
        assert!(control.can_cancel);
        assert_eq!(
            control.steer_disabled_reason.as_deref(),
            Some(QUEUED_PROMPT_STEER_EXTERNAL_REASON)
        );
        assert!(control.cancel_disabled_reason.is_none());
    }

    #[test]
    fn queued_prompt_controls_mark_stale_prompts_not_actionable() {
        let prompt_state = prompt_state(
            None,
            vec![PromptQueueItem::new(
                "cancelled-1",
                "attach-1",
                "agent-1",
                "cancelled",
                PromptStatus::Cancelled,
            )],
        );

        let controls = queued_prompt_controls_projection(Some(&prompt_state), None);
        let control = controls
            .get("cancelled-1")
            .expect("stale prompt control should exist");

        assert_eq!(control.status, "cancelled");
        assert!(!control.can_steer);
        assert!(!control.can_cancel);
        assert_eq!(
            control.steer_disabled_reason.as_deref(),
            Some(QUEUED_PROMPT_STALE_REASON)
        );
        assert_eq!(
            control.cancel_disabled_reason.as_deref(),
            Some(QUEUED_PROMPT_STALE_REASON)
        );
    }

    fn prompt_state(
        active_prompt: Option<PromptQueueItem>,
        queued_prompts: Vec<PromptQueueItem>,
    ) -> AgentPromptState {
        serde_json::from_value(serde_json::json!({
            "active_prompt": active_prompt,
            "queued_prompts": queued_prompts,
        }))
        .expect("prompt state should deserialize")
    }
}
