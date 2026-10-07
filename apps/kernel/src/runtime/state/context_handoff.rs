use std::collections::BTreeMap;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard};

use crate::provider::RuntimeProviderRun;

mod brief;
pub(super) mod brief_refresh;
mod builder;
mod facts;
use builder::{load_agent_conversation, AgentConversation};

const HIDDEN_HANDOFF_SUFFIX: &str = "The active user request is supplied separately.";
/// The packet's share of the target model's window, and the bytes it allows
/// per token: conservative, since code and paths tokenize densely.
const HANDOFF_WINDOW_PERCENT: u64 = 15;
const HANDOFF_BYTES_PER_TOKEN: u64 = 3;
/// The prompt-prefix transport's cap, whatever the window.
const MAX_PROMPT_HANDOFF_BYTES: u64 = 256_000;

#[derive(Debug, Clone)]
pub(super) struct PendingAgentContextHandoff {
    pub(super) source_provider: String,
    pub(super) source_model: String,
    pub(super) target_provider_run_id: Option<String>,
    pub(super) target_provider: String,
    pub(super) target_account_profile: String,
    pub(super) target_model: Option<String>,
    /// Captured from the target launch environment; internal, not serialized.
    target_1m_context_disabled: bool,
    pub(super) conversation: AgentConversation,
    /// Derived from history for a provider switch, so it carries the agent's
    /// handoff brief.
    pub(super) derived: bool,
}

/// Explicit handoffs for runs that start outside the agent's own conversation:
/// a fork target and a turn substitute. Profile switches need no entry; the
/// kernel derives them from history at dispatch.
#[derive(Debug, Clone, Default)]
pub(super) struct PendingAgentContextHandoffStore {
    inner: Arc<StdMutex<BTreeMap<String, PendingAgentContextHandoff>>>,
    /// Each agent's run known to hold its conversation, so later prompts to it
    /// skip the history checks and the derived handoff is delivered once.
    conversation_runs: Arc<StdMutex<BTreeMap<String, String>>>,
    /// Unsupported brief models are remembered only for this kernel lifetime,
    /// and only for the selected provider account and exact model.
    unavailable_brief_models: Arc<StdMutex<std::collections::BTreeSet<(String, String, String)>>>,
}

impl PendingAgentContextHandoffStore {
    pub(super) fn brief_model_is_unavailable(&self, run: &RuntimeProviderRun) -> bool {
        self.unavailable_brief_models
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(&(
                run.provider().to_string(),
                run.account_profile().to_string(),
                run.model().to_string(),
            ))
    }

    pub(super) fn remember_unavailable_brief_model(&self, run: &RuntimeProviderRun) {
        self.unavailable_brief_models
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert((
                run.provider().to_string(),
                run.account_profile().to_string(),
                run.model().to_string(),
            ));
    }

    fn write(&self) -> StdMutexGuard<'_, BTreeMap<String, PendingAgentContextHandoff>> {
        self.inner
            .lock()
            .expect("pending agent context handoff mutex poisoned")
    }

    pub(super) fn set(
        &self,
        session_id: &str,
        agent_id: &str,
        handoff: PendingAgentContextHandoff,
    ) {
        self.write()
            .insert(pending_handoff_key(session_id, agent_id), handoff);
    }

    pub(super) fn clear(&self, session_id: &str, agent_id: &str) {
        self.write()
            .remove(&pending_handoff_key(session_id, agent_id));
    }

    pub(super) fn peek_matching(
        &self,
        session_id: &str,
        agent_id: &str,
        target_run: &RuntimeProviderRun,
    ) -> Option<PendingAgentContextHandoff> {
        self.write()
            .get(&pending_handoff_key(session_id, agent_id))
            .filter(|handoff| handoff.matches_target(target_run))
            .cloned()
    }

    fn run_holds_conversation(&self, session_id: &str, agent_id: &str, run_id: &str) -> bool {
        self.conversation_runs
            .lock()
            .expect("agent conversation run mutex poisoned")
            .get(&pending_handoff_key(session_id, agent_id))
            .is_some_and(|holder| holder == run_id)
    }

    fn note_run_holds_conversation(&self, session_id: &str, agent_id: &str, run_id: &str) {
        self.conversation_runs
            .lock()
            .expect("agent conversation run mutex poisoned")
            .insert(
                pending_handoff_key(session_id, agent_id),
                run_id.to_string(),
            );
    }

    pub(super) fn consume_matching(
        &self,
        session_id: &str,
        agent_id: &str,
        target_run: &RuntimeProviderRun,
    ) -> Option<PendingAgentContextHandoff> {
        let key = pending_handoff_key(session_id, agent_id);
        let mut handoffs = self.write();
        if handoffs
            .get(&key)
            .is_some_and(|handoff| handoff.matches_target(target_run))
        {
            handoffs.remove(&key)
        } else {
            None
        }
    }
}

impl PendingAgentContextHandoff {
    pub(super) fn needs_brief(&self) -> bool {
        self.derived && !self.conversation.fits_without_brief(self.budget())
    }

    fn matches_target(&self, target_run: &RuntimeProviderRun) -> bool {
        self.target_provider == target_run.provider()
            && self.target_account_profile == target_run.account_profile()
            && self
                .target_provider_run_id
                .as_deref()
                .is_none_or(|run_id| run_id == target_run.id())
            && self
                .target_model
                .as_deref()
                .is_none_or(|model| model == target_run.model())
    }

    /// The packet size the target allows: 15% of its model's window, within
    /// the prompt transport's cap.
    fn budget(&self) -> usize {
        let window = crate::provider::effective_model_context_window_tokens(
            &self.target_provider,
            self.target_model.as_deref().unwrap_or_default(),
            self.target_1m_context_disabled,
        );
        (window * HANDOFF_WINDOW_PERCENT / 100 * HANDOFF_BYTES_PER_TOKEN)
            .min(MAX_PROMPT_HANDOFF_BYTES) as usize
    }

