//! A placed launch keeps its exec identity here and runs on another kernel.
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use tokio::sync::{mpsc, watch};

use super::placement::Placed;
use super::process::cancelled;
use super::{CancelReason, ExecOutcome, ExecStream, Record, RoomExecService};

const READ_WAIT: Duration = Duration::from_millis(250);

pub(super) async fn run(
    record: Arc<Record>,
    mut input: mpsc::Receiver<Option<Vec<u8>>>,
    mut cancel: watch::Receiver<Option<CancelReason>>,
    service: RoomExecService,
    placed: Arc<Placed>,
) {
    let target = &placed.target;
    // Never abandoned half-way: dropping the open could lose the only handle to
    // a command that already started there.
    let remote_id = match target
        .open(&record.intent, placed.attributed_agent.clone())
        .await
    {
        Ok(id) => id,
        Err(failure) => {
            // A lost start may have started the command; it is never sent again.
            let message = if matches!(&failure, crate::error::DaemonError::MachineRequest { code, .. } if code == "kernel_outcome_unknown")
            {
                "the other kernel may have started the command; its outcome is unknown, so it was not started again".to_string()
            } else {
                format!("the other kernel did not start the command: {failure}")
            };
            service.finish(&record, ExecOutcome::Failed { message });
            return;
        }
    };
    *record.state.lock().expect("exec state poisoned") = super::ExecState::Running;
    record.changed.notify_waiters();
    let lost = |failure: crate::error::DaemonError| {
        ExecOutcome::Failed {
        message: format!(
            "lost the command on the other kernel: {failure}; its outcome is unknown, so it was not started again"
        ),
    }
    };
    let (mut input_open, mut canceled) = (true, false);
    let (mut sent, mut cursor) = (0u64, 0u64);
    let outcome = 'run: loop {
        tokio::select! {
            biased;
            _ = cancelled(&mut cancel), if !canceled => {
                canceled = true;
                if let Err(failure) = target.cancel(&remote_id).await {
                    break 'run lost(failure);
                }
            }
            message = input.recv(), if input_open => match message {
                Some(Some(bytes)) => {
                    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
                    match target.write(&remote_id, sent, &encoded).await {
                        Ok(next) => sent = next,
                        Err(failure) => break 'run lost(failure),
                    }
                }
                Some(None) => {
                    input_open = false;
                    if let Err(failure) = target.close_stdin(&remote_id, sent).await {
                        break 'run lost(failure);
                    }
                }
                None => input_open = false,
            },
            page = target.read(&remote_id, cursor, READ_WAIT) => {
                let page = match page {
                    Ok(page) => page,
                    Err(failure) => break 'run lost(failure),
                };
                for chunk in &page.chunks {
                    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&chunk.data_base64) else {
                        break 'run ExecOutcome::Failed {
                            message: "the other kernel sent output that is not valid base64; the stream is incomplete".into(),
                        };
                    };
                    if !store(&record, chunk.stream, &bytes, &mut cancel).await {
                        break;
                    }
                }
                cursor = page.next_cursor;
                if let Some(outcome) = page.outcome {
                    break 'run outcome;
                }
            }
        }
    };
    service.finish(&record, outcome);
}

/// Returns false when the launch was cancelled while the reader was behind.
async fn store(
    record: &Record,
    stream: ExecStream,
    bytes: &[u8],
    cancel: &mut watch::Receiver<Option<CancelReason>>,
) -> bool {
    loop {
        let space = record.space.notified();
        tokio::pin!(space);
        space.as_mut().enable();
        if record
            .output
            .lock()
            .expect("exec output poisoned")
            .push(stream, bytes)
        {
            record.changed.notify_waiters();
            return true;
        }
        tokio::select! {
            _ = cancelled(cancel) => return false,
            _ = space => {},
        }
    }
}
