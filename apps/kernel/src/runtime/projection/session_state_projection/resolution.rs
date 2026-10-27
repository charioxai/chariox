//! Session reference resolution against projected sessions.

use crate::error::DaemonError;
use crate::session::{RuntimeSession, SessionStatus};

pub(super) fn resolve_session_ref_id(
    sessions: Vec<RuntimeSession>,
    session_ref: &str,
    workspace_id: Option<&str>,
) -> Option<String> {
    resolve_session_ref_id_from_sessions(sessions, session_ref, workspace_id).ok()
}

pub(super) fn resolve_session_ref_id_from_sessions(
    sessions: Vec<RuntimeSession>,
    session_ref: &str,
    workspace_id: Option<&str>,
) -> Result<String, DaemonError> {
    let normalized_ref = session_ref.trim().to_lowercase();
    if normalized_ref.is_empty() {
        return Err(DaemonError::SessionNotFound {
            session_id: normalized_ref,
        });
    }
    let visible_sessions = sessions
        .iter()
        .filter(|session| session.status() != SessionStatus::Ended)
        .collect::<Vec<_>>();
    let workspace_sessions = visible_sessions
        .iter()
        .copied()
        .filter(|session| workspace_id.is_none_or(|workspace| session.workspace_id() == workspace))
        .collect::<Vec<_>>();

    if let Some(session) = visible_sessions
        .iter()
        .find(|session| session.id() == normalized_ref)
    {
        return Ok(session.id().to_string());
    }
    if let Some(session) = workspace_sessions
        .iter()
        .find(|session| session.alias() == Some(normalized_ref.as_str()))
    {
        return Ok(session.id().to_string());
    }

    let id_matches = visible_sessions
        .iter()
        .filter(|session| session.id().starts_with(&normalized_ref))
        .collect::<Vec<_>>();
    if id_matches.len() == 1 {
        return Ok(id_matches[0].id().to_string());
    }
    if id_matches.len() > 1 {
        return Err(DaemonError::AmbiguousSessionRef {
            session_ref: normalized_ref,
            matches: id_matches
                .into_iter()
                .map(|session| describe_projected_session_match(session))
                .collect(),
        });
    }

    let alias_matches = workspace_sessions
        .iter()
        .filter(|session| {
            session
                .alias()
                .is_some_and(|alias| alias.starts_with(normalized_ref.as_str()))
        })
        .collect::<Vec<_>>();
    if alias_matches.len() == 1 {
        return Ok(alias_matches[0].id().to_string());
    }
    if alias_matches.len() > 1 {
        return Err(DaemonError::AmbiguousSessionRef {
            session_ref: normalized_ref,
            matches: alias_matches
                .into_iter()
                .map(|session| describe_projected_session_match(session))
                .collect(),
        });
    }

    Err(DaemonError::SessionNotFound {
        session_id: normalized_ref,
    })
}

fn describe_projected_session_match(session: &RuntimeSession) -> String {
    match session.alias() {
        Some(alias) => format!("{} ({alias})", session.id()),
        None => session.id().to_string(),
    }
}