    /// The handoff as the provider reads it, in at most `max_bytes`.
    fn render(&self, max_bytes: usize) -> Option<String> {
        let switch = format!(
            "Provider/account switch: {} ({}) -> {} [{}] ({}).",
            self.source_provider,
            model_label(Some(&self.source_model)),
            self.target_provider,
            self.target_account_profile,
            model_label(self.target_model.as_deref()),
        );
        let context = self
            .conversation
            .render(max_bytes.checked_sub(switch.len() + 2)?)?;
        let handoff = format!("{context}\n\n{switch}");
        crate::logging::info_with_fields(
            "daemon.provider_context_handoff",
            "rendered the agent conversation handoff",
            serde_json::json!({
                "target_provider_run_id": self.target_provider_run_id,
                "max_bytes": max_bytes,
                "context_bytes": handoff.len(),
                "brief_bytes": self.conversation.brief.as_ref().map(String::len),
            }),
        );
        Some(handoff)
    }

    /// The handoff as Claude native hidden context, sized to `room`: the space
    /// the turn's other hidden context leaves under the hook ceiling.
    pub(super) fn render_hidden(&self, room: usize) -> String {
        room.min(self.budget())
            .checked_sub(HIDDEN_HANDOFF_SUFFIX.len() + 2)
            .and_then(|max_bytes| self.render(max_bytes))
            .map(|handoff| format!("{handoff}\n\n{HIDDEN_HANDOFF_SUFFIX}"))
            .unwrap_or_default()
    }
}

pub(super) fn inject_context_handoff(prompt: &str, handoff: &PendingAgentContextHandoff) -> String {
    handoff
        .render(handoff.budget())
        .map(|context| crate::provider::encode_account_handoff(&context, prompt))
        .unwrap_or_else(|| prompt.to_string())
}

impl super::KernelRuntimeOwnedState {
    /// A turn substitute runs in a new provider session, and the configured
    /// profile's own session never sees the substitute's turn. Each side of
    /// that boundary gets the conversation, even when the profiles differ only
    /// in effort.
    pub(super) fn prepare_turn_substitute_context_handoff(
        &self,
        source_run: &RuntimeProviderRun,
        target_provider_run_id: Option<&str>,
        target_provider: &str,
        target_account_profile: &str,
        target_model: Option<&str>,
    ) {
        let Some(agent_id) = source_run.agent_instance_id() else {
            return;
        };
        self.prepare_context_handoff_for_target(
            source_run,
            source_run.session_id(),
            agent_id,
            agent_id,
            target_provider_run_id,
            target_provider,
            target_account_profile,
            target_model,
        );
    }

    pub(super) fn prepare_agent_fork_context_handoff(
        &self,
        source_run: &RuntimeProviderRun,
        target_agent_id: &str,
        target_run: &RuntimeProviderRun,
    ) {
        if source_run.session_id() != target_run.session_id() {
            return;
        }
        let source_agent_id = source_run.agent_instance_id().unwrap_or(target_agent_id);
        self.prepare_context_handoff_for_target(
            source_run,
            target_run.session_id(),
            source_agent_id,
            target_agent_id,
            Some(target_run.id()),
            target_run.provider(),
            target_run.account_profile(),
            Some(target_run.model()),
        );
    }

    fn prepare_context_handoff_for_target(
        &self,
        source_run: &RuntimeProviderRun,
        session_id: &str,
        source_history_agent_id: &str,
        agent_id: &str,
        target_provider_run_id: Option<&str>,
        target_provider: &str,
        target_account_profile: &str,
        target_model: Option<&str>,
    ) {
        self.pending_agent_context_handoffs
            .clear(session_id, agent_id);
        if let Some(conversation) =
            self.agent_conversation(session_id, source_history_agent_id, None)
        {
            self.pending_agent_context_handoffs.set(
                session_id,
                agent_id,
                PendingAgentContextHandoff {
                    source_provider: source_run.provider().to_string(),
                    source_model: source_run.model().to_string(),
                    target_provider_run_id: target_provider_run_id.map(str::to_string),
                    target_provider: target_provider.to_string(),
                    target_account_profile: target_account_profile.to_string(),
                    target_model: target_model.map(str::to_string),
                    target_1m_context_disabled: false,
                    conversation,
                    derived: false,
                },
            );
        }
    }

    fn agent_conversation(
        &self,
        session_id: &str,
        agent_id: &str,
        dispatching_prompt_id: Option<&str>,
    ) -> Option<AgentConversation> {
        load_agent_conversation(
            &self.operational_history_store,
            session_id,
            agent_id,
            dispatching_prompt_id,
            &self.room_secret_observations,
        )
        .map(|conversation| (!conversation.is_empty()).then_some(conversation))
        .unwrap_or_else(|error| {
            crate::logging::warn_with_fields(
                "daemon.provider_context_handoff",
                "failed to build the agent conversation handoff",
                serde_json::json!({
                    "session_id": session_id,
                    "agent_id": agent_id,
                    "error": error.to_string(),
                }),
            );
            None
        })
    }

