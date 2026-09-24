//! Fixed test transports only. The same signed policy, writer, real worker
//! identity and bounded ports remain mandatory. No URL/IP bypass ships.
use super::*;
use http_body_util::BodyExt;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::{watch, Notify};

pub(crate) enum FixedNetwork {
    Echo,
    Paused,
}
pub(crate) struct NetworkFixture {
    pub(crate) kind: FixedNetwork,
    pub(crate) entered: Notify,
    pub(crate) cancelled: Notify,
    pub(crate) released: Notify,
    pub(crate) uploaded: AtomicUsize,
}
impl NetworkFixture {
    pub(crate) fn new(kind: FixedNetwork) -> Arc<Self> {
        Arc::new(Self {
            kind,
            entered: Notify::new(),
            cancelled: Notify::new(),
            released: Notify::new(),
            uploaded: AtomicUsize::new(0),
        })
    }
    pub(super) async fn run(
        &self,
        exchange: transport::Exchange,
        mut stopped: watch::Receiver<bool>,
    ) -> Result<()> {
        self.entered.notify_one();
        if matches!(self.kind, FixedNetwork::Paused) {
            // Hold the actual two-chunk queues through observed cancellation and
            // an explicit cleanup checkpoint; no socket or App code is involved.
            let _exchange = exchange;
            cancelled(&mut stopped).await;
            self.cancelled.notify_one();
            self.released.notified().await;
            return Err(HttpError::Cancelled);
        }
        let (mut upload, headers, chunks) = exchange.fixture_parts();
        headers
            .send(Ok(transport::ResponseHead {
                status: 200,
                headers: vec![],
                url: "https://api.example.com/fixture".into(),
            }))
            .map_err(|_| HttpError::Cancelled)?;
        loop {
            let frame = tokio::select! { biased;
                _ = cancelled(&mut stopped) => return Err(HttpError::Cancelled),
                frame = upload.frame() => frame,
            };
            let Some(frame) = frame else {
                break;
            };
            let bytes = frame?.into_data().map_err(|_| HttpError::Invalid)?;
            self.uploaded.fetch_add(bytes.len(), Ordering::AcqRel);
        }
        chunks
            .send(Ok(bytes::Bytes::from_static(b"reply")))
            .await
            .map_err(|_| HttpError::Cancelled)?;
        Ok(())
    }
}
