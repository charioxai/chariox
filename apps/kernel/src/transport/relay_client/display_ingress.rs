//! Bounded display ingress with a scheduling handoff to its ready consumer.
use super::{RelayClientState, RelayDisplayTunnelClientEvent};
use std::sync::Arc;
use tokio::sync::RwLock;

pub(super) async fn forward_display_client_event(
    state: &Arc<RwLock<RelayClientState>>,
    stream_id: &str,
    event: RelayDisplayTunnelClientEvent,
) -> bool {
    let accepted = state
        .write()
        .await
        .try_send_display_stream_event(stream_id, event);
    if accepted {
        // Do not let a ready relay reader fill the queue before the consumer
        // gets scheduled. This preserves the fragment bound and fail-fast
        // closure for a consumer that remains stalled.
        tokio::task::yield_now().await;
    }
    accepted
}

#[cfg(test)]
mod tests {
    use super::*;
    use chariox_relay::protocol::RelayDisplayTunnelStreamChunk;
    use tokio::sync::mpsc;
    use tokio::time::{timeout, Duration};

    // MP-08/MP-10: buffered acknowledgments must not close a ready consumer.
    #[tokio::test]
    async fn healthy_display_ingress_drains_bursts_without_losing_fragments() {
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (sender, mut receiver) = mpsc::channel(16);
        state
            .write()
            .await
            .insert_display_stream("burst".to_owned(), sender);
        let consumer = tokio::spawn(async move {
            let mut packets = Vec::new();
            while let Some(RelayDisplayTunnelClientEvent::Chunk(chunk)) = receiver.recv().await {
                packets.push(chunk.data);
            }
            packets
        });
        for index in 0..64 {
            assert!(
                forward_display_client_event(
                    &state,
                    "burst",
                    RelayDisplayTunnelClientEvent::Chunk(RelayDisplayTunnelStreamChunk {
                        stream_id: "burst".to_owned(),
                        data: index.to_string(),
                        message_kind: Some("binary".to_owned())
                    })
                )
                .await,
                "ready ingress was closed before its consumer could run"
            );
        }
        state.write().await.remove_display_stream("burst");
        let packets = timeout(Duration::from_secs(1), consumer)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            packets,
            (0..64).map(|index| index.to_string()).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn stalled_display_ingress_closes_without_blocking_a_healthy_stream() {
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (sender, mut receiver) = mpsc::channel(1);
        state
            .write()
            .await
            .insert_display_stream("stalled".to_owned(), sender);
        assert!(
            forward_display_client_event(&state, "stalled", RelayDisplayTunnelClientEvent::Close)
                .await
        );
        assert!(
            !forward_display_client_event(&state, "stalled", RelayDisplayTunnelClientEvent::Close)
                .await
        );
        assert!(receiver.recv().await.is_some());
        assert!(receiver.recv().await.is_none());

        let (sender, mut receiver) = mpsc::channel(1);
        state
            .write()
            .await
            .insert_display_stream("healthy".to_owned(), sender);
        assert!(
            forward_display_client_event(&state, "healthy", RelayDisplayTunnelClientEvent::Close)
                .await
        );
        assert!(receiver.recv().await.is_some());
        assert!(state
            .read()
            .await
            .display_stream_sender("healthy")
            .is_some());
        state.write().await.remove_display_stream("healthy");
        assert!(receiver.recv().await.is_none());
    }
}