    /// The handoff a prompt to `target_run` carries: an explicit fork or
    /// substitute handoff, otherwise the agent's conversation whenever the
    /// run's native session does not hold it yet. That covers provider and
    /// account switches, sessions a provider could not resume, and switches
    /// made while no run was live or across a restart. Delivery to the run, or
    /// the session's first answer, ends it. A steering prompt joins a turn that
    /// already carried it.
    pub(super) fn context_handoff_for_dispatch(
        &self,
        session_id: &str,
        agent_id: &str,
        target_run: &RuntimeProviderRun,
        prompt_id: &str,
        steering: bool,
    ) -> Option<PendingAgentContextHandoff> {
        if steering {
            return None;
        }
        let started = std::time::Instant::now();
        if let Some(mut handoff) = self
            .pending_agent_context_handoffs
            .peek_matching(session_id, agent_id, target_run)
        {
            // A substitute is prepared after the failed turn was recorded and
            // receives that turn's prompt as its request.
            handoff.target_1m_context_disabled =
                crate::provider::claude_1m_context_disabled(target_run);
            handoff.conversation = handoff.conversation.before_prompt(prompt_id);
            return Some(handoff);
        }
        if target_run.workflow_fresh_context_node_run_id().is_some()
            || target_run.turn_substitute().is_some()
            || self.pending_agent_context_handoffs.run_holds_conversation(
                session_id,
                agent_id,
                target_run.id(),
            )
        {
            return None;
        }
        let latest = self
            .operational_history_store
            .load_latest_provider_output_event(session_id, agent_id)
            .inspect_err(|error| {
                crate::logging::warn_with_fields(
                    "daemon.provider_context_handoff",
                    "failed to read the agent's latest provider output",
                    serde_json::json!({
                        "session_id": session_id,
                        "agent_id": agent_id,
                        "error": error.to_string(),
                    }),
                );
            })
            .ok()?;
        let run_sessions = [
            target_run.provider_session_id(),
            target_run
                .resume_state()
                .provider_session_id(target_run.adapter_key()),
        ];
        // Run ids restart with the kernel, so only an answer given since this
        // run started is its own.
        let holds_conversation = |latest: &crate::history::HistoryEvent| {
            (latest.provider_run_id.as_deref() == Some(target_run.id())
                && latest.timestamp_ms >= target_run.started_at_ms())
                || run_sessions.into_iter().flatten().any(|run_session| {
                    latest.provider_session_id.as_deref() == Some(run_session)
                        || self
                            .operational_history_store
                            .provider_session_answered_agent(session_id, agent_id, run_session)
                            .unwrap_or_else(|error| {
                                crate::logging::warn_with_fields(
                                    "daemon.provider_context_handoff",
                                    "failed to check whether the provider session answered the agent",
                                    serde_json::json!({
                                        "session_id": session_id,
                                        "agent_id": agent_id,
                                        "provider_session_id": run_session,
                                        "error": error.to_string(),
                                    }),
                                );
                                true
                            })
                })
        };
        // An agent no provider ever answered has its prompts and errors to
        // carry; only an empty conversation leaves nothing to transfer.
        let conversation = match &latest {
            Some(latest) if holds_conversation(latest) => None,
            _ => self.agent_conversation(session_id, agent_id, Some(prompt_id)),
        };
        let Some(mut conversation) = conversation else {
            self.pending_agent_context_handoffs
                .note_run_holds_conversation(session_id, agent_id, target_run.id());
            return None;
        };
        self.add_session_facts(session_id, agent_id, prompt_id, &mut conversation);
        // Structured harnesses load the brief once during refresh. Other harnesses
        // carry the last stored brief alongside their deterministic packet.
        if !brief_refresh::writes_handoff_briefs(target_run) {
            conversation.brief = self.stored_handoff_brief(session_id, agent_id);
        }
        let latest = latest.as_ref();
        crate::logging::info_with_fields(
            "daemon.provider_context_handoff",
            "transferring the agent conversation to a new provider session",
            serde_json::json!({
                "session_id": session_id,
                "agent_id": agent_id,
                "source_provider_run_id": latest.and_then(|latest| latest.provider_run_id.as_deref()),
                "source_provider_session_id": latest.and_then(|latest| latest.provider_session_id.as_deref()),
                "target_provider_run_id": target_run.id(),
                "derive_ms": started.elapsed().as_millis() as u64,
            }),
        );
        Some(PendingAgentContextHandoff {
            source_provider: latest
                .and_then(|latest| latest.provider.clone())
                .unwrap_or_default(),
            source_model: latest
                .and_then(|latest| latest.model.clone())
                .unwrap_or_default(),
            target_provider_run_id: Some(target_run.id().to_string()),
            target_provider: target_run.provider().to_string(),
            target_account_profile: target_run.account_profile().to_string(),
            target_model: Some(target_run.model().to_string()),
            target_1m_context_disabled: crate::provider::claude_1m_context_disabled(target_run),
            conversation,
            derived: true,
        })
    }

    pub(super) fn stored_handoff_brief(&self, session_id: &str, agent_id: &str) -> Option<String> {
        self.protected_stored_handoff_brief(session_id, agent_id)
            .inspect_err(|error| {
                crate::logging::warn_with_fields(
                    "daemon.provider_context_handoff",
                    "failed to load the agent's handoff brief",
                    serde_json::json!({
                        "session_id": session_id,
                        "agent_id": agent_id,
                        "error": error.to_string(),
                    }),
                );
            })
            .ok()
            .flatten()
            .map(|stored| stored.brief)
    }

    /// All persisted brief consumers share current Room cache policy. A rejected
    /// cache cannot keep a watermark that skips the now-protected source history.
    fn protected_stored_handoff_brief(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Option<crate::history::AgentHandoffBrief>, crate::error::DaemonError> {
        let Some(mut stored) = self
            .operational_history_store
            .load_agent_handoff_brief(session_id, agent_id)?
        else {
            return Ok(None);
        };
        match self
            .room_secret_observations
            .scrub_cached_result(session_id, stored.brief)
        {
            Ok(brief) => {
                stored.brief = brief;
                Ok(Some(stored))
            }
            Err(_) => {
                self.operational_history_store
                    .delete_agent_handoff_brief(session_id, agent_id)?;
                Ok(None)
            }
        }
    }

    /// The agent's open interactions and prompts queued behind `prompt_id`.
    fn add_session_facts(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        conversation: &mut AgentConversation,
    ) {
        let Ok(session) = self.session_store.get_session(session_id) else {
            return;
        };
        conversation.facts.open_interactions = session
            .active_interactions()
            .iter()
            .filter(|interaction| interaction.agent_id() == Some(agent_id))
            .map(|interaction| match interaction.title() {
                Some(title) => format!("{title}: {}", interaction.message()),
                None => interaction.message().to_string(),
            })
            .collect();
        let (_, queued) = self.prompt_state_owner.state_parts(&session, agent_id);
        conversation.facts.queued_prompts = queued
            .iter()
            .filter(|queued| queued.id() != prompt_id)
            .map(|queued| queued.prompt().to_string())
            .collect();
    }

    pub(super) fn prompt_with_pending_context_handoff(
        &self,
        session_id: &str,
        agent_id: &str,
        target_run: &RuntimeProviderRun,
        prompt_id: &str,
        prompt: &str,
        steering: bool,
    ) -> String {
        self.context_handoff_for_dispatch(session_id, agent_id, target_run, prompt_id, steering)
            .map(|handoff| inject_context_handoff(prompt, &handoff))
            .unwrap_or_else(|| prompt.to_string())
    }

    /// The run received the prompt, and with it any handoff it carried.
    pub(super) fn consume_pending_context_handoff(
        &self,
        session_id: &str,
        agent_id: &str,
        target_run: &RuntimeProviderRun,
    ) {
        let _ = self
            .pending_agent_context_handoffs
            .consume_matching(session_id, agent_id, target_run);
        if target_run.turn_substitute().is_none() {
            self.pending_agent_context_handoffs
                .note_run_holds_conversation(session_id, agent_id, target_run.id());
        }
    }
}

fn pending_handoff_key(session_id: &str, agent_id: &str) -> String {
    format!("{session_id}\n{agent_id}")
}

fn model_label(model: Option<&str>) -> &str {
    let Some(model) = model else {
        return "unknown model";
    };
    if model.trim().is_empty() {
        "unknown model"
    } else {
        model
    }
}

#[cfg(test)]
mod tests {
    mod brief_protection;
    use super::builder::MAX_HANDOFF_BYTES;
    use super::*;
    use crate::runtime::state::KernelRuntimeState;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    async fn owned_runtime_state(app: &Arc<Mutex<crate::DaemonApp>>) -> KernelRuntimeState {
        let (
            config_projection,
            session_state,
            agents,
            attachments,
            providers,
            provider_process_tracking,
            slices,
            session_state_projection,
            provider_run_projection,
            operational_history,
            durable_state,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        ) = {
            let app_locked = app.lock().await;
            (
                app_locked.config_projection_store(),
                app_locked.session_state_store(),
                app_locked.agents().clone(),
                app_locked.attachments().clone(),
                app_locked.providers().clone(),
                app_locked.provider_process_tracking_store(),
                app_locked.slices(),
                app_locked.session_state_projection_store(),
                app_locked.provider_run_projection_store(),
                app_locked.operational_history_store(),
                app_locked.durable_state_store(),
                app_locked.prompt_state_owner(),
                app_locked.active_turn_store(),
                app_locked.prompt_activity_store(),
                app_locked.prompt_workspace_claim_store(),
                app_locked.structured_output_record_store(),
                app_locked.terminal_stream_store(),
                app_locked.workflow_design_event_store(),
                app_locked.metaagent_event_store(),
                app_locked.workspace_coordinator(),
            )
        };
        KernelRuntimeState::new_with_owned_state(
            Arc::clone(app),
            config_projection,
            session_state,
            agents,
            attachments,
            providers,
            provider_process_tracking,
            slices,
            session_state_projection,
            provider_run_projection,
            operational_history,
            durable_state,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        )
    }

