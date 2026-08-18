use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

use crate::local::{
    ExternalProviderSessionPage, ExternalProviderSessionRecord, ListExternalProviderSessionsRequest,
};
use crate::provider::{
    canonical_profile_external_provider_session_id, external_provider_session_providers,
    ExternalProviderImportMetadata, ExternalProviderObservedCursor, ProviderResumeState,
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
    pub(crate) account_profile: String,
    pub(crate) provider_session_id: String,
}

impl AttachedProviderTranscriptCursorKey {
    pub(crate) fn new(
        session_id: impl Into<String>,
        agent_id: impl Into<String>,
        provider: impl Into<String>,
        account_profile: impl Into<String>,
        provider_session_id: impl Into<String>,
    ) -> Self {
        let provider = provider.into();
        let provider_session_id = provider_session_id.into();
        let provider = normalize_external_provider_filter(&provider).unwrap_or(provider);
        let provider_session_id = provider_session_id.trim().to_string();
        Self {
            session_id: session_id.into(),
            agent_id: agent_id.into(),
            provider,
            account_profile: account_profile.into().trim().to_string(),
            provider_session_id,
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
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key, cursor);
    }

    pub(crate) fn detach_session(&self, session_id: &str) -> usize {
        let mut cursors = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous_len = cursors.len();
        cursors.retain(|key, _| key.session_id != session_id);
        previous_len.saturating_sub(cursors.len())
    }

