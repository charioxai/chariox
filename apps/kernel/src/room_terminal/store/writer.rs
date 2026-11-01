//! One bounded artifact writer; directory sync settles each ready batch.
use crate::error::DaemonError;
use crate::runtime::request_metrics::RequestObservation;
use std::fs::{self, File};
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;

const QUEUE_LIMIT: usize = 64;
const BATCH_LIMIT: usize = 32;

type Reply = Result<(), String>;

pub(super) enum Operation {
    #[cfg(test)]
    Pause {
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    },
    Save {
        id: String,
        body: Vec<u8>,
    },
    Remove {
        id: String,
    },
}
struct Job {
    operation: Operation,
    reply: mpsc::Sender<Reply>,
    observation: Option<RequestObservation>,
}

pub(crate) struct PendingWrite(mpsc::Receiver<Reply>);
impl PendingWrite {
    pub(crate) fn wait(self) -> Result<(), DaemonError> {
        crate::durable_state::await_writer_response(self.0)
            .map_err(|_| failed("terminal writer stopped before settlement"))?
            .map_err(failed)
    }
    pub(crate) fn try_result(&self) -> Option<Result<(), DaemonError>> {
        match self.0.try_recv() {
            Ok(result) => Some(result.map_err(failed)),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err(failed("terminal writer stopped before settlement")))
            }
        }
    }
}

pub(super) struct Writer {
    sender: Mutex<Option<mpsc::SyncSender<Job>>>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
}
impl Writer {
    pub(super) fn new(root: PathBuf) -> Result<Arc<Self>, DaemonError> {
        let (sender, receiver) = mpsc::sync_channel(QUEUE_LIMIT);
        let worker = std::thread::Builder::new()
            .name("chariox-terminal-writer".into())
            .stack_size(256 * 1024)
            .spawn(move || run(root, receiver))
            .map_err(|error| failed(error.to_string()))?;
        Ok(Arc::new(Self {
            sender: Mutex::new(Some(sender)),
            worker: Mutex::new(Some(worker)),
        }))
    }
    pub(super) fn submit(
        &self,
        operation: Operation,
        wait_for_capacity: bool,
    ) -> Result<PendingWrite, DaemonError> {
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .ok_or_else(|| failed("terminal writer has stopped"))?;
        let (reply, receiver) = mpsc::channel();
        let observation = RequestObservation::current();
        let job = Job {
            operation,
            reply,
            observation: observation.clone(),
        };
        if wait_for_capacity {
            let started = Instant::now();
            let submit = || {
                sender
                    .send(job)
                    .map_err(|_| failed("terminal writer has stopped"))
            };
            if tokio::runtime::Handle::try_current().is_ok_and(|handle| {
                handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread
            }) {
                tokio::task::block_in_place(submit)?;
            } else {
                submit()?;
            }
            if let Some(observation) = observation {
                observation.record_durable_wait(started.elapsed());
            }
        } else {
            sender.try_send(job).map_err(|error| {
                failed(format!(
                    "terminal writer is busy; retry the operation: {error}"
                ))
            })?;
        }
        Ok(PendingWrite(receiver))
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        self.sender
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(worker) = self
            .worker
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            let _ = worker.join();
        }
    }
}

fn failed(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "room terminal persistence",
        message: message.into(),
    }
}

fn run(root: PathBuf, receiver: mpsc::Receiver<Job>) {
    while let Ok(first) = receiver.recv() {
        let mut batch = vec![first];
        while batch.len() < BATCH_LIMIT {
            match receiver.try_recv() {
                Ok(job) => batch.push(job),
                Err(_) => break,
            }
        }
        let started = Instant::now();
        let mut sync_us = 0;
        let mut results = Vec::new();
        for job in &batch {
            results.push(apply(&root, &job.operation, &mut sync_us));
        }
        let directory_result = sync_directory(&root, &mut sync_us);
        for (job, result) in batch.into_iter().zip(results) {
            if let Some(observation) = job.observation {
                observation.record_fsync(sync_us);
                observation.record_durable_commit(started.elapsed());
            }
            let result = result.and_then(|_| directory_result.clone());
            if let Err(mpsc::SendError(Err(message))) = job.reply.send(result) {
                crate::logging::error_with_fields(
                    "room_terminal.persist",
                    "terminal persistence failed after its observer closed",
                    serde_json::json!({"error": message}),
                );
            }
        }
    }
}

fn apply(root: &std::path::Path, operation: &Operation, sync_us: &mut u64) -> Reply {
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    match operation {
        #[cfg(test)]
        Operation::Pause { entered, release } => {
            let _ = entered.send(());
            release.recv().map_err(|error| error.to_string())
        }
        Operation::Save { id, body } => {
            let path = root.join(format!("{id}.json"));
            let temporary = path.with_extension("json.tmp");
            let mut file = super::open_private(&temporary).map_err(|error| error.to_string())?;
            use std::io::Write;
            file.write_all(body).map_err(|error| error.to_string())?;
            let started = Instant::now();
            let result = file.sync_all().map_err(|error| error.to_string());
            *sync_us += started.elapsed().as_micros().min(u64::MAX as u128) as u64;
            result?;
            fs::rename(temporary, path).map_err(|error| error.to_string())
        }
        Operation::Remove { id } => match fs::remove_file(root.join(format!("{id}.json"))) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        },
    }
}

fn sync_directory(root: &std::path::Path, sync_us: &mut u64) -> Reply {
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        let directory = File::open(root).map_err(|error| error.to_string())?;
        let started = Instant::now();
        let result = directory.sync_all().map_err(|error| error.to_string());
        *sync_us += started.elapsed().as_micros().min(u64::MAX as u128) as u64;
        result
    }
    #[cfg(not(unix))]
    {
        let _ = (root, sync_us);
        Ok(())
    }
}
