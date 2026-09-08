use super::super::{HttpError, Result, CHUNK_BYTES, MAX_REQUEST_BYTES};
use bytes::Bytes;
use hyper::body::{Body, Frame, SizeHint};
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::sync::mpsc;

enum Upload {
    Data(Bytes),
    End,
}
pub(in crate::runtime::app_http) struct UploadBody {
    receiver: mpsc::Receiver<Upload>,
    ended: bool,
    received: u64,
}
pub(in crate::runtime::app_http) struct UploadPort {
    sender: mpsc::Sender<Upload>,
    sent: u64,
    ended: bool,
    poisoned: bool,
}
pub(super) fn channel(has_body: bool) -> (UploadPort, UploadBody) {
    let (sender, receiver) = mpsc::channel(2);
    (
        UploadPort {
            sender,
            sent: 0,
            ended: !has_body,
            poisoned: false,
        },
        UploadBody {
            receiver,
            received: 0,
            ended: !has_body,
        },
    )
}
impl UploadPort {
    /// The caller must retain exclusive mutable access through this await;
    /// only two bounded chunks can be queued ahead of the actual socket.
    pub(in crate::runtime::app_http) async fn write(
        &mut self,
        bytes: Bytes,
        end: bool,
    ) -> Result<()> {
        if self.ended || self.poisoned || bytes.len() > CHUNK_BYTES || (bytes.is_empty() && !end) {
            return Err(HttpError::Invalid);
        }
        let total = self
            .sent
            .checked_add(bytes.len() as u64)
            .ok_or(HttpError::Limit)?;
        if total > MAX_REQUEST_BYTES {
            return Err(HttpError::Limit);
        }
        // Dropping an in-flight write may have queued data before its end marker.
        // It permanently poisons this upload instead of making a retry appear
        // safe. The broker then cancels/drains the entire stream.
        self.poisoned = true;
        if !bytes.is_empty() {
            self.sender
                .send(Upload::Data(bytes))
                .await
                .map_err(|_| HttpError::Cancelled)?;
            self.sent = total;
        }
        if end {
            self.sender
                .send(Upload::End)
                .await
                .map_err(|_| HttpError::Cancelled)?;
            self.ended = true;
        }
        self.poisoned = false;
        Ok(())
    }
}
impl UploadBody {
    pub(super) fn empty() -> Self {
        channel(false).1
    }
}
impl Body for UploadBody {
    type Data = Bytes;
    type Error = HttpError;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>>>> {
        if self.ended {
            return Poll::Ready(None);
        }
        match self.receiver.poll_recv(cx) {
            Poll::Ready(Some(Upload::Data(bytes))) => {
                self.received += bytes.len() as u64;
                if bytes.len() > CHUNK_BYTES || self.received > MAX_REQUEST_BYTES {
                    self.ended = true;
                    Poll::Ready(Some(Err(HttpError::Limit)))
                } else {
                    Poll::Ready(Some(Ok(Frame::data(bytes))))
                }
            }
            Poll::Ready(Some(Upload::End)) => {
                self.ended = true;
                Poll::Ready(None)
            }
            Poll::Ready(None) => {
                self.ended = true;
                Poll::Ready(Some(Err(HttpError::Cancelled)))
            }
            Poll::Pending => Poll::Pending,
        }
    }
    fn is_end_stream(&self) -> bool {
        self.ended
    }
    fn size_hint(&self) -> SizeHint {
        let mut hint = SizeHint::new();
        if self.ended {
            hint.set_exact(0);
        }
        hint
    }
}
