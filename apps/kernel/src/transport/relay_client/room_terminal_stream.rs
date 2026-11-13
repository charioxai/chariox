//! A Room terminal's output pushed to one viewer as the program writes it.
//!
//! A viewer that polled with `ReadOutput` paid one request and one reply, each with its
//! own relay framing and the terminal's full summary, for every few kilobytes of output.
//! Here the viewer subscribes once from the offset it has drawn; the kernel reads on its
//! side and sends each page as one event, with the summary only when it changed. The event
//! id is the offset after the page, so a viewer that lost its link subscribes again from it.
//! The same stream keeps the viewer's tabs current: the Room's terminals, when they change,
//! instead of a list request every few seconds.

use base64::Engine as _;
use serde::Serialize;

use super::subscriptions::{relay_subscription_task_key, RelaySubscriptionTask};
use super::*;
use crate::local::{RoomTerminalOperation, RoomTerminalRequest, RoomTerminalResult};
use crate::room_terminal::{RoomTerminalOutput, RoomTerminalStatus, RoomTerminalSummary};
use crate::runtime::command::KernelCaller;

/// Protocol 1006: the subscription scope of a Room terminal's output stream.
pub(super) const ROOM_TERMINAL_SCOPE: &str = "room_terminal";

/// How often the stream compares the Room's terminals with the tabs the viewer has, as the
/// viewer's own list poll did.
const LIST_INTERVAL: Duration = Duration::from_secs(3);

/// What a viewer subscribes to: `attachment_id` names the terminal, as
/// `<terminal_id>` or, for a terminal on the machine of a Room agent,
/// `<terminal_id>@<host_agent>`; the resume cursor is the offset it has drawn.
pub(super) struct RoomTerminalSubscription {
    pub(super) request_id: String,
    pub(super) relay_id: String,
    pub(super) session_id: String,
    pub(super) address: String,
    pub(super) identity: Option<chariox_relay::protocol::RelayCallerIdentity>,
    pub(super) public_key: String,
    pub(super) from_offset: Option<u64>,
    pub(super) heartbeat_interval: Duration,
}

#[derive(Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum StreamEvent<'a> {
    /// The bytes after the viewer's offset travel raw after the header.
    RoomTerminalOutput {
        next_offset: u64,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        gap: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        terminal: Option<&'a RoomTerminalSummary>,
    },
    /// The Room's terminals, on the first page and whenever any but their offsets changed.
    RoomTerminalsListed {
        terminals: &'a [RoomTerminalSummary],
    },
    /// The stream sent nothing for a heartbeat interval.
    Heartbeat,
    /// The stream stopped; the viewer subscribes again or reads a snapshot.
    RoomTerminalStreamEnded { message: &'a str },
}

/// Answers the subscription once the first read shows the caller may see the terminal, then
/// pushes its output until the terminal ends, its offset is gone or the viewer unsubscribes.
pub(super) async fn handle_subscribe(
    router: &Arc<CommandRouter>,
    outgoing_tx: &RelayOutgoingSender,
    tasks: &RelaySubscriptionTasks,
    subscription: RoomTerminalSubscription,
) -> Result<(), DaemonError> {
    let key = relay_subscription_task_key(
        &subscription.session_id,
        &subscription.address,
        Some(ROOM_TERMINAL_SCOPE),
        &subscription.relay_id,
    );
    let relay_id = subscription.relay_id.clone();
    let public_key = subscription.public_key.clone();
    let handle = tokio::spawn(run(router.clone(), outgoing_tx.clone(), subscription));
    if let Some(existing) = tasks.lock().await.insert(
        key,
        RelaySubscriptionTask {
            relay_subscription_id: relay_id,
            client_public_key: public_key,
            display_id: None,
            display_pump: None,
            _terminal_lease: None,
            resource_watch: None,
            handle,
        },
    ) {
        existing.handle.abort();
    }
    Ok(())
}

