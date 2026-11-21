use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const QUEUED_RECORDS: usize = 1024;
const SHUTDOWN_FLUSH_BUDGET: Duration = Duration::from_secs(2);

pub(super) enum WriteAction {
    Line(String, bool),
    Dropped(u64),
    Flush,
}

enum Request {
    Record(String, bool),
    Flush(Sender<()>),
}

pub(super) struct BackgroundWriter {
    sender: Option<SyncSender<Request>>,
    dropped: Arc<AtomicU64>,
    worker: Option<JoinHandle<()>>,
}

impl BackgroundWriter {
    pub(super) fn start(
        mut write: impl FnMut(WriteAction) + Send + 'static,
    ) -> std::io::Result<Self> {
        let (sender, requests) = mpsc::sync_channel(QUEUED_RECORDS);
        let dropped = Arc::new(AtomicU64::new(0));
        let worker_dropped = Arc::clone(&dropped);
        let worker = thread::Builder::new()
            .name("chariox-log-writer".into())
            .spawn(move || {
                while let Ok(request) = requests.recv() {
                    let count = worker_dropped.swap(0, Ordering::Relaxed);
                    if count != 0 {
                        write(WriteAction::Dropped(count));
                    }
                    match request {
                        Request::Record(line, flush) => write(WriteAction::Line(line, flush)),
                        Request::Flush(completed) => {
                            write(WriteAction::Flush);
                            let _ = completed.send(());
                        }
                    }
                }
                write(WriteAction::Flush);
            })?;
        Ok(Self {
            sender: Some(sender),
            dropped,
            worker: Some(worker),
        })
    }

    pub(super) fn write(&self, line: String, flush: bool) {
        let Some(sender) = &self.sender else { return };
        // Diagnostics are bounded. Disk latency must never become relay latency.
        if let Err(TrySendError::Full(_)) = sender.try_send(Request::Record(line, flush)) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(super) fn flush(&self) -> bool {
        let Some(sender) = &self.sender else {
            return false;
        };
        let (completed, completion) = mpsc::channel();
        let mut request = Request::Flush(completed);
        let deadline = Instant::now() + SHUTDOWN_FLUSH_BUDGET;
        loop {
            match sender.try_send(request) {
                Ok(()) => {
                    return completion
                        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                        .is_ok();
                }
                Err(TrySendError::Disconnected(_)) => return false,
                Err(TrySendError::Full(pending)) => request = pending,
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for BackgroundWriter {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Duration;

    #[test]
    fn warning_does_not_wait_for_a_busy_sink() {
        let records = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&records);
        let (started, starting) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let writer = Arc::new(
            BackgroundWriter::start(move |action| match action {
                WriteAction::Line(line, flush) => {
                    if line == "info" {
                        started.send(()).unwrap();
                        released.recv().unwrap();
                    }
                    observed.lock().unwrap().push((line, flush));
                }
                WriteAction::Flush | WriteAction::Dropped(_) => {}
            })
            .unwrap(),
        );
        writer.write("info".into(), false);
        starting.recv().unwrap();
        let warning_writer = Arc::clone(&writer);
        let (submitted, submission) = mpsc::channel();
        let (finished, completion) = mpsc::channel();
        let warning = thread::spawn(move || {
            submitted.send(()).unwrap();
            warning_writer.write("warning".into(), true);
            finished.send(()).unwrap();
        });
        submission.recv().unwrap();
        let completed = completion.recv_timeout(Duration::from_secs(1)).is_ok();
        release.send(()).unwrap();
        warning.join().unwrap();
        writer.flush();
        assert!(completed, "warning waited for the disk writer");
        assert_eq!(
            *records.lock().unwrap(),
            vec![("info".into(), false), ("warning".into(), true)]
        );
    }

    #[test]
    fn full_log_queue_does_not_block_producers() {
        let records = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&records);
        let (started, starting) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let writer = Arc::new(
            BackgroundWriter::start(move |action| match action {
                WriteAction::Line(line, _) => {
                    if line == "first" {
                        started.send(()).unwrap();
                        released.recv().unwrap();
                    }
                    observed.lock().unwrap().push(line);
                }
                WriteAction::Flush => observed.lock().unwrap().push("flushed".into()),
                WriteAction::Dropped(count) => {
                    observed.lock().unwrap().push(format!("dropped {count}"))
                }
            })
            .unwrap(),
        );
        writer.write("first".into(), false);
        starting.recv().unwrap();
        for index in 0..QUEUED_RECORDS {
            writer.write(index.to_string(), false);
        }
        let overflow_writer = Arc::clone(&writer);
        let (submitted, submission) = mpsc::channel();
        let (finished, completion) = mpsc::channel();
        let overflow = thread::spawn(move || {
            submitted.send(()).unwrap();
            overflow_writer.write("last".into(), false);
            finished.send(()).unwrap();
        });
        submission.recv().unwrap();
        let completed = completion.recv_timeout(Duration::from_secs(1)).is_ok();
        release.send(()).unwrap();
        overflow.join().unwrap();
        drop(writer);
        let records = records.lock().unwrap();
        assert!(completed, "a full log queue blocked the producer");
        assert_eq!(records[0], "first");
        assert!(records.iter().any(|record| record == "dropped 1"));
        assert_eq!(
            records
                .iter()
                .filter_map(|record| record.parse().ok())
                .collect::<Vec<usize>>(),
            (0..QUEUED_RECORDS).collect::<Vec<_>>()
        );
        assert!(!records.iter().any(|record| record == "last"));
        assert_eq!(records.last().unwrap(), "flushed");
    }
}
