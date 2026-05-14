//! Session transcript history loading and projection refresh.

use crate::error::DaemonError;
use crate::history::{OperationalHistoryStore, SessionHistoryStore};
use crate::local::{GetSessionHistoryRequest, LocalDaemonResponse};
use crate::runtime::projection::{page_history_entries, SessionHistoryProjectionStore};

pub(crate) async fn execute_session_history_request_from_session(
    history: SessionHistoryStore,
    operational_history: OperationalHistoryStore,
    history_projection: SessionHistoryProjectionStore,
    session: crate::session::RuntimeSession,
    request: GetSessionHistoryRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    tokio::task::spawn_blocking(move || {
        let operational_entries =
            operational_history.load_session_history_entries(session.id(), None)?;
        let entries = if operational_entries.is_empty()
            && !operational_history.has_session_events(session.id())?
            && !operational_history.legacy_fallback_disabled(session.id())?
        {
            history.load(&session)?
        } else {
            operational_entries
        };
        history_projection.update_entries(session.id(), entries.clone());
        let page = page_history_entries(
            entries,
            request.agent_id.as_deref(),
            request.round_count,
            request.max_chars,
            request.before_entry_index,
            request.before_entry_char_offset,
        );
        Ok(LocalDaemonResponse::SessionHistory {
            entries: page.entries,
            next_cursor: page.next_cursor,
        })
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "load session history",
        message: error.to_string(),
    })?
}