async fn run(
    router: Arc<CommandRouter>,
    outgoing_tx: RelayOutgoingSender,
    subscription: RoomTerminalSubscription,
) {
    let RoomTerminalSubscription {
        request_id,
        relay_id,
        session_id,
        address,
        identity,
        public_key,
        from_offset,
        heartbeat_interval,
    } = subscription;
    let opened = open(
        &router,
        session_id,
        &address,
        identity,
        &public_key,
        from_offset,
    )
    .await;
    let (reader, first) = match opened {
        Ok(opened) => opened,
        Err(error) => {
            let _ = send_outgoing_envelope(
                &outgoing_tx,
                RelayEnvelope::DaemonResponse {
                    relay_request_id: request_id,
                    encrypted_response: None,
                    error: Some(super::request_errors::map_relay_error(&error)),
                },
            );
            return;
        }
    };
    let acknowledged = match encrypt_json_response(
        &router,
        &public_key,
        serde_json::json!({ "ok": true }),
    )
    .await
    {
        Ok(ack) => send_outgoing_envelope(
            &outgoing_tx,
            RelayEnvelope::DaemonResponse {
                relay_request_id: request_id,
                encrypted_response: Some(ack),
                error: None,
            },
        ),
        Err(error) => Err(error),
    };
    if acknowledged.is_err() {
        return;
    }
    let stream = Stream {
        router,
        outgoing_tx,
        relay_id,
        public_key,
    };
    let mut reached = first.next_offset;
    if let Err(error) = stream
        .push(&reader, first, heartbeat_interval, &mut reached)
        .await
    {
        let message = error.to_string();
        let _ = stream
            .send(
                &StreamEvent::RoomTerminalStreamEnded { message: &message },
                reached,
                &[],
            )
            .await;
    }
}

/// The first read checks that the caller may see the terminal before the stream is answered.
async fn open(
    router: &CommandRouter,
    session_id: String,
    address: &str,
    identity: Option<chariox_relay::protocol::RelayCallerIdentity>,
    public_key: &str,
    from_offset: Option<u64>,
) -> Result<(Reader, RoomTerminalOutput), DaemonError> {
    let ((terminal_id, host_agent), offset) =
        parse_address(address).zip(from_offset).ok_or_else(|| {
            crate::room_terminal::refusal(
                "a Room terminal stream names `<terminal_id>[@<host_agent>]` and the offset to start from"
                    .to_string(),
            )
        })?;
    let reader = Reader {
        session_id,
        terminal_id,
        host_agent,
        caller: super::browser_display::stream_caller(router, identity, public_key)?,
    };
    let first = reader.read(router, offset, Duration::ZERO).await?;
    Ok((reader, first))
}

struct Stream {
    router: Arc<CommandRouter>,
    outgoing_tx: RelayOutgoingSender,
    relay_id: String,
    public_key: String,
}

impl Stream {
    /// `Ok` once nothing is left to send: the terminal ended and its output is read to its
    /// end, or the viewer's offset is gone and it resyncs from a snapshot.
    async fn push(
        &self,
        reader: &Reader,
        first: RoomTerminalOutput,
        heartbeat_interval: Duration,
        reached: &mut u64,
    ) -> Result<(), DaemonError> {
        let mut shown: Option<RoomTerminalSummary> = None;
        let mut tabs: Option<Vec<RoomTerminalSummary>> = None;
        let mut listed_at: Option<Instant> = None;
        let mut sent_at = Instant::now();
        let mut page = first;
        loop {
            self.router.admit_relay_stream(&self.public_key)?;
            *reached = page.next_offset;
            if listed_at.is_none_or(|at| at.elapsed() >= LIST_INTERVAL) {
                listed_at = Some(Instant::now());
                // A Room-wide list fails while a worker is away: the viewer keeps the tabs it has.
                if let Ok(listed) = reader.list(&self.router).await {
                    if tabs.as_ref().is_none_or(|tabs| !same_tabs(tabs, &listed)) {
                        let event = StreamEvent::RoomTerminalsListed { terminals: &listed };
                        self.send(&event, page.next_offset, &[]).await?;
                        sent_at = Instant::now();
                        tabs = Some(listed);
                    }
                }
            }
            let data = base64::engine::general_purpose::STANDARD
                .decode(&page.data_base64)
                .map_err(|_| {
                    crate::room_terminal::refusal(
                        "the Room terminal answered with invalid output".to_string(),
                    )
                })?;
            let ended = matches!(page.terminal.status, RoomTerminalStatus::Ended { .. })
                && page.next_offset >= page.terminal.output_offset;
            let changed = shown
                .as_ref()
                .is_none_or(|shown| !same_terminal(shown, &page.terminal));
            if !data.is_empty() || page.gap || changed || ended {
                let event = StreamEvent::RoomTerminalOutput {
                    next_offset: page.next_offset,
                    gap: page.gap,
                    terminal: (changed || ended).then_some(&page.terminal),
                };
                self.send(&event, page.next_offset, &data).await?;
                sent_at = Instant::now();
                if page.gap || ended {
                    return Ok(());
                }
                shown = Some(page.terminal.clone());
            } else if sent_at.elapsed() >= heartbeat_interval {
                self.send(&StreamEvent::Heartbeat, page.next_offset, &[])
                    .await?;
                sent_at = Instant::now();
            }
            page = reader
                .read(
                    &self.router,
                    page.next_offset,
                    LIST_INTERVAL.min(heartbeat_interval),
                )
                .await?;
        }
    }

