use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::local::{
    ExternalProviderSessionPage, ExternalProviderSessionRecord, ListExternalProviderSessionsRequest,
};
use crate::session::unix_epoch_ms;

const DEFAULT_EXTERNAL_PROVIDER_SESSION_LIMIT: usize = 25;
const MAX_EXTERNAL_PROVIDER_SESSION_LIMIT: usize = 100;

#[derive(Debug, Clone, Default)]
pub(crate) struct ExternalProviderSessionIndexStore {
    inner: Arc<RwLock<ExternalProviderSessionIndex>>,
}

#[derive(Debug, Clone, Default)]
struct ExternalProviderSessionIndex {
    sessions: BTreeMap<String, ExternalProviderSessionRecord>,
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
            if let Some(existing) = index.sessions.get(&session.external_session_id) {
                session.already_imported = existing.already_imported;
                session.imported_session_ids = existing.imported_session_ids.clone();
                session.imported_agent_ids = existing.imported_agent_ids.clone();
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
        self.inner
            .write()
            .expect("external provider session index poisoned")
            .sessions
            .insert(session.external_session_id.clone(), session);
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

    #[allow(dead_code)]
    pub(crate) fn mark_imported(
        &self,
        external_session_id: &str,
        session_id: &str,
        agent_id: &str,
    ) -> Option<ExternalProviderSessionRecord> {
        let mut index = self
            .inner
            .write()
            .expect("external provider session index poisoned");
        let session = index.sessions.get_mut(external_session_id)?;
        session.already_imported = true;
        if !session
            .imported_session_ids
            .iter()
            .any(|existing| existing == session_id)
        {
            session.imported_session_ids.push(session_id.to_string());
        }
        if !session
            .imported_agent_ids
            .iter()
            .any(|existing| existing == agent_id)
        {
            session.imported_agent_ids.push(agent_id.to_string());
        }
        Some(session.clone())
    }
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
    use crate::local::{
        ExternalProviderSessionCapabilities, ExternalProviderSessionMode,
        ExternalProviderSessionRecord,
    };

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
    fn replace_provider_sessions_preserves_import_markers() {
        let store = ExternalProviderSessionIndexStore::default();
        store.upsert(record("codex", "thread-1", 20));
        store.mark_imported("codex:thread-1", "session-1", "agent-1");

        store.replace_provider_sessions("codex", vec![record("codex", "thread-1", 40)]);

        let session = store
            .get("codex:thread-1")
            .expect("session should remain indexed");
        assert!(session.already_imported);
        assert_eq!(session.imported_session_ids, vec!["session-1"]);
        assert_eq!(session.imported_agent_ids, vec!["agent-1"]);
        assert_eq!(session.last_modified_at_ms, 40);
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
            running_state: None,
            capabilities: ExternalProviderSessionCapabilities {
                can_resume: true,
                ..ExternalProviderSessionCapabilities::default()
            },
            mode: ExternalProviderSessionMode::ResumeOnly,
            already_imported: false,
            imported_session_ids: Vec::new(),
            imported_agent_ids: Vec::new(),
        }
    }
}
