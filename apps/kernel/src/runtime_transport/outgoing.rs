use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::runtime::projection::TransportHealthStore;
use crate::transport::kernel_protocol::KernelOutgoingFrame;

use super::{ConnectionCloseCommand, BACKPRESSURE_CLOSE_REASON};

#[derive(Debug, Clone)]
pub(super) struct KernelOutgoingSender {
    priority_tx: mpsc::Sender<KernelOutgoingFrame>,
    event_tx: mpsc::Sender<KernelOutgoingFrame>,
}

impl KernelOutgoingSender {
    pub(super) fn new(
        priority_tx: mpsc::Sender<KernelOutgoingFrame>,
        event_tx: mpsc::Sender<KernelOutgoingFrame>,
    ) -> Self {
        Self {
            priority_tx,
            event_tx,
        }
    }

    fn try_send(
        &self,
        frame: KernelOutgoingFrame,
    ) -> Result<(), mpsc::error::TrySendError<KernelOutgoingFrame>> {
        match frame {
            KernelOutgoingFrame::Response { .. } => self.priority_tx.try_send(frame),
            KernelOutgoingFrame::Event { .. } => self.event_tx.try_send(frame),
        }
    }
}

pub(super) fn try_send_outgoing_frame(
    outgoing_tx: &KernelOutgoingSender,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_responses_to_priority_queue_and_events_to_event_queue() {
        let (priority_tx, mut priority_rx) = mpsc::channel(1);
        let (event_tx, mut event_rx) = mpsc::channel(1);
        let sender = KernelOutgoingSender::new(priority_tx, event_tx);

        sender
            .try_send(KernelOutgoingFrame::Response {
                request_id: "request-1".to_string(),
                response: Box::new(None),
                error: None,
            })
            .expect("response should route");
        sender
            .try_send(KernelOutgoingFrame::Event {
                event_id: 1,
                event: Box::new(crate::transport::kernel_protocol::KernelEvent::Heartbeat {
                    session_id: "session-1".to_string(),
                }),
            })
            .expect("event should route");

        assert!(matches!(
            priority_rx.try_recv(),
            Ok(KernelOutgoingFrame::Response { request_id, .. }) if request_id == "request-1"
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Ok(KernelOutgoingFrame::Event { event_id, .. }) if event_id == 1
        ));
    }

    #[test]
    fn priority_queue_overflow_requests_slow_consumer_close() {
        let (priority_tx, _priority_rx) = mpsc::channel(1);
        let (event_tx, _event_rx) = mpsc::channel(1);
        let sender = KernelOutgoingSender::new(priority_tx, event_tx);
        let (close_tx, mut close_rx) = mpsc::unbounded_channel();
        let close_requested = Arc::new(AtomicBool::new(false));
        let transport_health = TransportHealthStore::default();

        assert!(try_send_outgoing_frame(
            &sender,
            &close_tx,
            &close_requested,
            &transport_health,
            KernelOutgoingFrame::Response {
                request_id: "request-1".to_string(),
                response: Box::new(None),
                error: None,
            },
            None,
            None,
        ));
        assert!(!try_send_outgoing_frame(
            &sender,
            &close_tx,
            &close_requested,
            &transport_health,
            KernelOutgoingFrame::Response {
                request_id: "request-2".to_string(),
                response: Box::new(None),
                error: None,
            },
            None,
            None,
        ));

        assert_eq!(
            close_rx
                .try_recv()
                .expect("close should be requested")
                .reason,
            BACKPRESSURE_CLOSE_REASON
        );
        let snapshot = transport_health.snapshot(1, 1, 1);
        assert_eq!(snapshot.outgoing_queue_overflows, 1);
        assert_eq!(snapshot.slow_consumer_closes, 1);
    }
}