    async fn send(
        &self,
        event: &StreamEvent<'_>,
        event_id: u64,
        data: &[u8],
    ) -> Result<(), DaemonError> {
        let serialize = |error: serde_json::Error| DaemonError::LocalTransport {
            operation: "serialize Room terminal stream event",
            message: error.to_string(),
        };
        let header = serde_json::to_value(event).map_err(serialize)?;
        let plaintext = match event {
            StreamEvent::RoomTerminalOutput { .. } => {
                crate::transport::terminal_output_wire::encode_room_output(header, data)
                    .map_err(serialize)?
            }
            _ => serde_json::to_vec(&header).map_err(serialize)?,
        };
        let encrypted = relay_crypto::encrypt_payload_for_peer(
            &self.router.relay_private_key(),
            &self.public_key,
            &plaintext,
        )?;
        super::events::send_relay_event_frame(
            &self.outgoing_tx,
            &self.relay_id,
            event_id,
            encrypted,
        )
        .await
    }
}

/// The tabs differ when a terminal came, went or changed in anything but its offset.
fn same_tabs(sent: &[RoomTerminalSummary], now: &[RoomTerminalSummary]) -> bool {
    sent.len() == now.len()
        && sent
            .iter()
            .zip(now)
            .all(|(sent, now)| same_terminal(sent, now))
}

/// A summary differs from the one sent when anything but its offset changed.
fn same_terminal(sent: &RoomTerminalSummary, now: &RoomTerminalSummary) -> bool {
    *sent
        == RoomTerminalSummary {
            output_offset: sent.output_offset,
            ..now.clone()
        }
}

/// Where the stream reads: the Room, the terminal, the machine that runs it, and as whom.
struct Reader {
    session_id: String,
    terminal_id: String,
    host_agent: Option<String>,
    caller: KernelCaller,
}

/// `<terminal_id>` or `<terminal_id>@<host_agent>`.
fn parse_address(address: &str) -> Option<(String, Option<String>)> {
    let (terminal_id, host_agent) = match address.split_once('@') {
        Some((terminal_id, host_agent)) => (terminal_id, Some(host_agent)),
        None => (address, None),
    };
    (!terminal_id.is_empty() && host_agent != Some(""))
        .then(|| (terminal_id.to_string(), host_agent.map(str::to_string)))
}

impl Reader {
    /// The Room-wide list a polling viewer sends: this kernel's terminals and its workers'.
    async fn list(&self, router: &CommandRouter) -> Result<Vec<RoomTerminalSummary>, DaemonError> {
        let request = RoomTerminalRequest {
            session_id: self.session_id.clone(),
            acting_agent: None,
            host_agent: None,
            operation: RoomTerminalOperation::List,
        };
        match router
            .runtime_state()
            .room_terminal_for_client(&self.caller, request)
            .await?
        {
            crate::local::LocalDaemonResponse::RoomTerminal {
                result: RoomTerminalResult::Listed { terminals },
            } => Ok(terminals),
            other => Err(crate::room_terminal::refusal(format!(
                "unexpected answer to a Room terminal list: {other:?}"
            ))),
        }
    }