    #[test]
    fn pending_handoff_is_consumed_once() {
        let store = PendingAgentContextHandoffStore::default();
        store.set(
            "session",
            "agent",
            PendingAgentContextHandoff {
                source_provider: "opencode".to_string(),
                source_model: "model-old".to_string(),
                target_provider_run_id: Some("run-new".to_string()),
                target_provider: "codex".to_string(),
                target_account_profile: "default".to_string(),
                target_model: Some("model-new".to_string()),
                target_1m_context_disabled: false,
                conversation: conversation("prior context"),
                derived: false,
            },
        );

        let target_run = test_run("run-new", "session", "agent", "codex", "model-new");
        let first = store.peek_matching("session", "agent", &target_run);
        let consumed = store.consume_matching("session", "agent", &target_run);
        let second = store.peek_matching("session", "agent", &target_run);

        assert!(first.is_some());
        assert!(consumed.is_some());
        assert!(second.is_none());
        let injected = inject_context_handoff("next request", &first.unwrap());
        assert!(injected.contains("prior context"));
        assert!(injected.contains("<user_request>\nnext request\n</user_request>"));
        assert_eq!(
            crate::provider::normalized_observed_prompt_text(&injected),
            Some("next request".to_string())
        );
    }

    #[tokio::test]
    async fn fork_context_handoff_uses_source_agent_history_for_target_agent() {
        let mut app = crate::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, source_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-fork-handoff",
                "worktree-fork-handoff",
            ))
            .expect("session should be created");
        let forked_agent = app
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias("forked"),
            )
            .expect("fork target should spawn");
        let source_run = app
            .providers
            .start_run_provider_only(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "dev-stub",
                    "default",
                    "model-source",
                )
                .with_agent_id(source_agent.id()),
            )
            .expect("source provider should start")
            .into_run();
        let target_run = app
            .providers
            .start_run_provider_only(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "dev-stub",
                    "default",
                    "model-source",
                )
                .with_agent_id(forked_agent.id()),
            )
            .expect("target provider should start")
            .into_run();
        let history = app.operational_history_store();
        let prompt_entry = crate::history::SessionHistoryEntry::user_prompt(
            session.id(),
            "attachment-1",
            source_agent.id(),
            "remember source context",
        );
        history
            .append(&crate::history::HistoryEvent::transcript(
                1,
                &prompt_entry,
                crate::history::HistoryEventTurnContext {
                    provider: Some(source_run.provider().to_string()),
                    model: Some(source_run.model().to_string()),
                    provider_run_id: Some(source_run.id().to_string()),
                    prompt_id: Some("prompt-1".to_string()),
                    turn_id: Some("turn-1".to_string()),
                    ..crate::history::HistoryEventTurnContext::default()
                },
            ))
            .expect("source prompt history should append");
        let output_entry = crate::history::SessionHistoryEntry::provider_output(
            session.id(),
            source_run.id(),
            Some(source_agent.id()),
            crate::terminal::TerminalOutputKind::ProviderOutput,
            None,
            "source answer for fork handoff",
        );
        history
            .append(&crate::history::HistoryEvent::transcript(
                2,
                &output_entry,
                crate::history::HistoryEventTurnContext {
                    provider: Some(source_run.provider().to_string()),
                    model: Some(source_run.model().to_string()),
                    provider_run_id: Some(source_run.id().to_string()),
                    prompt_id: Some("prompt-1".to_string()),
                    turn_id: Some("turn-1".to_string()),
                    ..crate::history::HistoryEventTurnContext::default()
                },
            ))
            .expect("source output history should append");

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        runtime.owned.prepare_agent_fork_context_handoff(
            &source_run,
            forked_agent.id(),
            &target_run,
        );

        let handoff = runtime
            .owned
            .pending_agent_context_handoffs
            .peek_matching(session.id(), forked_agent.id(), &target_run)
            .expect("fork target should receive pending source context handoff");
        assert_eq!(handoff.source_provider, source_run.provider());
        assert_eq!(
            handoff.target_provider_run_id.as_deref(),
            Some(target_run.id())
        );
        let context = handoff.render(MAX_HANDOFF_BYTES).unwrap();
        assert!(context.contains("remember source context"));
        assert!(context.contains("source answer for fork handoff"));
        assert!(runtime
            .owned
            .pending_agent_context_handoffs
            .peek_matching(session.id(), source_agent.id(), &target_run)
            .is_none());
    }

    struct DerivedHandoffFixture {
        runtime: KernelRuntimeState,
        history: crate::history::OperationalHistoryStore,
        session_id: String,
        agent_id: String,
    }

    impl DerivedHandoffFixture {
        async fn new() -> Self {
            let mut app = crate::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
                .expect("daemon bootstrap should succeed");
            let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(crate::session::CreateSessionRequest::new(
                    "workspace-derived-handoff",
                    "worktree-derived-handoff",
                ))
                .expect("session should be created");
            let history = app.operational_history_store();
            let app = Arc::new(Mutex::new(app));
            Self {
                runtime: owned_runtime_state(&app).await,
                history,
                session_id: session.id().to_string(),
                agent_id: agent.id().to_string(),
            }
        }

        fn user(&self, sequence: u64, prompt: &str) {
            self.append(
                sequence,
                crate::history::SessionHistoryEntry::user_prompt(
                    &self.session_id,
                    "attachment-1",
                    &self.agent_id,
                    prompt,
                ),
                crate::history::HistoryEventTurnContext {
                    prompt_id: Some(format!("prompt-{sequence}")),
                    ..crate::history::HistoryEventTurnContext::default()
                },
                None,
            );
        }

        fn error(&self, sequence: u64, run_id: &str, text: &str) {
            self.append(
                sequence,
                crate::history::SessionHistoryEntry::provider_output(
                    &self.session_id,
                    run_id,
                    Some(&self.agent_id),
                    crate::terminal::TerminalOutputKind::ProviderError,
                    None,
                    text,
                ),
                crate::history::HistoryEventTurnContext {
                    provider: Some("codex".to_string()),
                    provider_run_id: Some(run_id.to_string()),
                    prompt_id: Some(format!("prompt-{}", sequence - 1)),
                    ..crate::history::HistoryEventTurnContext::default()
                },
                None,
            );
        }

        /// The handoff a dispatch of the prompt recorded at `sequence` carries.
        fn dispatch(&self, run: &RuntimeProviderRun, sequence: u64) -> Option<String> {
            self.runtime
                .owned
                .context_handoff_for_dispatch(
                    &self.session_id,
                    &self.agent_id,
                    run,
                    &format!("prompt-{sequence}"),
                    false,
                )
                .and_then(|handoff| handoff.render(MAX_HANDOFF_BYTES))
        }

        fn output(
            &self,
            sequence: u64,
            run_id: &str,
            provider: &str,
            provider_session_id: Option<&str>,
            text: &str,
        ) {
            self.output_at(sequence, run_id, provider, provider_session_id, text, None);
        }

        fn output_at(
            &self,
            sequence: u64,
            run_id: &str,
            provider: &str,
            provider_session_id: Option<&str>,
            text: &str,
            timestamp_ms: Option<u64>,
        ) {
            self.append(
                sequence,
                crate::history::SessionHistoryEntry::provider_output(
                    &self.session_id,
                    run_id,
                    Some(&self.agent_id),
                    crate::terminal::TerminalOutputKind::ProviderOutput,
                    None,
                    text,
                ),
                crate::history::HistoryEventTurnContext {
                    provider: Some(provider.to_string()),
                    model: Some(
                        if provider == "codex" {
                            "gpt-5.5"
                        } else {
                            "sonnet"
                        }
                        .to_string(),
                    ),
                    provider_run_id: Some(run_id.to_string()),
                    provider_session_id: provider_session_id.map(str::to_string),
                    prompt_id: Some(format!("prompt-{sequence}")),
                    ..crate::history::HistoryEventTurnContext::default()
                },
                timestamp_ms,
            );
        }

        fn append(
            &self,
            sequence: u64,
            entry: crate::history::SessionHistoryEntry,
            context: crate::history::HistoryEventTurnContext,
            timestamp_ms: Option<u64>,
        ) {
            let mut event = crate::history::HistoryEvent::transcript(sequence, &entry, context);
            if let Some(timestamp_ms) = timestamp_ms {
                event.timestamp_ms = timestamp_ms;
            }
            self.history.append(&event).expect("history should append");
        }

        fn prompt(&self, run: &RuntimeProviderRun) -> String {
            self.runtime.owned.prompt_with_pending_context_handoff(
                &self.session_id,
                &self.agent_id,
                run,
                "prompt-next",
                "next",
                false,
            )
        }

        fn run(
            &self,
            run_id: &str,
            provider: &str,
            provider_session_id: Option<&str>,
        ) -> RuntimeProviderRun {
            let model = if provider == "codex" {
                "gpt-6"
            } else {
                "sonnet"
            };
            test_run_in_session(run_id, &self.agent_id, provider, model, provider_session_id)
        }
    }

    #[tokio::test]
    async fn native_model_switch_keeps_user_next_step_and_later_assistant_risk_distinct() {
        let fixture = DerivedHandoffFixture::new().await;
        // Shape of the real Codex audit: an explicit next step followed by a
        // later assistant risk summary, then a no-tool recall after a model
        // change. The observed paraphrase must not be mistaken for a brief
        // dropping or superseding the user's words.
        let next_step = "Next step: check queued delivery ordering. Inspect queued-prompt promotion logic and summarize the ownership guard.";
        let risk = "With restart continuity prioritized, the main audit risks remain launch-policy ID clearing and unverified serialization between profile changes and queued promotion; persisted metadata and history provide supporting source evidence, not live recall proof.";
        fixture.user(1, next_step);
        fixture.output(2, "run-source", "codex", Some("thread-audit"), "These checks guard against stale delivery, but the inspected source does not establish serialization with profile changes.");
        fixture.user(
            3,
            "Assess the documented protocol contract and list the remaining audit risks.",
        );
        fixture.output(4, "run-source", "codex", Some("thread-audit"), risk);
        fixture.user(5, "Without tools or reading any file, recall our audit conversation. Preserve the original short decision phrases; do not infer missing facts.");

        let resumed = test_run_in_session(
            "run-target",
            &fixture.agent_id,
            "codex",
            "gpt-6-sol",
            Some("thread-audit"),
        );
        assert!(fixture.dispatch(&resumed, 5).is_none());

        // A genuinely fresh session would instead receive both statements in
        // their original order, without treating the risk as a user update.
        let fresh = fixture.run("run-fresh", "codex", None);
        let packet = fixture.dispatch(&fresh, 5).unwrap();
        assert!(packet.contains(next_step), "{packet}");
        assert!(packet.contains(risk), "{packet}");
        assert!(packet.find(next_step).unwrap() < packet.find(risk).unwrap());
    }

    #[tokio::test]
    async fn new_provider_session_receives_the_conversation_until_it_answers() {
        let fixture = DerivedHandoffFixture::new().await;
        fixture.user(1, "remember the codename amber-kestrel");
        fixture.output(2, "run-old", "codex", Some("thread-old"), "OK");

        let resumed = fixture.run("run-resumed", "codex", Some("thread-old"));
        assert_eq!(fixture.prompt(&resumed), "next");

        let fresh = fixture.run("run-new", "claude", None);
        let transferred = fixture.prompt(&fresh);
        assert!(transferred.contains("amber-kestrel"), "{transferred}");
        assert!(transferred
            .contains("Provider/account switch: codex (gpt-5.5) -> claude [default] (sonnet)."));
        assert_eq!(
            crate::provider::normalized_observed_prompt_text(&transferred),
            Some("next".to_string())
        );

        fixture.output(
            3,
            "run-new",
            "claude",
            Some("claude-session"),
            "codename amber-kestrel",
        );
        let answered = fixture.run("run-new-restarted", "claude", Some("claude-session"));
        assert_eq!(fixture.prompt(&answered), "next");
        // A session that answered this agent before holds its own conversation;
        // turns it missed, such as a substitute's, travel as explicit handoffs.
        assert_eq!(fixture.prompt(&resumed), "next");
    }

    #[tokio::test]
    async fn a_new_session_receives_the_conversation_once() {
        let fixture = DerivedHandoffFixture::new().await;
        fixture.user(1, "remember the codename amber-kestrel");
        fixture.output(2, "run-old", "codex", Some("thread-old"), "OK");
        let fresh = fixture.run("run-new", "claude", None);
        assert!(fixture.prompt(&fresh).contains("amber-kestrel"));
        fixture.runtime.owned.consume_pending_context_handoff(
            &fixture.session_id,
            &fixture.agent_id,
            &fresh,
        );

        // A queued prompt, or the next one after a first turn that failed,
        // reaches the same session before it has answered.
        assert_eq!(fixture.prompt(&fresh), "next");
    }

    #[tokio::test]
    async fn a_resumed_session_keeps_its_conversation_after_history_retention() {
        let fixture = DerivedHandoffFixture::new().await;
        fixture.user(1, "remember the codename amber-kestrel");
        fixture.output_at(2, "run-old", "codex", Some("thread-old"), "OK", Some(1_000));
        fixture.user(3, "and the port 41000");
        fixture.output(4, "run-sub", "claude", Some("substitute-session"), "OK");
        fixture
            .history
            .prune_events_before(2_000, true)
            .expect("retention should prune the old answer");

        let resumed = fixture.run("run-resumed", "codex", Some("thread-old"));
        assert_eq!(fixture.prompt(&resumed), "next");
    }

    #[tokio::test]
    async fn a_switch_from_output_without_a_session_id_still_transfers() {
        let fixture = DerivedHandoffFixture::new().await;
        let old = fixture.run("run-old", "codex", None);
        fixture.user(1, "remember the codename amber-kestrel");
        fixture.output(2, "run-old", "codex", None, "OK");

        assert_eq!(fixture.prompt(&old), "next");
        let fresh = fixture.run("run-new", "claude", None);
        assert!(fixture.prompt(&fresh).contains("amber-kestrel"));
    }

    #[tokio::test]
    async fn a_run_id_reused_after_a_restart_does_not_hold_the_conversation() {
        let fixture = DerivedHandoffFixture::new().await;
        fixture.user(1, "remember the codename amber-kestrel");
        // Run ids restart with the kernel: the answer came from an earlier
        // process's `provider-run-1`, not from the run that now has that id.
        fixture.output_at(
            2,
            "provider-run-1",
            "codex",
            Some("thread-old"),
            "OK",
            Some(1_000),
        );

        let fresh = fixture.run("provider-run-1", "claude", None);
        assert!(fixture.prompt(&fresh).contains("amber-kestrel"));
    }

    #[tokio::test]
    async fn a_steering_prompt_joins_the_turn_that_carried_the_handoff() {
        let fixture = DerivedHandoffFixture::new().await;
        fixture.user(1, "remember the codename amber-kestrel");
        fixture.output(2, "run-old", "codex", Some("thread-old"), "OK");
        let fresh = fixture.run("run-new", "claude", None);
        let steer = |steering| {
            fixture.runtime.owned.context_handoff_for_dispatch(
                &fixture.session_id,
                &fixture.agent_id,
                &fresh,
                "prompt-next",
                steering,
            )
        };

        assert!(steer(true).is_none());
        assert!(steer(false).is_some());
    }

    #[tokio::test]
    async fn a_claude_handoff_fits_any_room_the_turn_leaves() {
        let fixture = DerivedHandoffFixture::new().await;
        for index in 0..60u64 {
            fixture.user(
                index * 2 + 1,
                &format!("prior prompt {index} {}", "x".repeat(900)),
            );
            fixture.output(
                index * 2 + 2,
                "run-old",
                "codex",
                Some("thread-old"),
                &format!("prior answer {index} {}", "y".repeat(900)),
            );
        }
        fixture.user(1_000, "latest question about amber-kestrel");
        let fresh = fixture.run("run-new", "claude-headless", None);
        let handoff = fixture
            .runtime
            .owned
            .context_handoff_for_dispatch(
                &fixture.session_id,
                &fixture.agent_id,
                &fresh,
                "prompt-next",
                false,
            )
            .expect("the new session receives the conversation");

        for room in [0, 100, 1_000, 9_000, 18_000, 48_000] {
            let hidden = handoff.render_hidden(room);
            assert!(
                hidden.len() <= room.min(handoff.budget()),
                "{} > {room}",
                hidden.len()
            );
            if room >= 1_000 {
                assert!(
                    hidden.contains("latest question about amber-kestrel"),
                    "{room}"
                );
                assert!(hidden.ends_with(HIDDEN_HANDOFF_SUFFIX));
            }
        }
        assert!(handoff.render_hidden(18_000).contains("prior prompt 59"));
    }

    #[tokio::test]
    async fn the_dispatching_prompt_travels_once_as_the_request() {
        let fixture = DerivedHandoffFixture::new().await;
        fixture.user(1, "remember the codename amber-kestrel");
        fixture.output(
            2,
            "run-old",
            "codex",
            Some("thread-old"),
            "OK, amber-kestrel noted",
        );
        fixture.user(3, "what is the codename?");

        let handoff = fixture
            .dispatch(&fixture.run("run-new", "claude", None), 3)
            .expect("the new session receives the conversation");

        assert!(!handoff.contains("what is the codename?"), "{handoff}");
        assert!(
            handoff.contains("Latest turn:\n- User: remember the codename amber-kestrel"),
            "{handoff}"
        );
    }

    #[tokio::test]
    async fn a_conversation_no_provider_answered_still_transfers() {
        let fixture = DerivedHandoffFixture::new().await;
        fixture.user(1, "first request");
        assert_eq!(
            fixture.dispatch(&fixture.run("run-old", "codex", None), 1),
            None
        );
        fixture.error(2, "run-old", "codex login expired");
        fixture.user(3, "migrate the schema to v7");
        fixture.error(4, "run-old", "quota exhausted until 18:00");
        fixture.user(5, "try again on claude");

        let handoff = fixture
            .dispatch(&fixture.run("run-new", "claude", None), 5)
            .expect("the failed turns travel to the new session");

        assert!(handoff.contains("first request"), "{handoff}");
        assert!(handoff.contains("migrate the schema to v7"), "{handoff}");
        assert!(handoff.contains("quota exhausted until 18:00"), "{handoff}");
        assert!(!handoff.contains("try again on claude"), "{handoff}");
    }

    #[tokio::test]
    async fn a_switch_carries_the_stored_brief_and_the_packet_fits_the_target_window() {
        let fixture = DerivedHandoffFixture::new().await;
        fixture.user(1, "remember the codename amber-kestrel");
        fixture.output(2, "run-old", "codex", Some("thread-old"), "OK");
        fixture
            .history
            .save_agent_handoff_brief(
                &fixture.session_id,
                &fixture.agent_id,
                &crate::history::AgentHandoffBrief {
                    brief: "## Goal\nShip amber-kestrel.\n## Next Steps\nTest.".to_string(),
                    covered_through_sequence: 2,
                },
            )
            .unwrap();

        let handoff = fixture
            .runtime
            .owned
            .context_handoff_for_dispatch(
                &fixture.session_id,
                &fixture.agent_id,
                &fixture.run("run-new", "claude", None),
                "prompt-next",
                false,
            )
            .expect("the new session receives the conversation");

        let handoff = fixture
            .runtime
            .with_current_handoff_brief(
                &fixture.runtime.owned,
                handoff,
                &fixture.session_id,
                &fixture.agent_id,
                "prompt-next",
                &fixture.run("run-new", "claude", None),
                || true,
            )
            .await;
        assert!(handoff.derived);
        assert_eq!(handoff.budget(), MAX_PROMPT_HANDOFF_BYTES as usize);
        assert!(handoff
            .render(handoff.budget())
            .unwrap()
            .contains("Ship amber-kestrel."));
        let codex = PendingAgentContextHandoff {
            target_provider: "codex".to_string(),
            ..handoff
        };
        assert_eq!(codex.budget(), 116_280);
    }

    #[tokio::test]
    async fn claude_handoff_budget_uses_the_target_accounts_effective_context_cap() {
        let fixture = DerivedHandoffFixture::new().await;
        fixture.user(1, "remember amber-kestrel");
        let request = crate::provider::LaunchProviderRequest::new(
            &fixture.session_id,
            "claude",
            "claude",
            "default",
            "sonnet",
        )
        .with_agent_id(&fixture.agent_id);
        let mut target = RuntimeProviderRun::new(
            "capped-target",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "fixture".into(),
                pty_target: None,
                pty_program: None,
                pty_args: vec![],
                pty_env: [("CLAUDE_CODE_DISABLE_1M_CONTEXT".into(), "1".into())].into(),
                pty_env_remove: vec![],
                working_directory: None,
                structured_endpoint: None,
            },
        );
        target.mark_running();
        let capped = fixture
            .runtime
            .owned
            .context_handoff_for_dispatch(
                &fixture.session_id,
                &fixture.agent_id,
                &target,
                "next",
                false,
            )
            .unwrap();
        assert_eq!(
            capped.budget(),
            90_000,
            "the account's 200k cap must constrain the handoff"
        );
        assert!(capped
            .render(capped.budget())
            .unwrap()
            .contains("amber-kestrel"));
    }

    fn test_run_in_session(
        run_id: &str,
        agent_id: &str,
        provider: &str,
        model: &str,
        provider_session_id: Option<&str>,
    ) -> RuntimeProviderRun {
        let mut resume_state = crate::provider::ProviderResumeState::default();
        if let Some(session_id) = provider_session_id {
            resume_state.set_provider_session_id(provider, session_id);
        }
        test_run_from_request(
            run_id,
            crate::provider::LaunchProviderRequest::new(
                "session", provider, provider, "default", model,
            )
            .with_agent_id(agent_id)
            .with_resume_state(resume_state),
        )
    }

    #[test]
    fn codex_briefs_default_to_the_fast_model_and_configuration_wins() {
        let handoff = |source_provider: &str, source_model: &str| PendingAgentContextHandoff {
            source_provider: source_provider.to_string(),
            source_model: source_model.to_string(),
            target_provider_run_id: None,
            target_provider: String::new(),
            target_account_profile: "default".to_string(),
            target_model: None,
            target_1m_context_disabled: false,
            conversation: AgentConversation::default(),
            derived: true,
        };
        let codex = test_run_in_session("run", "agent", "codex", "gpt-5.5", None);
        let claude = test_run_in_session("run", "agent", "claude", "opus", None);
        let brief_model = super::brief_refresh::brief_model;

        assert_eq!(
            brief_model(None, &handoff("claude", "sonnet"), &codex),
            "gpt-6-luna"
        );
        assert_eq!(
            brief_model(None, &handoff("codex", "gpt-6"), &codex),
            "gpt-6-luna"
        );
        assert_eq!(
            brief_model(Some("gpt-5.6-luna"), &handoff("claude", "sonnet"), &codex),
            "gpt-5.6-luna"
        );
        assert_eq!(
            brief_model(None, &handoff("claude", "sonnet"), &claude),
            "sonnet"
        );
        assert_eq!(
            brief_model(Some("haiku"), &handoff("claude", "sonnet"), &claude),
            "haiku"
        );
        assert_eq!(
            brief_model(None, &handoff("claude-headless", "sonnet"), &claude),
            "sonnet"
        );
        assert_eq!(brief_model(None, &handoff("claude", "  "), &claude), "opus");
        assert_eq!(
            brief_model(None, &handoff("codex", "gpt-6"), &claude),
            "opus"
        );
    }

    #[tokio::test]
    async fn a_failed_refresh_uses_the_last_stored_brief() {
        let fixture = DerivedHandoffFixture::new().await;
        fixture.user(
            1,
            &format!("remember amber-kestrel {}", "notes ".repeat(500)),
        );
        fixture.output(2, "old", "codex", Some("old-thread"), "OK");
        let target = fixture.run("new", "claude", None);
        let handoff = fixture
            .runtime
            .owned
            .context_handoff_for_dispatch(
                &fixture.session_id,
                &fixture.agent_id,
                &target,
                "prompt-next",
                false,
            )
            .unwrap();
        // A successful fold is durable even when the next utility call fails.
        let brief = "## Goal\nShip amber-kestrel.\n## Next Steps\nTest.";
        fixture
            .history
            .save_agent_handoff_brief(
                &fixture.session_id,
                &fixture.agent_id,
                &crate::history::AgentHandoffBrief {
                    brief: brief.into(),
                    covered_through_sequence: 1,
                },
            )
            .unwrap();
        // No target runtime: the utility fails, as a later chunk can in production.
        let handoff = fixture
            .runtime
            .with_current_handoff_brief(
                &fixture.runtime.owned,
                handoff,
                &fixture.session_id,
                &fixture.agent_id,
                "prompt-next",
                &target,
                || true,
            )
            .await;
        assert_eq!(handoff.conversation.brief.as_deref(), Some(brief));
    }

    #[test]
    fn handoff_injection_is_source_agnostic() {
        let store = PendingAgentContextHandoffStore::default();
        store.set(
            "session",
            "agent",
            PendingAgentContextHandoff {
                source_provider: "opencode".to_string(),
                source_model: "model-old".to_string(),
                target_provider_run_id: None,
                target_provider: "codex".to_string(),
                target_account_profile: "default".to_string(),
                target_model: None,
                target_1m_context_disabled: false,
                conversation: conversation("workflow context"),
                derived: false,
            },
        );

        let target_run = test_run("run-new", "session", "agent", "codex", "model-new");
        let handoff = store.consume_matching("session", "agent", &target_run);

        assert!(handoff.is_some());
        let injected = inject_context_handoff("run workflow node", &handoff.unwrap());
        assert!(injected.contains("workflow context"));
        assert!(store
            .consume_matching("session", "agent", &target_run)
            .is_none());
    }

    #[test]
    fn handoff_hidden_context_does_not_duplicate_active_user_request() {
        let handoff = PendingAgentContextHandoff {
            source_provider: "codex".to_string(),
            source_model: "gpt-5.5".to_string(),
            target_provider_run_id: None,
            target_provider: "claude-headless".to_string(),
            target_account_profile: "default".to_string(),
            target_model: Some("claude-opus-4-7".to_string()),
            target_1m_context_disabled: false,
            conversation: conversation("prior context"),
            derived: false,
        };
        let hidden = handoff.render_hidden(MAX_HANDOFF_BYTES);

        assert!(hidden.contains("<chariox_context_handoff>"));
        assert!(hidden.contains("- User: prior context"));
        assert!(hidden.contains(
            "Provider/account switch: codex (gpt-5.5) -> claude-headless [default] (claude-opus-4-7)."
        ));
        assert!(hidden.contains("The active user request is supplied separately."));
        assert!(!hidden.contains("<user_request>"));
    }

    #[test]
    fn handoff_ignores_mismatched_target_provider() {
        let store = PendingAgentContextHandoffStore::default();
        store.set(
            "session",
            "agent",
            PendingAgentContextHandoff {
                source_provider: "claude".to_string(),
                source_model: "opus".to_string(),
                target_provider_run_id: None,
                target_provider: "codex".to_string(),
                target_account_profile: "default".to_string(),
                target_model: Some("gpt-5".to_string()),
                target_1m_context_disabled: false,
                conversation: conversation("prior context"),
                derived: false,
            },
        );

        let wrong_provider = test_run("run-new", "session", "agent", "opencode", "gpt-5");
        let right_provider = test_run("run-new", "session", "agent", "codex", "gpt-5");

        assert!(store
            .consume_matching("session", "agent", &wrong_provider)
            .is_none());
        assert!(store
            .consume_matching("session", "agent", &right_provider)
            .is_some());
    }

    fn conversation(prompt: &str) -> AgentConversation {
        AgentConversation::from_events(&[crate::history::HistoryEvent::transcript(
            1,
            &crate::history::SessionHistoryEntry::user_prompt(
                "session",
                "attachment",
                "agent",
                prompt,
            ),
            crate::history::HistoryEventTurnContext::default(),
        )])
    }

    fn test_run(
        run_id: &str,
        session_id: &str,
        agent_id: &str,
        provider: &str,
        model: &str,
    ) -> RuntimeProviderRun {
        test_run_with_account(run_id, session_id, agent_id, provider, model, "default")
    }

    fn test_run_with_account(
        run_id: &str,
        session_id: &str,
        agent_id: &str,
        provider: &str,
        model: &str,
        account_profile: &str,
    ) -> RuntimeProviderRun {
        let request = crate::provider::LaunchProviderRequest::new(
            session_id,
            "dev-stub",
            provider,
            account_profile,
            model,
        )
        .with_agent_id(agent_id);
        test_run_from_request(run_id, request)
    }

    fn test_run_from_request(
        run_id: &str,
        request: crate::provider::LaunchProviderRequest,
    ) -> RuntimeProviderRun {
        let provider = request.provider.clone();
        RuntimeProviderRun::new(
            run_id,
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: provider.to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        )
    }
}
