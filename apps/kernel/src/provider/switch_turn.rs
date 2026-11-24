//! Fit a provider switch against the model window and the dispatch transport.
use crate::error::DaemonError;
use crate::prompt_assembly::PromptEnvelope;

use super::{ProviderProcessServiceStore, RuntimeProviderRun};

pub(crate) type RenderHandoff<'a> = dyn Fn(usize) -> Result<String, DaemonError> + 'a;

const FAILURE_PREFIX: &str = "The provider switch cannot fit ";

pub(crate) fn too_large(component: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "fit provider switch",
        message: format!("{FAILURE_PREFIX}{component} in the new provider's turn."),
    }
}

pub(crate) fn needs_fresh_agent(reason: &str) -> bool {
    reason.starts_with(FAILURE_PREFIX)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SwitchSkillBudget {
    pub(crate) model_room: usize,
    pub(crate) delivery_room: usize,
    pub(crate) skill_room: usize,
    pub(crate) preferred_handoff_bytes: usize,
}

impl SwitchSkillBudget {
    pub(crate) fn full_skill_room(self) -> usize {
        self.delivery_room
            .saturating_sub(self.preferred_handoff_bytes)
            .min(self.skill_room)
    }
}

fn model_room(run: &RuntimeProviderRun, envelope: &PromptEnvelope) -> usize {
    let usage = run.usage();
    let window = usage.context_window.unwrap_or_else(|| {
        super::effective_model_context_window_tokens(
            run.provider(),
            run.model(),
            super::claude_1m_context_disabled(run),
        )
    });
    let window = run
        .pty_env()
        .get("CLAUDE_CODE_AUTO_COMPACT_WINDOW")
        .and_then(|value| value.parse::<u64>().ok())
        .map_or(window, |configured| window.min(configured));
    window
        .saturating_sub(usage.context_tokens.unwrap_or_default())
        .saturating_sub(envelope.visible_user_prompt.len() as u64)
        .saturating_sub(envelope.hidden_system_context.len() as u64)
        .saturating_sub(attachment_bytes(envelope) as u64)
        .min(usize::MAX as u64) as usize
}

fn delivery_room(run: &RuntimeProviderRun, envelope: &PromptEnvelope, native_hook: bool) -> usize {
    let room = model_room(run, envelope);
    if native_hook {
        room.min(super::claude_native_hidden_context_room(
            &envelope.hidden_system_context,
        ))
    } else {
        // The frame includes lengths in decimal. Allow the context length to
        // grow from a single digit to the largest possible usize.
        room.saturating_sub(
            super::encode_account_handoff("", "").len() + 2 * usize::MAX.ilog10() as usize,
        )
    }
}

pub(crate) struct FittedHandoff {
    pub(crate) text: String,
    pub(crate) room: usize,
}

fn attachment_bytes(envelope: &PromptEnvelope) -> usize {
    envelope.attachments.iter().fold(0, |total, attachment| {
        total.saturating_add(attachment.contents_base64().map_or(0, str::len))
    })
}

fn largest_required_input(envelope: &PromptEnvelope) -> &'static str {
    [
        ("the message", envelope.visible_user_prompt.len()),
        ("the attachments", attachment_bytes(envelope)),
        (
            "the required instructions",
            envelope.hidden_system_context.len(),
        ),
    ]
    .into_iter()
    .max_by_key(|(_, bytes)| *bytes)
    .map_or("the turn", |(component, _)| component)
}

impl ProviderProcessServiceStore {
    pub(crate) fn prepare_switch_skill_prompt(
        &self,
        run: &RuntimeProviderRun,
        prompt_id: &str,
        origin_prompt_id: &str,
        mut envelope: PromptEnvelope,
        native_hook: bool,
        render_handoff: &RenderHandoff<'_>,
    ) -> Result<(PromptEnvelope, Option<FittedHandoff>), DaemonError> {
        let preferred = render_handoff(usize::MAX)?;
        let agent_id = run
            .agent_instance_id()
            .ok_or_else(|| DaemonError::InternalInvariant {
                operation: "fit provider switch",
                message: "The provider run has no agent".into(),
            })?;
        if preferred.is_empty() {
            let envelope = self.prepare_skill_prompt(
                run.session_id(),
                agent_id,
                prompt_id,
                origin_prompt_id,
                run,
                envelope,
            )?;
            return Ok((envelope, None));
        }
        let required_input = largest_required_input(&envelope);
        let room_without_skills = delivery_room(run, &envelope, native_hook);
        // Even on Claude's structured path, the skill section is sent through
        // the hook. The handoff shares that ceiling only for native input.
        let skill_room =
            if native_hook || run.pty_env().contains_key("CHARIOX_CLAUDE_NATIVE_CONTEXT") {
                super::claude_native_hidden_context_room(&envelope.hidden_system_context)
            } else {
                room_without_skills
            };
        let budget = SwitchSkillBudget {
            model_room: model_room(run, &envelope),
            delivery_room: room_without_skills.saturating_sub(2),
            skill_room,
            preferred_handoff_bytes: preferred.len(),
        };
        envelope.switch_skill_budget = Some(budget);
        let original_prompt_bytes = envelope.visible_user_prompt.len();
        let mut envelope = self.prepare_skill_prompt(
            run.session_id(),
            agent_id,
            prompt_id,
            origin_prompt_id,
            run,
            envelope,
        )?;
        envelope.switch_skill_budget = None;
        let room = delivery_room(run, &envelope, native_hook);
        let text = if preferred.len() <= room {
            preferred
        } else {
            render_handoff(room)?
        };
        crate::logging::info_with_fields(
            "daemon.provider_context_handoff",
            "fitted provider switch context",
            serde_json::json!({
                "target_provider_run_id": run.id(),
                "prompt_id": prompt_id,
                "handoff_bytes_before": budget.preferred_handoff_bytes,
                "handoff_bytes_after": text.len(),
                "handoff_room_bytes": room,
                "request_bytes": original_prompt_bytes,
            }),
        );
        if text.is_empty() || text.len() > room {
            let component =
                if room == room_without_skills || render_handoff(room_without_skills)?.is_empty() {
                    format!("{required_input} together with the handoff brief")
                } else {
                    "the handoff brief and skill references together with the required turn context"
                        .into()
                };
            return Err(too_large(&component));
        }
        Ok((envelope, Some(FittedHandoff { text, room })))
    }
}