    /// The same read a polling viewer sends, with the same access check and routing.
    async fn read(
        &self,
        router: &CommandRouter,
        after_offset: u64,
        wait: Duration,
    ) -> Result<RoomTerminalOutput, DaemonError> {
        let request = RoomTerminalRequest {
            session_id: self.session_id.clone(),
            acting_agent: None,
            host_agent: self.host_agent.clone(),
            operation: RoomTerminalOperation::ReadOutput {
                terminal_id: self.terminal_id.clone(),
                after_offset,
                wait_ms: wait.as_millis() as u64,
            },
        };
        match router
            .runtime_state()
            .room_terminal_for_client(&self.caller, request)
            .await?
        {
            crate::local::LocalDaemonResponse::RoomTerminal {
                result: RoomTerminalResult::Output { output },
            } => Ok(output),
            other => Err(crate::room_terminal::refusal(format!(
                "unexpected answer to a Room terminal read: {other:?}"
            ))),
        }
    }
}

#[cfg(test)]
mod protocol_shape {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn mp08_room_terminal_stream_1006_snapshot_and_hash() {
        let bytes =
            include_bytes!("../../local/api/tests/protocol_shapes/room-terminal-stream-1006.json");
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            "ddfa07a77ed61ca4d4c8ed0c80b4223f3ed3c7e597723135fa16a244a02f2a0a"
        );
        let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        assert_eq!(
            fixture["local_protocol"],
            crate::local::LOCAL_DAEMON_PROTOCOL_VERSION
        );
        assert_eq!(
            fixture["peer_protocol"],
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
        );
        assert_eq!(fixture["subscription_scope"], ROOM_TERMINAL_SCOPE);
        assert_eq!(
            fixture["attachment_id"]
                .as_array()
                .unwrap()
                .iter()
                .map(|address| parse_address(address.as_str().unwrap()))
                .collect::<Vec<_>>(),
            [
                Some(("term-1a2b".to_string(), None)),
                Some(("term-1a2b".to_string(), Some("agent-w".to_string())))
            ]
        );
        assert_eq!(parse_address("term-1a2b@"), None);
        assert_eq!(parse_address("@agent-w"), None);
        let header = |event: &StreamEvent, data: &[u8]| {
            let wire = crate::transport::terminal_output_wire::encode_room_output(
                serde_json::to_value(event).unwrap(),
                data,
            )
            .unwrap();
            let size = u32::from_be_bytes(wire[4..8].try_into().unwrap()) as usize;
            assert_eq!(&wire[8 + size..], data, "the bytes travel raw");
            serde_json::from_slice::<serde_json::Value>(&wire[8..8 + size]).unwrap()
        };
        let page = StreamEvent::RoomTerminalOutput {
            next_offset: 4893,
            gap: false,
            terminal: None,
        };
        assert_eq!(header(&page, &[b'x'; 4893]), fixture["output_header"]);
        let gap = StreamEvent::RoomTerminalOutput {
            next_offset: 9000,
            gap: true,
            terminal: None,
        };
        assert_eq!(header(&gap, &[]), fixture["gap_header"]);
        let listed = StreamEvent::RoomTerminalsListed { terminals: &[] };
        assert_eq!(serde_json::to_value(listed).unwrap(), fixture["listed"]);
        assert_eq!(
            serde_json::to_value(StreamEvent::Heartbeat).unwrap(),
            fixture["heartbeat"]
        );
        let ended = StreamEvent::RoomTerminalStreamEnded {
            message: fixture["ended"]["message"].as_str().unwrap(),
        };
        assert_eq!(serde_json::to_value(ended).unwrap(), fixture["ended"]);
    }
}
