//! MP-08 / MP-10 / MP-11: keep waiting-room transport independent of catalog size.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

use crate::error::DaemonError;
use crate::local::{
    ListWaitingRoomSessionsRequest, WaitingRoomPublicSessionSummary, WaitingRoomPublicSnapshot,
    WaitingRoomSessionPage,
};

// Leave room for the encrypted relay envelope (including its base64 expansion).
const MAX_JSON_BYTES: usize = 2 * 1024 * 1024;
const RECENT_SESSIONS: u32 = 100;
const PAGE_SESSIONS: u32 = 500;

pub(crate) fn is_public_code(code: &str) -> bool {
    matches!(
        code,
        "waiting_room_inventory_invalid"
            | "waiting_room_inventory_too_large"
            | "waiting_room_cursor_invalid"
            | "waiting_room_session_too_large"
    )
}

fn fail(code: &str, message: impl Into<String>) -> DaemonError {
    DaemonError::MachineRequest {
        code: code.into(),
        message: message.into(),
    }
}

fn serialized_len(value: &impl serde::Serialize) -> Result<usize, DaemonError> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|error| {
            fail(
                "waiting_room_inventory_invalid",
                format!("Cannot encode waiting-room inventory: {error}"),
            )
        })
}

pub(crate) fn recent_snapshot(
    mut snapshot: WaitingRoomPublicSnapshot,
) -> Result<WaitingRoomPublicSnapshot, DaemonError> {
    let sessions = std::mem::take(&mut snapshot.sessions);
    snapshot.sessions_total = sessions.len();
    let reserved = serialized_len(&snapshot)?.saturating_add(4096);
    let budget = MAX_JSON_BYTES.checked_sub(reserved).ok_or_else(|| {
        fail("waiting_room_inventory_too_large", "Waiting-room metadata exceeds the transport budget. Reduce the kernel's project or device inventory.")
    })?;
    let page = page(sessions, None, RECENT_SESSIONS, budget)?;
    snapshot.sessions = page.sessions;
    snapshot.sessions_next_cursor = page.next_cursor;
    Ok(snapshot)
}

pub(crate) fn session_page(
    sessions: Vec<WaitingRoomPublicSessionSummary>,
    request: ListWaitingRoomSessionsRequest,
) -> Result<WaitingRoomSessionPage, DaemonError> {
    page(
        sessions,
        request.cursor.as_deref(),
        request
            .limit
            .unwrap_or(PAGE_SESSIONS)
            .clamp(1, PAGE_SESSIONS),
        MAX_JSON_BYTES - 4096,
    )
}

fn page(
    mut sessions: Vec<WaitingRoomPublicSessionSummary>,
    cursor: Option<&str>,
    limit: u32,
    budget: usize,
) -> Result<WaitingRoomSessionPage, DaemonError> {
    let after: Option<(u64, String)> = cursor
        .map(|cursor| {
            URL_SAFE_NO_PAD
                .decode(cursor)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .ok_or_else(|| {
                    fail(
                        "waiting_room_cursor_invalid",
                        "The session cursor is invalid. Reopen the session list.",
                    )
                })
        })
        .transpose()?;
    let last_used = |session: &WaitingRoomPublicSessionSummary| {
        session.last_used_at_ms.unwrap_or(session.created_at_ms)
    };
    sessions.sort_by(|a, b| {
        last_used(b)
            .cmp(&last_used(a))
            .then_with(|| a.id.cmp(&b.id))
    });
    let total = sessions.len();
    let mut selected = Vec::new();
    let mut bytes: usize = 0;
    let mut has_more = false;
    for session in sessions {
        if after.as_ref().is_some_and(|(time, id)| {
            last_used(&session) > *time || (last_used(&session) == *time && session.id <= *id)
        }) {
            continue;
        }
        let size = serialized_len(&session)?.saturating_add(1);
        if selected.len() >= limit as usize || bytes.saturating_add(size) > budget {
            if selected.is_empty() {
                return Err(fail("waiting_room_session_too_large", "A session summary exceeds the transport budget. Reduce that session's agents or workflows and reopen the list."));
            }
            has_more = true;
            break;
        }
        bytes += size;
        selected.push(session);
    }
    let next_cursor = if has_more {
        let last = selected.last().expect("nonempty page");
        Some(
            URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(&(last_used(last), &last.id))
                    .map_err(|error| fail("waiting_room_cursor_invalid", error.to_string()))?,
            ),
        )
    } else {
        None
    };
    Ok(WaitingRoomSessionPage {
        sessions: selected,
        total,
        next_cursor,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(index: usize) -> WaitingRoomPublicSessionSummary {
        serde_json::from_value(serde_json::json!({
            "id": format!("session-{index:05}"), "project_id": "project", "workspace_id": "workspace",
            "worktree_id": "worktree", "created_at_ms": 42, "status": "Active", "connected_cli_count": 0,
        })).unwrap()
    }

    #[test]
    fn tied_timestamps_page_every_session_once_and_respect_the_requested_limit() {
        let sessions: Vec<_> = (0..20_000).map(session).collect();
        let mut ids = std::collections::BTreeSet::new();
        let mut cursor = None;
        loop {
            let page = session_page(
                sessions.clone(),
                ListWaitingRoomSessionsRequest {
                    cursor,
                    limit: Some(500),
                },
            )
            .unwrap();
            assert_eq!(page.total, 20_000);
            assert!(serde_json::to_vec(&page).unwrap().len() < MAX_JSON_BYTES);
            for session in page.sessions {
                assert!(ids.insert(session.id));
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(ids.len(), 20_000);
    }

    #[test]
    fn large_summaries_stop_before_the_byte_budget_and_an_oversized_row_is_explicit() {
        let mut summaries: Vec<_> = (0..500).map(session).collect();
        for summary in &mut summaries {
            summary.alias = Some("a".repeat(20_000));
        }
        let page = session_page(
            summaries,
            ListWaitingRoomSessionsRequest {
                cursor: None,
                limit: Some(500),
            },
        )
        .unwrap();
        assert!(page.sessions.len() < 500);
        assert!(page.next_cursor.is_some());
        let json = serde_json::to_vec(&page).unwrap();
        assert!(json.len() < MAX_JSON_BYTES);
        assert!(URL_SAFE_NO_PAD.encode(json).len() + 4096 < 4 * 1024 * 1024);
        let mut oversized = session(0);
        oversized.alias = Some("a".repeat(MAX_JSON_BYTES));
        let error = session_page(
            vec![oversized],
            ListWaitingRoomSessionsRequest {
                cursor: None,
                limit: None,
            },
        )
        .unwrap_err();
        assert!(
            matches!(error, DaemonError::MachineRequest { code, .. } if code == "waiting_room_session_too_large")
        );
    }

    #[test]
    fn invalid_cursors_are_refused_with_a_recovery_instruction() {
        let error = session_page(
            vec![session(0)],
            ListWaitingRoomSessionsRequest {
                cursor: Some("not a cursor".into()),
                limit: None,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("Reopen"));
        assert!(
            matches!(error, DaemonError::MachineRequest { code, .. } if code == "waiting_room_cursor_invalid")
        );
    }
}
