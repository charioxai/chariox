//! Private Selkies process to encrypted display transport. No viewer admission.

use std::process::Stdio;

use chariox_relay::protocol::EncryptedRelayPayload;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::{mpsc, watch};
use tokio::time::{interval, sleep_until, timeout, Duration, Instant, MissedTickBehavior};

use super::secure_display::{DisplayMessageKind, SecureDisplayChannel};
use crate::error::DaemonError;

mod records;
use records::{PrivateRecord, RecordReader};

const IO_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_LEASE: Duration = Duration::from_secs(60);

/// The caller must already authorize the Room, attachment, execution target,
/// peer key, and fresh stream ID. `command` is kernel-selected, never supplied
/// by a viewer. Only the kernel owns the lease sender. Closing it, revoking it,
/// or reaching its monotonic deadline stops the stream, even during blocked I/O.
/// Each direction must have a bounded queue of at most 16 encrypted fragments.
pub(crate) async fn forward_selkies_stream(
    mut command: Command,
    cipher: SecureDisplayChannel,
    incoming: mpsc::Receiver<EncryptedRelayPayload>,
    outgoing: mpsc::Sender<EncryptedRelayPayload>,
    mut lease: watch::Receiver<Option<Instant>>,
) -> Result<(), DaemonError> {
    valid_deadline(*lease.borrow_and_update())?;
    if incoming.max_capacity() > 16 || outgoing.max_capacity() > 16 {
        return Err(stream_error("display fragment queue exceeds its bound"));
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // A subprocess error can include private endpoint credentials. Do not
        // copy arbitrary stderr into transport errors, terminal logs or history.
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| stream_error("cannot start private stream adapter"))?;
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| stream_error("missing private stream input"))?;
    let mut records = RecordReader::new(
        child
            .stdout
            .take()
            .ok_or_else(|| stream_error("missing private stream output"))?,
    );
    let result = {
        let forward = forward_records(&mut input, &mut records, cipher, incoming, outgoing);
        tokio::pin!(forward);
        loop {
            let deadline = match valid_deadline(*lease.borrow_and_update()) {
                Ok(deadline) => deadline,
                Err(error) => break Err(error),
            };
            tokio::select! {
                biased;
                changed = lease.changed() => {
                    if changed.is_err() { break Err(stream_error("display admission was closed")); }
                }
                _ = sleep_until(deadline) => break Err(stream_error("display admission expired")),
                result = &mut forward => break result,
            }
        }
    };
    // EOF asks the adapter to revoke its token. Drain already-queued output so
    // its bounded writer can finish, then reap our child. Never wait forever.
    drop(input);
    let cleanup = timeout(Duration::from_secs(5), async {
        records.drain().await;
        child.wait().await
    })
    .await;
    match cleanup {
        Ok(Ok(status)) if status.success() => result,
        _ => {
            let _ = child.start_kill();
            let _ = timeout(Duration::from_secs(2), child.wait()).await;
            result.and(Err(stream_error("private stream did not stop cleanly")))
        }
    }
}

fn valid_deadline(deadline: Option<Instant>) -> Result<Instant, DaemonError> {
    let now = Instant::now();
    deadline
        .filter(|value| *value > now && *value <= now + MAX_LEASE)
        .ok_or_else(|| stream_error("display admission revoked, expired, or invalid"))
}

async fn forward_records(
    input: &mut tokio::process::ChildStdin,
    records: &mut RecordReader,
    mut cipher: SecureDisplayChannel,
    mut incoming: mpsc::Receiver<EncryptedRelayPayload>,
    outgoing: mpsc::Sender<EncryptedRelayPayload>,
) -> Result<(), DaemonError> {
    let ready = timeout(Duration::from_secs(10), records.next())
        .await
        .map_err(|_| stream_error("private stream startup timed out"))??;
    if !matches!(ready, PrivateRecord::Ready { protocol, read_only: true } if protocol == "selkies-stdio-v1")
    {
        return Err(stream_error(
            "private stream did not confirm read-only protocol",
        ));
    }
    let mut renewal = interval(Duration::from_secs(20));
    renewal.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = renewal.tick() => write_control(input, serde_json::json!({"kind":"renew"})).await?,
            packet = incoming.recv() => {
                let Some(packet) = packet else { return Ok(()); };
                if let Some(message) = cipher.decode(&packet)? {
                    if message.kind != DisplayMessageKind::Text || !safe_viewer_control(&message.data) {
                        return Err(stream_error("viewer input is not a read-only display control"));
                    }
                    let text = std::str::from_utf8(&message.data).map_err(|_| stream_error("invalid display control"))?;
                    write_control(input, serde_json::json!({"kind":"control", "text":text})).await?;
                }
            }
            record = records.next() => {
                let message = record?.into_message()?;
                for packet in cipher.encode(message.kind, &message.data)? {
                    timeout(IO_TIMEOUT, outgoing.send(packet)).await
                        .map_err(|_| stream_error("display consumer stopped reading"))?
                        .map_err(|_| stream_error("display consumer disconnected"))?;
                }
            }
        }
    }
}

async fn write_control(
    input: &mut tokio::process::ChildStdin,
    value: serde_json::Value,
) -> Result<(), DaemonError> {
    let mut bytes =
        serde_json::to_vec(&value).map_err(|_| stream_error("invalid private control"))?;
    bytes.push(b'\n');
    timeout(IO_TIMEOUT, input.write_all(&bytes))
        .await
        .map_err(|_| stream_error("private control write timed out"))?
        .map_err(|_| stream_error("private control input disconnected"))
}

fn safe_viewer_control(bytes: &[u8]) -> bool {
    if matches!(bytes, b"START_VIDEO" | b"STOP_VIDEO" | b"REQUEST_KEYFRAME") {
        return true;
    }
    let Some(number) = bytes.strip_prefix(b"CLIENT_FRAME_ACK ") else {
        return false;
    };
    !number.is_empty()
        && number.len() <= 5
        && number.iter().all(u8::is_ascii_digit)
        && std::str::from_utf8(number)
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .is_some()
}

fn stream_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "private Selkies stream",
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod tests;
