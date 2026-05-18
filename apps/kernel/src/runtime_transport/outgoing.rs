use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::runtime::projection::TransportHealthStore;
use crate::transport::kernel_protocol::KernelOutgoingFrame;

use super::{ConnectionCloseCommand, BACKPRESSURE_CLOSE_REASON};

pub(super) fn try_send_outgoing_frame(
    outgoing_tx: &mpsc::Sender<KernelOutgoingFrame>,
    close_tx: &mpsc::UnboundedSender<ConnectionCloseCommand>,
    close_requested: &Arc<AtomicBool>,
    transport_health: &TransportHealthStore,
    frame: KernelOutgoingFrame,
    session_id: Option<&str>,
    attachment_id: Option<&str>,
) -> bool {
    match outgoing_tx.try_send(frame) {
        Ok(()) => true,
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            transport_health.record_outgoing_queue_overflow();
            if !close_requested.swap(true, Ordering::SeqCst) {
                transport_health.record_slow_consumer_close();
                crate::logging::warn_with_fields(
                    "daemon.runtime_transport",
                    "kernel websocket connection overflowed; closing slow consumer",
                    serde_json::json!({
                        "session_id": session_id,
                        "attachment_id": attachment_id,
                    }),
                );
                let _ = close_tx.send(ConnectionCloseCommand {
                    reason: BACKPRESSURE_CLOSE_REASON.to_string(),
                });
            }
            false
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => false,
    }
}
