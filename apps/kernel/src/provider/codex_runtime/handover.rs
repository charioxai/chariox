//! Hot reload: a running Codex turn continues in the next kernel program on
//! the same app-server connection. The app-server keeps running the turn, and
//! what it streams during the swap waits in the connection, so every delta
//! arrives once and in order.

use std::collections::BTreeMap;
use std::os::fd::{FromRawFd, IntoRawFd};

use serde::{Deserialize, Serialize};

use crate::provider::codex_client::continue_codex_socket;

use super::transcript::CodexTextTranscriptState;
use super::CodexRuntimeState;

/// What the next kernel program needs to continue a Codex turn; the
/// app-server process itself is handed over as a PTY.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct CodexHandover {
    endpoint: String,
    thread_id: String,
    turn_id: String,
    // The inherited app-server connection, and the bytes of a message the
    // previous program had read only in part.
    connection: i32,
    unread: Vec<u8>,
    next_request_id: u64,
    text_items: BTreeMap<String, CodexTextTranscriptState>,
}

impl CodexHandover {
    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub(crate) fn connection(&self) -> i32 {
        self.connection
    }
}

/// The run's turn with its connection, when one is running on a thread the
/// app-server keeps; otherwise the state back.
pub(crate) fn hand_over_codex_turn(
    mut state: CodexRuntimeState,
) -> Result<CodexHandover, Box<CodexRuntimeState>> {
    let Some(turn_id) = state.active_turn_id.clone() else {
        return Err(Box::new(state));
    };
    // Notifications already read from the connection live only here.
    if !state.thread_ready()
        || state.ephemeral
        || state.notification_only
        || !state.buffered_notifications.is_empty()
    {
        return Err(Box::new(state));
    }
    // A pong the socket owes goes out before the connection changes hands.
    if state.socket.flush().is_err() {
        return Err(Box::new(state));
    }
    let (tcp, unread) = match state.socket.get_mut().hand_over() {
        Ok(parts) => parts,
        Err(_) => return Err(Box::new(state)),
    };
    let endpoint = state.endpoint().to_string();
    let thread_id = state.thread_id().to_string();
    let next_request_id = state.next_request_id;
    let text_items = std::mem::take(&mut state.text_items);
    // This program's descriptor closes; the connection stays open in `tcp`.
    drop(state);
    Ok(CodexHandover {
        endpoint,
        thread_id,
        turn_id,
        connection: tcp.into_raw_fd(),
        unread,
        next_request_id,
        text_items,
    })
}

/// The running turn of an adopted connection, until its prompt is submitted.
#[derive(Debug)]
pub(super) struct AdoptedTurn {
    turn_id: String,
    text_items: BTreeMap<String, CodexTextTranscriptState>,
}

/// The runtime of a handed-over turn, reading on where the previous program
/// stopped.
pub(super) fn adopt(turn: CodexHandover) -> CodexRuntimeState {
    // SAFETY: the handover names this inherited descriptor for this run only,
    // and the hot reload gives each handover to one claim.
    let tcp = unsafe { std::net::TcpStream::from_raw_fd(turn.connection) };
    let socket = continue_codex_socket(tcp, turn.unread);
    let mut state =
        CodexRuntimeState::new(turn.endpoint, turn.thread_id, socket, turn.next_request_id);
    state.adopted_turn = Some(AdoptedTurn {
        turn_id: turn.turn_id,
        text_items: turn.text_items,
    });
    state
}

/// Continues the handed-over turn on this program.
pub(super) fn continue_turn(state: &mut CodexRuntimeState, turn: AdoptedTurn) {
    state.turn_tracker.reset_for_submitted();
    state.turn_tracker.provider_active_turn_id = Some(turn.turn_id.clone());
    state.turn_tracker.reset_for_started();
    state.active_turn_id = Some(turn.turn_id);
    state.text_items = turn.text_items;
    // Everything the turn streamed meanwhile waits in the connection: a turn
    // that ended during the swap must not settle before it is read.
    state.authoritative_backfill_gate =
        super::backfill::CodexAuthoritativeBackfillGate::started_reconciled(
            std::time::Instant::now(),
        );
}
