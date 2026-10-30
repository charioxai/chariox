//! Hot reload: a running Codex turn continues in the next kernel program on
//! the same app-server connection. The app-server keeps running the turn, and
//! what it streams during the swap waits in the connection, so every delta
//! arrives once and in order.

use std::collections::BTreeMap;
use std::os::fd::{FromRawFd, IntoRawFd};

use serde::{Deserialize, Serialize};

use crate::provider::codex_client::continue_codex_socket;
use crate::provider::CodexNotification;

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
    // Notifications already read from the connection, not yet applied.
    buffered_notifications: Vec<CodexNotification>,
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
    if !state.thread_ready() || state.ephemeral || state.notification_only {
        return Err(Box::new(state));
    }
    // A pong the socket owes goes out before the connection changes hands.
    let parts = state
        .socket
        .flush()
        .map_err(std::io::Error::other)
        .and_then(|()| state.socket.get_mut().hand_over());
    let (tcp, unread) = match parts {
        Ok(parts) => parts,
        Err(error) => {
            crate::logging::warn_with_fields(
                "hot_reload.codex",
                "the Codex connection could not be handed over; the turn stops as for a restart",
                serde_json::json!({ "thread_id": state.thread_id(), "error": error.to_string() }),
            );
            return Err(Box::new(state));
        }
    };
    let endpoint = state.endpoint().to_string();
    let thread_id = state.thread_id().to_string();
    let next_request_id = state.next_request_id;
    let buffered_notifications = std::mem::take(&mut state.buffered_notifications);
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
        buffered_notifications,
        text_items,
    })
}

/// The running turn of an adopted connection, until its prompt is submitted.
#[derive(Debug)]
pub(super) struct AdoptedTurn {
    turn_id: String,
    buffered_notifications: Vec<CodexNotification>,
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
        buffered_notifications: turn.buffered_notifications,
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
    state.buffered_notifications = turn.buffered_notifications;
    state.text_items = turn.text_items;
    // Everything the turn streamed meanwhile waits in the connection: a turn
    // that ended during the swap must not settle before it is read.
    state.authoritative_backfill_gate =
        super::backfill::CodexAuthoritativeBackfillGate::started_reconciled(
            std::time::Instant::now(),
        );
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::time::Duration;

    use tokio_tungstenite::tungstenite::{accept, Message};

    use super::*;
    use crate::provider::{test_codex_connect, ProviderRuntimeBinding};

    /// A poll that did not drain to quiet reads one notification ahead: the
    /// turn goes over with it, and the next program applies it first.
    #[test]
    fn a_handover_carries_the_notifications_already_read() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = accept(stream).unwrap();
            socket.send(Message::text("after the swap")).unwrap();
            std::thread::sleep(Duration::from_millis(500));
        });
        let (socket, ()) = test_codex_connect(&endpoint).unwrap();
        let ProviderRuntimeBinding::Codex(binding) =
            CodexRuntimeState::active_turn_binding_fixture(endpoint, socket)
        else {
            unreachable!("a Codex fixture")
        };
        let mut state = binding.state;
        let read_ahead = CodexNotification::AgentMessageDelta {
            item_id: "msg-1".into(),
            delta: "read ahead".into(),
        };
        state.buffered_notifications.push(read_ahead.clone());

        let handover =
            hand_over_codex_turn(state).unwrap_or_else(|_| panic!("a running turn is handed over"));
        // Through the handover file, as the next program reads it.
        let handover: CodexHandover =
            serde_json::from_slice(&serde_json::to_vec(&handover).unwrap()).unwrap();
        let mut adopted = adopt(handover);
        let turn = adopted.adopted_turn.take().unwrap();
        continue_turn(&mut adopted, turn);

        assert_eq!(adopted.buffered_notifications, [read_ahead]);
        assert_eq!(adopted.active_turn_id.as_deref(), Some("notification-turn"));
        assert_eq!(
            adopted.socket.read().unwrap(),
            Message::text("after the swap")
        );
        server.join().unwrap();
    }
}
