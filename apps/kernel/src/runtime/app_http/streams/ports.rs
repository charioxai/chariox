use super::*;
use bytes::Bytes;
use chariox_app_runtime::worker_peer::BrokerCancellation;

#[derive(Debug)]
pub(in crate::runtime::app_http) enum ReadResult {
    Chunk(Bytes),
    Pending,
    End,
}

struct WriteGuard {
    group: Weak<Inner>,
    id: String,
    entry: Arc<Entry>,
    completed: bool,
}
impl Drop for WriteGuard {
    fn drop(&mut self) {
        if !self.completed {
            // Cancellation may follow accepted upload bytes. Poison and close
            // the entire stream; no retry or body replay is performed here.
            retire(&self.group, &self.id, &self.entry.stop);
        }
    }
}

pub(super) struct StreamPort {
    pub(super) group: Weak<Inner>,
    pub(super) id: String,
    pub(super) entry: Arc<Entry>,
}
impl StreamPort {
    pub(super) fn cancel(&self) {
        retire(&self.group, &self.id, &self.entry.stop);
    }

    /// Called after the shared operation-authority dispatcher admits this
    /// write. One in-flight write owns the upload port through actual enqueue.
    pub(in crate::runtime::app_http) async fn write(
        &self,
        bytes: Bytes,
        end: bool,
        deadline: Instant,
        mut cancellation: BrokerCancellation,
    ) -> Result<()> {
        if bytes.len() > super::super::CHUNK_BYTES || (bytes.is_empty() && !end) {
            return Err(HttpError::Invalid);
        }
        let entry = &self.entry;
        if *entry.stop.borrow() || Instant::now() >= entry.deadline {
            return Err(HttpError::Cancelled);
        }
        let mut upload = entry.upload.try_lock().map_err(|_| HttpError::Busy)?;
        let mut stopped = entry.stop.subscribe();
        let mut guard = WriteGuard {
            group: self.group.clone(),
            id: self.id.clone(),
            entry: entry.clone(),
            completed: false,
        };
        let outcome = tokio::select! { biased;
            _ = super::super::cancelled(&mut stopped) => Err(HttpError::Cancelled),
            _ = cancellation.cancelled() => Err(HttpError::Cancelled),
            _ = tokio::time::sleep_until(deadline.min(entry.deadline)) => Err(HttpError::Deadline),
            result = upload.write(bytes, end) => result,
        };
        guard.completed = outcome.is_ok();
        outcome
    }

    /// `wait_until` is a bounded pull interval shorter than the peer request's
    /// deadline. None means pending, so SSE can wait across multiple IPC calls.
    pub(in crate::runtime::app_http) async fn headers(
        &self,
        wait_until: Instant,
        mut cancellation: BrokerCancellation,
    ) -> Result<Option<ResponseHead>> {
        let entry = &self.entry;
        if *entry.stop.borrow() || Instant::now() >= entry.deadline {
            return Err(HttpError::Cancelled);
        }
        let mut receive = entry.receive.try_lock().map_err(|_| HttpError::Busy)?;
        if let Some(head) = &receive.head {
            return Ok(Some(head.clone()));
        }
        let mut stopped = entry.stop.subscribe();
        let head = tokio::select! { biased;
            _ = super::super::cancelled(&mut stopped) => return Err(HttpError::Cancelled),
            _ = cancellation.cancelled() => return Err(HttpError::Cancelled),
            _ = tokio::time::sleep_until(wait_until.min(entry.deadline)) => return Ok(None),
            result = &mut receive.port.headers => result.unwrap_or(Err(HttpError::Network)),
        };
        match head {
            Ok(head) => {
                receive.head = Some(head.clone());
                Ok(Some(head))
            }
            Err(error) => {
                self.cancel();
                Err(error)
            }
        }
    }

    /// Exactly one reader consumes each chunk. EOF alone is insufficient:
    /// transport errors can outlive a full response queue, and are retained in
    /// the task's completion watch until the final reader observes them.
    pub(in crate::runtime::app_http) async fn read(
        &self,
        wait_until: Instant,
        mut cancellation: BrokerCancellation,
    ) -> Result<ReadResult> {
        let entry = &self.entry;
        if *entry.stop.borrow() || Instant::now() >= entry.deadline {
            return Err(HttpError::Cancelled);
        }
        let mut receive = entry.receive.try_lock().map_err(|_| HttpError::Busy)?;
        let mut stopped = entry.stop.subscribe();
        let chunk = tokio::select! { biased;
            _ = super::super::cancelled(&mut stopped) => return Err(HttpError::Cancelled),
            _ = cancellation.cancelled() => return Err(HttpError::Cancelled),
            _ = tokio::time::sleep_until(wait_until.min(entry.deadline)) => return Ok(ReadResult::Pending),
            chunk = receive.port.chunks.recv() => chunk,
        };
        match chunk {
            Some(Ok(bytes)) => return Ok(ReadResult::Chunk(bytes)),
            Some(Err(error)) => {
                self.cancel();
                return Err(error);
            }
            None => {}
        }
        let mut completed = entry.completed.clone();
        loop {
            let outcome = *completed.borrow_and_update();
            if let Some(outcome) = outcome {
                self.cancel();
                return outcome.map(|()| ReadResult::End);
            }
            tokio::select! { biased;
                _ = super::super::cancelled(&mut stopped) => return Err(HttpError::Cancelled),
                _ = cancellation.cancelled() => return Err(HttpError::Cancelled),
                _ = tokio::time::sleep_until(wait_until.min(entry.deadline)) => return Ok(ReadResult::Pending),
                changed = completed.changed() => if changed.is_err() {
                    self.cancel();
                    return Err(HttpError::Network);
                },
            }
        }
    }
}