    pub(crate) fn detach_agent(&self, session_id: &str, agent_id: &str) -> usize {
        let mut cursors = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ExternalProviderSessionAttachmentRef {
    pub(crate) external_session_id: String,
    pub(crate) session_id: String,
    pub(crate) agent_id: String,
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
    pub(crate) fn replace_provider_sessions(
        &self,
        provider: &str,
        sessions: Vec<ExternalProviderSessionRecord>,
    ) {
        let Some(provider) = normalize_external_provider_filter(provider) else {
            return;
        };
        let mut index = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut replacement = BTreeMap::new();
        for mut session in sessions {
            if normalize_external_provider_filter(&session.provider).as_deref()
                != Some(provider.as_str())
            {
                continue;
            }
            normalize_external_provider_session_record(&mut session);
            if let Some(attachment) = index.attached.get(&session.external_session_id) {
                apply_attachment_marker(&mut session, attachment);
            }
            replacement.insert(session.external_session_id.clone(), session);
        }
        let current = index
            .sessions
            .iter()
            .filter(|(_, session)| {
                normalize_external_provider_filter(&session.provider).as_deref()
                    == Some(provider.as_str())
            })
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

    pub(crate) fn upsert(&self, session: ExternalProviderSessionRecord) {
        let mut index = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut session = session;
        normalize_external_provider_session_record(&mut session);
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
        account_profile: &str,
        provider_session_id: &str,
        session_id: &str,
        agent_id: &str,
    ) -> Option<ExternalProviderSessionRecord> {
        let external_session_id = canonical_profile_external_provider_session_id(
            provider,
            account_profile,
            provider_session_id,
        )?;
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
        account_profile: &str,
        session_id: &str,
        agent_id: &str,
    ) -> usize {
        let mut count = 0usize;
        for (provider, provider_session_id) in resume_state.external_provider_sessions() {
            self.mark_provider_session_attached(
                provider,
                account_profile,
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
        account_profile: &str,
        provider_session_id: Option<&str>,
        resume_state: &ProviderResumeState,
        session_id: &str,
        agent_id: &str,
    ) {
        self.mark_resume_state_attached(resume_state, account_profile, session_id, agent_id);
        if let Some(provider_session_id) = provider_session_id {
            self.mark_provider_session_attached(
                provider,
                account_profile,
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
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let external_session_id = normalize_external_session_id_key(external_session_id);
        let attachment = {
            let attachment = index
                .attached
                .entry(external_session_id.clone())
                .or_default();
            attachment.insert(session_id, agent_id);
            attachment.clone()
        };
        if let Some(session) = index.sessions.get_mut(&external_session_id) {
            apply_attachment_marker(session, &attachment);
            return Some(session.clone());
        }
        None
    }

    pub(crate) fn detach_session(&self, session_id: &str) {
        let mut index = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    pub(crate) fn detach_attachment(
        &self,
        external_session_id: &str,
        session_id: &str,
        agent_id: &str,
    ) -> bool {
        let mut index = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let external_session_id = normalize_external_session_id_key(external_session_id);
        let Some(attachment) = index.attached.get_mut(&external_session_id) else {
            return false;
        };
        if !attachment.remove_agent(session_id, agent_id) {
            return false;
        }
        let attachment = if attachment.is_empty() {
            index.attached.remove(&external_session_id);
            None
        } else {
            index.attached.get(&external_session_id).cloned()
        };
        if let Some(session) = index.sessions.get_mut(&external_session_id) {
            if let Some(attachment) = attachment.as_ref() {
                apply_attachment_marker(session, attachment);
            } else {
                clear_attachment_marker(session);
            }
        }
        true
    }

    pub(crate) fn attachment_refs(&self) -> BTreeSet<ExternalProviderSessionAttachmentRef> {
        let index = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        index
            .attached
            .iter()
            .flat_map(|(external_session_id, attachment)| {
                attachment.agent_ids_by_session_id.iter().flat_map(
                    move |(session_id, agent_ids)| {
                        agent_ids
                            .iter()
                            .map(move |agent_id| ExternalProviderSessionAttachmentRef {
                                external_session_id: external_session_id.clone(),
                                session_id: session_id.clone(),
                                agent_id: agent_id.clone(),
                            })
                    },
                )
            })
            .collect()
    }

    pub(crate) fn get(&self, external_session_id: &str) -> Option<ExternalProviderSessionRecord> {
        let external_session_id = normalize_external_session_id_key(external_session_id);
        self.inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .sessions
            .get(&external_session_id)
            .cloned()
    }

    #[cfg(test)]
    pub(crate) fn list(
        &self,
        request: &ListExternalProviderSessionsRequest,
    ) -> ExternalProviderSessionPage {
        self.list_scoped(None, request)
    }

    pub(crate) fn list_for_owner(
        &self,
        owner_user_id: &str,
        request: &ListExternalProviderSessionsRequest,
    ) -> ExternalProviderSessionPage {
        self.list_scoped(Some(owner_user_id), request)
    }

    fn list_scoped(
        &self,
        owner_user_id: Option<&str>,
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
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut sessions = index
            .sessions
            .values()
            .filter(|session| {
                if owner_user_id.is_some_and(|owner| session.owner_user_id != owner) {
                    return false;
                }
                if !session.is_attachable_to_chariox() {
                    return false;
                }
                request
                    .provider
                    .as_deref()
                    .and_then(normalize_external_provider_filter)
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

fn apply_attachment_marker(
    session: &mut ExternalProviderSessionRecord,
    attachment: &ExternalProviderSessionAttachment,
) {
    session.mark_attached_to_chariox(attachment.session_ids(), attachment.agent_ids());
}

fn clear_attachment_marker(session: &mut ExternalProviderSessionRecord) {
    session.clear_chariox_attachment();
}

fn parse_external_provider_session_cursor(cursor: &str) -> Option<usize> {
    cursor.strip_prefix("offset:")?.parse::<usize>().ok()
}

fn format_external_provider_session_cursor(offset: usize) -> String {
    format!("offset:{offset}")
}

fn normalize_external_provider_filter(provider: &str) -> Option<String> {
    let provider = provider.trim().to_ascii_lowercase();
    external_provider_session_providers()
        .contains(&provider.as_str())
        .then_some(provider)
}

fn normalize_external_session_id_key(external_session_id: &str) -> String {
    let external_session_id = external_session_id.trim();
    let mut parts = external_session_id.splitn(3, ':');
    let Some(provider) = parts.next() else {
        return external_session_id.to_string();
    };
    let Some(profile_or_session) = parts.next() else {
        return external_session_id.to_string();
    };
    let (account_profile, provider_session_id) = match parts.next() {
        Some(provider_session_id) => (profile_or_session, provider_session_id),
        None => ("default", profile_or_session),
    };
    canonical_profile_external_provider_session_id(provider, account_profile, provider_session_id)
        .unwrap_or_else(|| external_session_id.to_string())
}

fn normalize_external_provider_session_record(session: &mut ExternalProviderSessionRecord) {
    if let Some(provider) = normalize_external_provider_filter(&session.provider) {
        session.provider = provider.clone();
        session.provider_session_id = session.provider_session_id.trim().to_string();
        if let Some(external_session_id) = canonical_profile_external_provider_session_id(
            &provider,
            &session.account_profile,
            &session.provider_session_id,
        ) {
            session.external_session_id = external_session_id;
        } else {
            session.external_session_id =
                normalize_external_session_id_key(&session.external_session_id);
        }
        return;
    }
    session.external_session_id = session.external_session_id.trim().to_string();
}

#[cfg(test)]
mod tests;
