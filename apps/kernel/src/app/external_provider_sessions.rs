use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

use crate::local::{
    ExternalProviderSessionPage, ExternalProviderSessionRecord, ListExternalProviderSessionsRequest,
};
use crate::provider::{
    canonical_external_provider_session_id, ExternalProviderImportMetadata,
    ExternalProviderObservedCursor, ProviderResumeState,
};
use crate::session::unix_epoch_ms;

const DEFAULT_EXTERNAL_PROVIDER_SESSION_LIMIT: usize = 25;
const MAX_EXTERNAL_PROVIDER_SESSION_LIMIT: usize = 100;

#[derive(Debug, Clone, Default)]
pub(crate) struct ExternalProviderSessionIndexStore {
    inner: Arc<RwLock<ExternalProviderSessionIndex>>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct AttachedProviderTranscriptCursorKey {
    pub(crate) session_id: String,
    pub(crate) agent_id: String,
    pub(crate) provider: String,
    pub(crate) provider_session_id: String,
}

impl AttachedProviderTranscriptCursorKey {
    pub(crate) fn new(
        session_id: impl Into<String>,
        agent_id: impl Into<String>,
        provider: impl Into<String>,
        provider_session_id: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            agent_id: agent_id.into(),
            provider: provider.into(),
            provider_session_id: provider_session_id.into(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct AttachedProviderTranscriptCursorStore {
    inner:
        Arc<RwLock<BTreeMap<AttachedProviderTranscriptCursorKey, ExternalProviderObservedCursor>>>,
}

impl AttachedProviderTranscriptCursorStore {
    pub(crate) fn get(
        &self,
        key: &AttachedProviderTranscriptCursorKey,
    ) -> ExternalProviderObservedCursor {
        self.inner
            .read()
            .expect("attached provider transcript cursor store poisoned")
            .get(key)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn set(
        &self,
        key: AttachedProviderTranscriptCursorKey,
        cursor: ExternalProviderObservedCursor,
    ) {
        self.inner
            .write()
            .expect("attached provider transcript cursor store poisoned")
            .insert(key, cursor);
    }

    pub(crate) fn detach_session(&self, session_id: &str) -> usize {
        let mut cursors = self
            .inner
            .write()
            .expect("attached provider transcript cursor store poisoned");
        let previous_len = cursors.len();
        cursors.retain(|key, _| key.session_id != session_id);
        previous_len.saturating_sub(cursors.len())
    }

    pub(crate) fn detach_agent(&self, session_id: &str, agent_id: &str) -> usize {
        let mut cursors = self
            .inner
            .write()
            .expect("attached provider transcript cursor store poisoned");
        let previous_len = cursors.len();
        cursors.retain(|key, _| key.session_id != session_id || key.agent_id != agent_id);
        previous_len.saturating_sub(cursors.len())
    }
}

#[derive(Debug, Clone, Default)]
struct ExternalProviderSessionIndex {
    sessions: BTreeMap<String, ExternalProviderSessionRecord>,
    attached: BTreeMap<String, ExternalProviderSessionAttachment>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ExternalProviderSessionAttachment {
    agent_ids_by_session_id: BTreeMap<String, BTreeSet<String>>,
}

impl ExternalProviderSessionAttachment {
    fn insert(&mut self, session_id: &str, agent_id: &str) {
        self.agent_ids_by_session_id
            .entry(session_id.to_string())
            .or_default()
            .insert(agent_id.to_string());
    }

    fn remove_session(&mut self, session_id: &str) -> bool {
        self.agent_ids_by_session_id.remove(session_id).is_some()
    }

    fn remove_agent(&mut self, session_id: &str, agent_id: &str) -> bool {
        let Some(agent_ids) = self.agent_ids_by_session_id.get_mut(session_id) else {
            return false;
        };
        let removed = agent_ids.remove(agent_id);
        if agent_ids.is_empty() {
            self.agent_ids_by_session_id.remove(session_id);
        }
        removed
    }

    fn is_empty(&self) -> bool {
        self.agent_ids_by_session_id.is_empty()
    }

    fn session_ids(&self) -> Vec<String> {
        self.agent_ids_by_session_id.keys().cloned().collect()
    }

    fn agent_ids(&self) -> Vec<String> {
        self.agent_ids_by_session_id
            .values()
            .flat_map(|agent_ids| agent_ids.iter().cloned())
            .collect()
    }
}

impl ExternalProviderSessionIndexStore {
    #[allow(dead_code)]
    pub(crate) fn replace_provider_sessions(
        &self,
        provider: &str,
        sessions: Vec<ExternalProviderSessionRecord>,
    ) {
        let mut index = self
            .inner
            .write()
            .expect("external provider session index poisoned");
        let mut replacement = BTreeMap::new();
        for mut session in sessions {
            if let Some(attachment) = index.attached.get(&session.external_session_id) {
                apply_attachment_marker(&mut session, attachment);
            }
            replacement.insert(session.external_session_id.clone(), session);
        }
        let current = index
            .sessions
            .iter()
            .filter(|(_, session)| session.provider == provider)
            .map(|(id, session)| (id.clone(), session.clone()))
            .collect::<BTreeMap<_, _>>();
        if current == replacement {
            return;
        }
        index
            .sessions
            .retain(|_, session| session.provider != provider);
        index.sessions.extend(replacement);
    }

    #[allow(dead_code)]
    pub(crate) fn upsert(&self, session: ExternalProviderSessionRecord) {
        let mut index = self
            .inner
            .write()
            .expect("external provider session index poisoned");
        let mut session = session;
        if let Some(attachment) = index.attached.get(&session.external_session_id) {
            apply_attachment_marker(&mut session, attachment);
        }
        index
            .sessions
            .insert(session.external_session_id.clone(), session);
    }

    pub(crate) fn mark_provider_session_attached(
        &self,
        provider: &str,
        provider_session_id: &str,
        session_id: &str,
        agent_id: &str,
    ) -> Option<ExternalProviderSessionRecord> {
        let external_session_id =
            external_session_id_for_provider_session(provider, provider_session_id)?;
        self.mark_attached(&external_session_id, session_id, agent_id)
    }

    pub(crate) fn mark_import_attached(
        &self,
        import: &ExternalProviderImportMetadata,
        session_id: &str,
        agent_id: &str,
    ) -> Option<ExternalProviderSessionRecord> {
        self.mark_attached(&import.external_provider_session_id, session_id, agent_id)
    }

    pub(crate) fn mark_resume_state_attached(
        &self,
        resume_state: &ProviderResumeState,
        session_id: &str,
        agent_id: &str,
    ) -> usize {
        let mut count = 0usize;
        for (provider, provider_session_id) in resume_state.external_provider_sessions() {
            self.mark_provider_session_attached(
                provider,
                provider_session_id,
                session_id,
                agent_id,
            );
            count += 1;
        }
        count
    }

    pub(crate) fn mark_provider_run_attached(
        &self,
        provider: &str,
        provider_session_id: Option<&str>,
        resume_state: &ProviderResumeState,
        session_id: &str,
        agent_id: &str,
    ) {
        self.mark_resume_state_attached(resume_state, session_id, agent_id);
        if let Some(provider_session_id) = provider_session_id {
            self.mark_provider_session_attached(
                provider,
                provider_session_id,
                session_id,
                agent_id,
            );
        }
    }

    pub(crate) fn mark_attached(
        &self,
        external_session_id: &str,
        session_id: &str,
        agent_id: &str,
    ) -> Option<ExternalProviderSessionRecord> {
        let mut index = self
            .inner
            .write()
            .expect("external provider session index poisoned");
        let attachment = {
            let attachment = index
                .attached
                .entry(external_session_id.to_string())
                .or_default();
            attachment.insert(session_id, agent_id);
            attachment.clone()
        };
        if let Some(session) = index.sessions.get_mut(external_session_id) {
            apply_attachment_marker(session, &attachment);
            return Some(session.clone());
        }
        None
    }

    pub(crate) fn detach_session(&self, session_id: &str) {
        let mut index = self
            .inner
            .write()
            .expect("external provider session index poisoned");
        let mut changed_external_session_ids = Vec::new();
        index.attached.retain(|external_session_id, attachment| {
            if attachment.remove_session(session_id) {
                changed_external_session_ids.push(external_session_id.clone());
            }
            !attachment.is_empty()
        });
        for external_session_id in changed_external_session_ids {
            let attachment = index.attached.get(&external_session_id).cloned();
            if let Some(session) = index.sessions.get_mut(&external_session_id) {
                if let Some(attachment) = attachment.as_ref() {
                    apply_attachment_marker(session, attachment);
                } else {
                    clear_attachment_marker(session);
                }
            }
        }
    }

    pub(crate) fn detach_agent(&self, session_id: &str, agent_id: &str) {
        let mut index = self
            .inner
            .write()
            .expect("external provider session index poisoned");
        let mut changed_external_session_ids = Vec::new();
        index.attached.retain(|external_session_id, attachment| {
            if attachment.remove_agent(session_id, agent_id) {
                changed_external_session_ids.push(external_session_id.clone());
            }
            !attachment.is_empty()
        });
        for external_session_id in changed_external_session_ids {
            let attachment = index.attached.get(&external_session_id).cloned();
            if let Some(session) = index.sessions.get_mut(&external_session_id) {
                if let Some(attachment) = attachment.as_ref() {
                    apply_attachment_marker(session, attachment);
                } else {
                    clear_attachment_marker(session);
                }
            }
        }
    }

    pub(crate) fn get(&self, external_session_id: &str) -> Option<ExternalProviderSessionRecord> {
        self.inner
            .read()
            .expect("external provider session index poisoned")
            .sessions
            .get(external_session_id)
            .cloned()
    }

    pub(crate) fn list(
        &self,
        request: &ListExternalProviderSessionsRequest,
    ) -> ExternalProviderSessionPage {
        let limit = request
            .limit
            .unwrap_or(DEFAULT_EXTERNAL_PROVIDER_SESSION_LIMIT)
            .clamp(1, MAX_EXTERNAL_PROVIDER_SESSION_LIMIT);
        let offset = request
            .cursor
            .as_deref()
            .and_then(parse_external_provider_session_cursor)
            .unwrap_or(0);
        let index = self
            .inner
            .read()
            .expect("external provider session index poisoned");
        let mut sessions = index
            .sessions
            .values()
            .filter(|session| {
                if !session.is_attachable_to_arroba() {
                    return false;
                }
                request
                    .provider
                    .as_deref()
                    .map_or(true, |provider| session.provider == provider)
            })
            .cloned()
            .collect::<Vec<_>>();
        sessions.sort_by(|left, right| {
            right
                .last_modified_at_ms
                .cmp(&left.last_modified_at_ms)
                .then_with(|| left.provider.cmp(&right.provider))
                .then_with(|| left.provider_session_id.cmp(&right.provider_session_id))
        });
        let page_sessions = sessions
            .iter()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect::<Vec<_>>();
        let next_offset = offset.saturating_add(page_sessions.len());
        let has_more = next_offset < sessions.len();
        ExternalProviderSessionPage {
            sessions: page_sessions,
            next_cursor: has_more.then(|| format_external_provider_session_cursor(next_offset)),
            has_more,
            generated_at_ms: unix_epoch_ms(),
        }
    }
}

pub(crate) fn external_session_id_for_provider_session(
    provider: &str,
    provider_session_id: &str,
) -> Option<String> {
    canonical_external_provider_session_id(provider, provider_session_id)
}

fn apply_attachment_marker(
    session: &mut ExternalProviderSessionRecord,
    attachment: &ExternalProviderSessionAttachment,
) {
    session.mark_attached_to_arroba(attachment.session_ids(), attachment.agent_ids());
}

fn clear_attachment_marker(session: &mut ExternalProviderSessionRecord) {
    session.clear_arroba_attachment();
}

fn parse_external_provider_session_cursor(cursor: &str) -> Option<usize> {
    cursor.strip_prefix("offset:")?.parse::<usize>().ok()
}

fn format_external_provider_session_cursor(offset: usize) -> String {
    format!("offset:{offset}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::{ExternalProviderSessionCapabilities, ExternalProviderSessionRecord};

    #[test]
    fn list_sorts_filters_and_paginates_external_provider_sessions() {
        let store = ExternalProviderSessionIndexStore::default();
        store.upsert(record("codex", "thread-1", 20));
        store.upsert(record("opencode", "session-1", 30));
        store.upsert(record("codex", "thread-2", 10));

        let first = store.list(&ListExternalProviderSessionsRequest {
            provider: None,
            cursor: None,
            limit: Some(2),
        });
        assert_eq!(
            first
                .sessions
                .iter()
                .map(|session| session.external_session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["opencode:session-1", "codex:thread-1"]
        );
        assert!(first.has_more);
        assert_eq!(first.next_cursor.as_deref(), Some("offset:2"));

        let second = store.list(&ListExternalProviderSessionsRequest {
            provider: None,
            cursor: first.next_cursor,
            limit: Some(2),
        });
        assert_eq!(second.sessions[0].external_session_id, "codex:thread-2");
        assert!(!second.has_more);

        let codex = store.list(&ListExternalProviderSessionsRequest {
            provider: Some("codex".to_string()),
            cursor: None,
            limit: None,
        });
        assert_eq!(codex.sessions.len(), 2);
    }

    #[test]
    fn replace_provider_sessions_preserves_attachment_markers() {
        let store = ExternalProviderSessionIndexStore::default();
        store.upsert(record("codex", "thread-1", 20));
        store.mark_attached("codex:thread-1", "session-1", "agent-1");

        store.replace_provider_sessions("codex", vec![record("codex", "thread-1", 40)]);

        let session = store
            .get("codex:thread-1")
            .expect("session should remain indexed");
        assert!(session.is_attached_to_arroba());
        assert_eq!(session.first_attached_session_id(), Some("session-1"));
        assert_eq!(session.first_attached_agent_id(), Some("agent-1"));
        assert_eq!(session.last_modified_at_ms, 40);
    }

    #[test]
    fn attachment_marker_applies_to_later_discovered_provider_session() {
        let store = ExternalProviderSessionIndexStore::default();
        assert!(store
            .mark_provider_session_attached("codex", "thread-1", "session-1", "agent-1")
            .is_none());

        store.replace_provider_sessions("codex", vec![record("codex", "thread-1", 40)]);

        let page = store.list(&ListExternalProviderSessionsRequest {
            provider: Some("codex".to_string()),
            cursor: None,
            limit: None,
        });
        assert!(page.sessions.is_empty());
        let session = store
            .get("codex:thread-1")
            .expect("session should be indexed");
        assert!(session.is_attached_to_arroba());
        assert_eq!(session.first_attached_session_id(), Some("session-1"));
        assert_eq!(session.first_attached_agent_id(), Some("agent-1"));
    }

    #[test]
    fn attachment_marker_can_be_applied_from_external_import_metadata() {
        let store = ExternalProviderSessionIndexStore::default();
        store.upsert(record("codex", "thread-1", 40));
        let import =
            ExternalProviderImportMetadata::observed_history("codex:thread-1", "codex", "thread-1");

        let attached = store
            .mark_import_attached(&import, "session-1", "agent-1")
            .expect("provider session should be indexed");

        assert!(attached.is_attached_to_arroba());
        assert_eq!(attached.first_attached_session_id(), Some("session-1"));
        assert_eq!(attached.first_attached_agent_id(), Some("agent-1"));
    }

    #[test]
    fn attachment_marker_can_be_applied_from_provider_resume_state() {
        let store = ExternalProviderSessionIndexStore::default();
        store.upsert(record("codex", "thread-1", 40));
        store.upsert(record("claude", "thread-2", 30));
        store.upsert(record("opencode", "thread-3", 20));
        let mut resume_state = ProviderResumeState::from_codex_thread_id("thread-1");
        resume_state.set_claude_session_id("thread-2");
        resume_state.set_opencode_session_id("thread-3");

        let attached_count =
            store.mark_resume_state_attached(&resume_state, "session-1", "agent-1");

        assert_eq!(attached_count, 3);
        for external_session_id in ["codex:thread-1", "claude:thread-2", "opencode:thread-3"] {
            let session = store
                .get(external_session_id)
                .expect("provider session should remain indexed");
            assert!(session.is_attached_to_arroba());
            assert_eq!(session.first_attached_session_id(), Some("session-1"));
            assert_eq!(session.first_attached_agent_id(), Some("agent-1"));
        }
        assert!(
            store
                .list(&ListExternalProviderSessionsRequest {
                    provider: None,
                    cursor: None,
                    limit: None,
                })
                .sessions
                .is_empty(),
            "resume-state attached provider sessions should not be attachable"
        );
    }

    #[test]
    fn provider_run_attachment_marks_resume_state_and_direct_provider_session() {
        let store = ExternalProviderSessionIndexStore::default();
        store.upsert(record("codex", "thread-from-resume", 40));
        store.upsert(record("opencode", "session-from-run", 30));
        let resume_state = ProviderResumeState::from_codex_thread_id("thread-from-resume");

        store.mark_provider_run_attached(
            "opencode",
            Some("session-from-run"),
            &resume_state,
            "session-1",
            "agent-1",
        );

        for external_session_id in ["codex:thread-from-resume", "opencode:session-from-run"] {
            let session = store
                .get(external_session_id)
                .expect("provider session should remain indexed");
            assert!(session.is_attached_to_arroba());
            assert_eq!(session.first_attached_session_id(), Some("session-1"));
            assert_eq!(session.first_attached_agent_id(), Some("agent-1"));
        }
    }

    #[test]
    fn external_session_id_for_provider_session_canonicalizes_known_providers() {
        assert_eq!(
            external_session_id_for_provider_session(" Codex ", " thread-1 ").as_deref(),
            Some("codex:thread-1")
        );
        assert_eq!(
            external_session_id_for_provider_session("unknown", "thread-1"),
            None
        );
        assert_eq!(
            external_session_id_for_provider_session("codex", "   "),
            None
        );
    }

    #[test]
    fn list_excludes_attached_to_arroba_external_provider_sessions() {
        let store = ExternalProviderSessionIndexStore::default();
        store.upsert(record("codex", "thread-1", 30));
        store.upsert(record("codex", "thread-2", 20));
        store.mark_attached("codex:thread-1", "session-1", "agent-1");

        let page = store.list(&ListExternalProviderSessionsRequest {
            provider: Some("codex".to_string()),
            cursor: None,
            limit: None,
        });

        assert_eq!(
            page.sessions
                .iter()
                .map(|session| session.external_session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["codex:thread-2"]
        );
    }

    #[test]
    fn detach_session_returns_provider_session_to_attachable_list() {
        let store = ExternalProviderSessionIndexStore::default();
        store.upsert(record("codex", "thread-1", 30));
        store.mark_attached("codex:thread-1", "session-1", "agent-1");

        assert!(store
            .list(&ListExternalProviderSessionsRequest {
                provider: Some("codex".to_string()),
                cursor: None,
                limit: None,
            })
            .sessions
            .is_empty());

        store.detach_session("session-1");

        let page = store.list(&ListExternalProviderSessionsRequest {
            provider: Some("codex".to_string()),
            cursor: None,
            limit: None,
        });
        assert_eq!(page.sessions.len(), 1);
        assert_eq!(page.sessions[0].external_session_id, "codex:thread-1");
        assert!(page.sessions[0].is_attachable_to_arroba());
        assert_eq!(page.sessions[0].first_attached_session_id(), None);
        assert_eq!(page.sessions[0].first_attached_agent_id(), None);
    }

    #[test]
    fn detach_session_preserves_other_session_attachment_agents() {
        let store = ExternalProviderSessionIndexStore::default();
        store.upsert(record("codex", "thread-1", 30));
        store.mark_attached("codex:thread-1", "session-1", "agent-1");
        store.mark_attached("codex:thread-1", "session-2", "agent-2");

        store.detach_session("session-1");

        let session = store
            .get("codex:thread-1")
            .expect("session should remain indexed");
        assert!(session.is_attached_to_arroba());
        assert_eq!(session.attached_session_ids, vec!["session-2"]);
        assert_eq!(session.attached_agent_ids, vec!["agent-2"]);
        assert!(store
            .list(&ListExternalProviderSessionsRequest {
                provider: Some("codex".to_string()),
                cursor: None,
                limit: None,
            })
            .sessions
            .is_empty());
    }

    #[test]
    fn detach_agent_returns_provider_session_to_attachable_list() {
        let store = ExternalProviderSessionIndexStore::default();
        store.upsert(record("codex", "thread-1", 30));
        store.mark_attached("codex:thread-1", "session-1", "agent-1");

        store.detach_agent("session-1", "agent-1");

        let page = store.list(&ListExternalProviderSessionsRequest {
            provider: Some("codex".to_string()),
            cursor: None,
            limit: None,
        });
        assert_eq!(page.sessions.len(), 1);
        assert_eq!(page.sessions[0].external_session_id, "codex:thread-1");
        assert!(page.sessions[0].is_attachable_to_arroba());
        assert_eq!(page.sessions[0].first_attached_session_id(), None);
        assert_eq!(page.sessions[0].first_attached_agent_id(), None);
    }

    #[test]
    fn detach_agent_preserves_other_agent_attachments() {
        let store = ExternalProviderSessionIndexStore::default();
        store.upsert(record("codex", "thread-1", 30));
        store.mark_attached("codex:thread-1", "session-1", "agent-1");
        store.mark_attached("codex:thread-1", "session-1", "agent-2");
        store.mark_attached("codex:thread-1", "session-2", "agent-3");

        store.detach_agent("session-1", "agent-1");

        let session = store
            .get("codex:thread-1")
            .expect("session should remain indexed");
        assert!(session.is_attached_to_arroba());
        assert_eq!(session.attached_session_ids, vec!["session-1", "session-2"]);
        assert_eq!(session.attached_agent_ids, vec!["agent-2", "agent-3"]);
        assert!(store
            .list(&ListExternalProviderSessionsRequest {
                provider: Some("codex".to_string()),
                cursor: None,
                limit: None,
            })
            .sessions
            .is_empty());

        store.detach_agent("session-1", "agent-2");
        let session = store
            .get("codex:thread-1")
            .expect("session should remain indexed");
        assert!(session.is_attached_to_arroba());
        assert_eq!(session.attached_session_ids, vec!["session-2"]);
        assert_eq!(session.attached_agent_ids, vec!["agent-3"]);
    }

    #[test]
    fn transcript_cursor_store_detaches_session_cursors() {
        let store = AttachedProviderTranscriptCursorStore::default();
        store.set(
            AttachedProviderTranscriptCursorKey::new("session-1", "agent-1", "codex", "thread-1"),
            ExternalProviderObservedCursor {
                last_observed_turn_id: Some("turn-1".to_string()),
                ..ExternalProviderObservedCursor::default()
            },
        );
        let preserved_key =
            AttachedProviderTranscriptCursorKey::new("session-2", "agent-2", "codex", "thread-2");
        store.set(
            preserved_key.clone(),
            ExternalProviderObservedCursor {
                last_observed_turn_id: Some("turn-2".to_string()),
                ..ExternalProviderObservedCursor::default()
            },
        );

        assert_eq!(store.detach_session("session-1"), 1);

        assert_eq!(
            store.get(&AttachedProviderTranscriptCursorKey::new(
                "session-1",
                "agent-1",
                "codex",
                "thread-1"
            )),
            ExternalProviderObservedCursor::default()
        );
        assert_eq!(
            store.get(&preserved_key).last_observed_turn_id.as_deref(),
            Some("turn-2")
        );
    }

    #[test]
    fn transcript_cursor_store_detaches_agent_cursors() {
        let store = AttachedProviderTranscriptCursorStore::default();
        store.set(
            AttachedProviderTranscriptCursorKey::new("session-1", "agent-1", "codex", "thread-1"),
            ExternalProviderObservedCursor {
                last_observed_turn_id: Some("turn-1".to_string()),
                ..ExternalProviderObservedCursor::default()
            },
        );
        let preserved_same_session =
            AttachedProviderTranscriptCursorKey::new("session-1", "agent-2", "codex", "thread-2");
        store.set(
            preserved_same_session.clone(),
            ExternalProviderObservedCursor {
                last_observed_turn_id: Some("turn-2".to_string()),
                ..ExternalProviderObservedCursor::default()
            },
        );

        assert_eq!(store.detach_agent("session-1", "agent-1"), 1);

        assert_eq!(
            store.get(&AttachedProviderTranscriptCursorKey::new(
                "session-1",
                "agent-1",
                "codex",
                "thread-1"
            )),
            ExternalProviderObservedCursor::default()
        );
        assert_eq!(
            store
                .get(&preserved_same_session)
                .last_observed_turn_id
                .as_deref(),
            Some("turn-2")
        );
    }

    fn record(
        provider: &str,
        provider_session_id: &str,
        last_modified_at_ms: u64,
    ) -> ExternalProviderSessionRecord {
        ExternalProviderSessionRecord {
            external_session_id: format!("{provider}:{provider_session_id}"),
            provider: provider.to_string(),
            provider_session_id: provider_session_id.to_string(),
            title: Some(provider_session_id.to_string()),
            title_source: Some("test".to_string()),
            first_prompt_preview: None,
            created_at_ms: None,
            last_modified_at_ms,
            worktree_path: None,
            account_profile: None,
            capabilities: ExternalProviderSessionCapabilities {
                ..ExternalProviderSessionCapabilities::default()
            },
            attached_to_arroba: false,
            attached_session_ids: Vec::new(),
            attached_agent_ids: Vec::new(),
        }
    }
}
