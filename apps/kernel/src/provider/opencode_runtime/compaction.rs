//! MP-08 / MP-10: correlate provider-native automatic compaction with an already accepted turn.
use crate::provider::OpenCodeMessage;

use super::OpenCodeRuntimeState;

pub(super) fn record_compaction_continuations(
    state: &mut OpenCodeRuntimeState,
    messages: &[OpenCodeMessage],
) {
    let Some(active) = state.active_user_message_id.as_deref() else {
        return;
    };
    let Some(index) = messages.iter().position(|message| {
        message.info.id == active
            && message.info.session_id == state.session_id
            && message.info.role == "user"
    }) else {
        return;
    };
    let submitted_at = messages[index].info.time.created;
    if submitted_at == 0 {
        return;
    }
    // OpenCode may start compaction before this prompt arrives. Its summary
    // starts earlier, but completes while this accepted user is queued.
    let summaries = messages
        .iter()
        .filter_map(|message| {
            if message.info.session_id != state.session_id
                || !message.info.is_compaction_summary()
                || message.info.error.is_some()
            {
                return None;
            }
            let completed = message.info.time.completed?;
            if completed < submitted_at {
                return None;
            }
            let parent = message.info.parent_id.as_deref()?;
            let automatic = messages.iter().any(|trigger| {
                trigger.info.session_id == state.session_id
                    && trigger.info.role == "user"
                    && trigger.info.id == parent
                    && trigger.parts.iter().any(|part| {
                        part.session_id == state.session_id
                            && part.message_id == trigger.info.id
                            && part.kind == "compaction"
                            && part.auto
                    })
            });
            automatic.then_some(completed)
        })
        .collect::<Vec<_>>();
    for message in &messages[index + 1..] {
        if message.info.session_id != state.session_id || message.info.role != "user" {
            continue;
        }
        // A compaction trigger is provider context, not another user prompt.
        if !message.parts.is_empty()
            && message.parts.iter().all(|part| {
                part.session_id == state.session_id
                    && part.message_id == message.info.id
                    && part.kind == "compaction"
                    && part.auto
            })
        {
            continue;
        }
        let synthetic = !message.parts.is_empty()
            && message.parts.iter().all(|part| {
                part.session_id == state.session_id
                    && part.message_id == message.info.id
                    && part.kind == "text"
                    && part.synthetic
            });
        let follows_summary = summaries
            .iter()
            .any(|completed| *completed <= message.info.time.created);
        if !synthetic
            || !follows_summary
            || state.preexisting_message_ids.contains(&message.info.id)
        {
            // Never cross a real intervening user message or baseline context.
            break;
        }
        state
            .active_continuation_user_ids
            .insert(message.info.id.clone());
    }
}
