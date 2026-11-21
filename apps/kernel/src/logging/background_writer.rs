use std::sync::mpsc::{self, Sender, SyncSender};
use std::thread::{self, JoinHandle};

const QUEUED_RECORDS: usize = 1024;

pub(super) enum WriteAction {
    Line(String, bool),
    Flush,
}

struct Request {
    action: WriteAction,
    completed: Option<Sender<()>>,
}

pub(super) struct BackgroundWriter {
    sender: Option<SyncSender<Request>>,
    worker: Option<JoinHandle<()>>,
}

impl BackgroundWriter {
    pub(super) fn start(
        mut write: impl FnMut(WriteAction) + Send + 'static,
    ) -> std::io::Result<Self> {
        let (sender, requests) = mpsc::sync_channel::<Request>(QUEUED_RECORDS);
        let worker = thread::Builder::new()
            .name("chariox-log-writer".into())
            .spawn(move || {
                while let Ok(request) = requests.recv() {
                    write(request.action);
                    if let Some(completed) = request.completed {
                        let _ = completed.send(());
                    }
                }
                write(WriteAction::Flush);
            })?;
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
        })
    }

    pub(super) fn write(&self, line: String, flush: bool) {
        self.submit(WriteAction::Line(line, flush), flush);
    }

    pub(super) fn flush(&self) {
        self.submit(WriteAction::Flush, true);
    }

    fn submit(&self, action: WriteAction, wait: bool) {
        let Some(sender) = &self.sender else { return };
        let (completed, completion) = if wait {
            let (sender, receiver) = mpsc::channel();
            (Some(sender), Some(receiver))
        } else {
            (None, None)
        };
        if sender.send(Request { action, completed }).is_ok() {
            if let Some(completion) = completion {
                let _ = completion.recv();
            }
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
    use std::sync::{Arc, Mutex};
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
                WriteAction::Flush => {}
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
        for index in 0..QUEUED_RECORDS {
            assert_eq!(records[index + 1], index.to_string());
        }
        assert!(!records.iter().any(|record| record == "last"));
        assert_eq!(records.last().unwrap(), "flushed");
    }
}
