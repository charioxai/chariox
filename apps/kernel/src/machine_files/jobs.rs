//! File transfers between machines as kernel-owned jobs. The kernel that is to hold the copy runs
//! the job: it reads the source slice by slice (from this machine or over the relay), stages the
//! copy beside its destination and publishes it only when it is whole and its digest matches.
//! The job outlives whoever asked for it, and a restart of the kernel: its record is kept on disk,
//! its state stays readable after it ends, and a job the kernel stopped under continues from the
//! bytes it had staged.
use std::collections::{BTreeSet, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use futures_util::StreamExt;
use sha2::{Digest, Sha256};

use super::records::{read_records, Pending, Record, Recorder};
use super::{FileSource, Machine};
use crate::agent_file_copy::{
    patiently, pull, CancelToken, Control, Copied, CopyOptions, LinkError, RemoteSource, CANCELED,
    UNREACHABLE_ADVICE,
};
use crate::local::{
    FileTransferConflict, FileTransferEndpoint, FileTransferJob, FileTransferState,
    KernelDirectoryEntry, KernelFileKind, StartFileTransferRequest,
};

/// Files of a folder copied at once.
const FILES_IN_FLIGHT: usize = 4;
/// Finished jobs the kernel remembers.
const FINISHED_KEPT: usize = 64;
/// How long a copy waits for a link that dropped before it gives up (and can be resumed).
#[cfg(not(test))]
const LINK_WINDOW: Duration = Duration::from_secs(15 * 60);
#[cfg(test)]
const LINK_WINDOW: Duration = Duration::from_secs(3);
/// How often the record of a running job is written.
#[cfg(not(test))]
const SAVE_EVERY: Duration = Duration::from_secs(1);
#[cfg(test)]
const SAVE_EVERY: Duration = Duration::from_millis(100);
/// The hidden names a transfer stages under, beside its destination.
const STAGING_PREFIX: &str = ".chariox-copy-";

struct Entry {
    job: Arc<Mutex<FileTransferJob>>,
    request: StartFileTransferRequest,
    cancel: tokio::sync::watch::Sender<bool>,
    token: CancelToken,
}

/// A job a kernel stopped under, which the next life of the kernel continues.
pub(crate) struct Interrupted {
    pub(crate) job_id: String,
    pub(crate) request: StartFileTransferRequest,
}

#[derive(Default)]
pub(crate) struct TransferJobs {
    entries: Mutex<VecDeque<Entry>>,
    /// The writer of the records, once the kernel has opened their folder.
    store: Arc<OnceLock<Recorder>>,
    /// The destinations transfers are writing to, so no two write to one.
    reserved: Arc<Mutex<BTreeSet<PathBuf>>>,
    /// The running transfers, which a test stops the way a dying process does.
    #[cfg(test)]
    runs: Mutex<Vec<tokio::task::AbortHandle>>,
}

impl TransferJobs {
    /// Opens the folder the jobs are kept in: the jobs of an earlier life of the kernel are listed
    /// again, and those it stopped under are returned to be resumed. A job whose source was being
    /// deleted ends as failed, since repeating its deletes blindly could remove what was not copied.
    pub(crate) fn attach(self: &Arc<Self>, dir: PathBuf) -> Vec<Interrupted> {
        let recorder = self
            .store
            .get_or_init(|| Recorder::open(dir.clone()))
            .clone();
        let mut records = read_records(&dir);
        records.sort_by_key(|record| record.job.started_ms);
        let mut interrupted = Vec::new();
        let mut entries = self.entries.lock().expect("transfer jobs");
        for mut record in records {
            let known = |entries: &VecDeque<Entry>| {
                entries
                    .iter()
                    .any(|entry| entry.job.lock().expect("job state").job_id == record.job.job_id)
            };
            if known(&entries) {
                continue;
            }
            if record.job.state == FileTransferState::Finishing {
                record.job.state = FileTransferState::Failed;
                record.job.current = None;
                record.job.finished_ms = Some(crate::session::unix_epoch_ms());
                record.job.error = Some(format!(
                    "The copy is complete and verified, but the kernel restarted while `{}` was being deleted from the source, so it may be partly deleted. Check it and delete what is left yourself",
                    record.job.source.path
                ));
            } else if !record.job.state.is_finished() {
                interrupted.push(Interrupted {
                    job_id: record.job.job_id.clone(),
                    request: record.request.clone(),
                });
            }
            let (cancel, token) = CancelToken::new();
            entries.push_back(Entry {
                job: Arc::new(Mutex::new(record.job)),
                request: record.request,
                cancel,
                token,
            });
        }
        // What ran before the folder was opened is written down too.
        let removed = trim(&mut entries);
        for entry in entries.iter() {
            drop(entry.save(&recorder));
        }
        recorder.forget(removed);
        interrupted
    }

    /// Continues a job the kernel stopped under, from what it had staged.
    pub(crate) fn resume(
        self: &Arc<Self>,
        interrupted: &Interrupted,
        source: FileSource,
        machine: Machine,
    ) {
        let entries = self.entries.lock().expect("transfer jobs");
        let Some(entry) = entries
            .iter()
            .find(|entry| entry.job.lock().expect("job state").job_id == interrupted.job_id)
        else {
            return;
        };
        {
            let mut job = entry.job.lock().expect("job state");
            if job.state.is_finished() {
                return;
            }
            // The run counts everything again, the staged bytes included.
            job.state = FileTransferState::Scanning;
            job.bytes_done = 0;
            job.files_done = 0;
            job.skipped = 0;
            job.current = None;
        }
        self.spawn_run(entry, source, machine);
    }

    /// Ends a job the kernel could not continue, with the reason.
    pub(crate) fn fail(&self, job_id: &str, reason: String) {
        let entries = self.entries.lock().expect("transfer jobs");
        let Some(entry) = entries
            .iter()
            .find(|entry| entry.job.lock().expect("job state").job_id == job_id)
        else {
            return;
        };
        let mut job = entry.job.lock().expect("job state");
        if job.state.is_finished() {
            return;
        }
        job.state = FileTransferState::Failed;
        job.current = None;
        job.finished_ms = Some(crate::session::unix_epoch_ms());
        job.error = Some(reason);
        if let Some(recorder) = self.store.get() {
            drop(recorder.save(Record {
                job: job.clone(),
                request: entry.request.clone(),
            }));
        }
    }

    /// Starts the transfer and answers how it stands; the work goes on after the caller is gone.
    pub(crate) fn start(
        self: &Arc<Self>,
        request: StartFileTransferRequest,
        source: FileSource,
        machine: Machine,
    ) -> FileTransferJob {
        let (cancel, token) = CancelToken::new();
        let job = Arc::new(Mutex::new(FileTransferJob {
            job_id: format!("transfer-{:016x}", rand::random::<u64>()),
            state: FileTransferState::Scanning,
            source: request.source.clone(),
            destination: String::new(),
            delete_source: request.delete_source,
            bytes_done: 0,
            bytes_total: 0,
            files_done: 0,
            files_total: 0,
            skipped: 0,
            current: None,
            sha256: None,
            error: None,
            started_ms: crate::session::unix_epoch_ms(),
            finished_ms: None,
        }));
        let first = job.lock().expect("job state").clone();
        let mut entries = self.entries.lock().expect("transfer jobs");
        entries.push_back(Entry {
            job,
            request,
            cancel,
            token,
        });
        let removed = trim(&mut entries);
        let entry = entries.back().expect("the job just added");
        if let Some(recorder) = self.store.get() {
            recorder.forget(removed);
            drop(entry.save(recorder));
        }
        self.spawn_run(entry, source, machine);
        first
    }

    fn spawn_run(&self, entry: &Entry, source: FileSource, machine: Machine) {
        let (job, request, token) = (
            entry.job.clone(),
            entry.request.clone(),
            entry.token.clone(),
        );
        let (store, reserved) = (self.store.clone(), self.reserved.clone());
        let run = tokio::spawn(async move {
            let run = Run {
                job: job.clone(),
                request: request.clone(),
                token,
                source,
                machine,
                reserved,
                store: store.clone(),
            };
            let outcome = {
                let work = run.transfer(&request);
                tokio::pin!(work);
                let mut tick = tokio::time::interval(SAVE_EVERY);
                let mut saved: Option<FileTransferJob> = None;
                loop {
                    tokio::select! {
                        outcome = &mut work => break outcome,
                        _ = tick.tick() => {
                            let now = job.lock().expect("job state");
                            if saved.as_ref() != Some(&*now) {
                                if let Some(recorder) = store.get() {
                                    drop(recorder.save(Record { job: now.clone(), request: request.clone() }));
                                }
                                saved = Some(now.clone());
                            }
                        }
                    }
                }
            };
            let recorded = {
                let mut job = job.lock().expect("job state");
                job.current = None;
                job.finished_ms = Some(crate::session::unix_epoch_ms());
                match outcome {
                    Ok(()) => job.state = FileTransferState::Done,
                    Err(reason) if reason == CANCELED => {
                        job.state = FileTransferState::Canceled;
                        job.error = Some(reason);
                    }
                    Err(reason) => {
                        job.state = FileTransferState::Failed;
                        job.error = Some(reason);
                    }
                }
                store.get().map(|recorder| {
                    recorder.save(Record {
                        job: job.clone(),
                        request: request.clone(),
                    })
                })
            };
            if let Some(recorded) = recorded {
                let _ = recorded.recorded().await;
            }
        });
        #[cfg(test)]
        self.runs
            .lock()
            .expect("running transfers")
            .push(run.abort_handle());
        #[cfg(not(test))]
        drop(run);
    }

    /// Stops every running transfer where it is, without any of its cleanup: a process that dies.
    #[cfg(test)]
    pub(crate) fn kill_runs(&self) {
        for run in self.runs.lock().expect("running transfers").drain(..) {
            run.abort();
        }
    }

    pub(crate) fn get(&self, job_id: &str) -> Option<FileTransferJob> {
        self.entries
            .lock()
            .expect("transfer jobs")
            .iter()
            .map(|entry| entry.job.lock().expect("job state").clone())
            .find(|job| job.job_id == job_id)
    }

    /// Newest first.
    pub(crate) fn list(&self) -> Vec<FileTransferJob> {
        self.entries
            .lock()
            .expect("transfer jobs")
            .iter()
            .rev()
            .map(|entry| entry.job.lock().expect("job state").clone())
            .collect()
    }

    /// Stops the transfer; the answer is how it stands now, which settles a moment later.
    pub(crate) fn cancel(&self, job_id: &str) -> Option<FileTransferJob> {
        let entries = self.entries.lock().expect("transfer jobs");
        let entry = entries
            .iter()
            .find(|entry| entry.job.lock().expect("job state").job_id == job_id)?;
        entry.cancel.send_replace(true);
        let job = entry.job.lock().expect("job state").clone();
        Some(job)
    }
}

/// One transfer being carried out.
struct Run {
    job: Arc<Mutex<FileTransferJob>>,
    request: StartFileTransferRequest,
    token: CancelToken,
    source: FileSource,
    machine: Machine,
    reserved: Arc<Mutex<BTreeSet<PathBuf>>>,
    store: Arc<OnceLock<Recorder>>,
}

/// A file or folder of the source tree, by path below the transferred item.
#[derive(Clone)]
struct Item {
    relative: String,
    size: u64,
    mode: u32,
}

impl Run {
    /// `request` on the source, repeated while the link is down, for as long as a copy would wait.
    async fn linked<T, F, Fut>(&self, request: F) -> Result<T, String>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T, LinkError>>,
    {
        let control = Control {
            cancel: self.token.clone(),
            link_window: Some(LINK_WINDOW),
            ..Default::default()
        };
        patiently(&self.source.peer(), &control, request).await
    }

    /// Puts the deleting phase on record, and waits until it is on disk, before the first source
    /// file is deleted: a restart then knows a move that began deleting is not to be repeated.
    async fn begin_deleting(&self) -> Result<(), String> {
        let recorded = {
            let mut job = self.job.lock().expect("job state");
            job.state = FileTransferState::Finishing;
            job.current = None;
            self.store.get().map(|recorder| {
                recorder.save(Record {
                    job: job.clone(),
                    request: self.request.clone(),
                })
            })
        };
        match recorded {
            Some(recorded) => recorded.recorded().await.map_err(|reason| {
                format!("The copy is complete and verified, but the move could not be recorded ({reason}), so the source was not deleted. Delete it yourself")
            }),
            None => Ok(()),
        }
    }

    fn update(&self, change: impl FnOnce(&mut FileTransferJob)) {
        change(&mut self.job.lock().expect("job state"));
    }

    async fn transfer(&self, request: &StartFileTransferRequest) -> Result<(), String> {
        let source_path = request.source.path.clone();
        let stat = self.linked(|| self.source.stat(&source_path)).await?;
        if !stat.exists {
            return Err(format!(
                "`{source_path}` does not exist on the source machine"
            ));
        }
        let name = match &request.name {
            Some(name) => name.clone(),
            None => Path::new(&source_path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .ok_or_else(|| format!("`{source_path}` has no name; say which name to use"))?,
        };
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.contains('/')
            || name.starts_with(STAGING_PREFIX)
        {
            return Err(format!("`{name}` is not a name a file can have here"));
        }
        let folder = Machine::absolute(&request.destination_folder)?;
        self.machine.guard().check(&folder.join(&name))?;
        // A job continued after a restart lands where it had settled to land.
        let kept = self.job.lock().expect("job state").destination.clone();
        let settled = if kept.is_empty() {
            self.settle_name(&folder, &name, request.conflict)?
        } else {
            Some(self.reserve(Path::new(&kept))?)
        };
        let Some(destination) = settled else {
            self.update(|job| job.skipped += 1);
            return Ok(());
        };
        // Only the replace choice puts a copy over what is there.
        let replacing = request.conflict == FileTransferConflict::Replace
            && std::fs::symlink_metadata(&destination.path).is_ok();
        if replacing {
            self.machine.guard().check_replace(&destination.path)?;
        }
        self.update(|job| job.destination = destination.path.display().to_string());
        match stat.kind {
            Some(KernelFileKind::File) => {
                self.transfer_file(
                    request,
                    &source_path,
                    &destination.path,
                    stat.size.unwrap_or(0),
                    stat.mode,
                    replacing,
                )
                .await
            }
            Some(KernelFileKind::Directory) => {
                self.transfer_folder(
                    request,
                    &source_path,
                    &destination.path,
                    stat.mode,
                    replacing,
                )
                .await
            }
            _ => Err(format!(
                "`{source_path}` is not a file or a folder; links and special files are not copied"
            )),
        }
    }

    /// Where the item lands under `conflict`, or `None` when the name is taken and is to be left.
    /// The name is held until the transfer ends, so no other transfer writes to it meanwhile.
    fn settle_name(
        &self,
        folder: &Path,
        name: &str,
        conflict: FileTransferConflict,
    ) -> Result<Option<Settled>, String> {
        let folder = resolved(folder);
        let wanted = folder.join(name);
        if std::fs::symlink_metadata(&wanted).is_err() {
            match self.reserve(&wanted) {
                Ok(held) => return Ok(Some(held)),
                // Another transfer is bringing that name: keep_both moves to the next free one.
                Err(error) if conflict != FileTransferConflict::KeepBoth => return Err(error),
                Err(_) => {}
            }
        }
        match conflict {
            FileTransferConflict::Fail => Err(format!(
                "`{}` already exists; choose replace, keep_both or skip",
                wanted.display()
            )),
            FileTransferConflict::Skip => Ok(None),
            FileTransferConflict::Replace => self.reserve(&wanted).map(Some),
            FileTransferConflict::KeepBoth => {
                let path = Path::new(name);
                let stem = path.file_stem().map_or_else(
                    || name.to_owned(),
                    |stem| stem.to_string_lossy().into_owned(),
                );
                let extension = path
                    .extension()
                    .map(|extension| format!(".{}", extension.to_string_lossy()));
                (2..)
                    .map(|n| {
                        folder.join(format!(
                            "{stem} ({n}){}",
                            extension.as_deref().unwrap_or("")
                        ))
                    })
                    .filter(|candidate| std::fs::symlink_metadata(candidate).is_err())
                    .find_map(|candidate| self.reserve(&candidate).ok())
                    .map(Some)
                    .ok_or_else(|| "no free name".to_owned())
            }
        }
    }

    /// Holds the place `path` names for this transfer, or says another transfer is writing there
    /// or in a folder around it. The place is found through the links of its folders, so two
    /// spellings of one place are one place.
    fn reserve(&self, path: &Path) -> Result<Settled, String> {
        let path = match (path.parent(), path.file_name()) {
            (Some(parent), Some(name)) => resolved(parent).join(name),
            _ => path.to_owned(),
        };
        let mut reserved = self.reserved.lock().expect("reserved destinations");
        if let Some(held) = reserved
            .iter()
            .find(|held| held.starts_with(&path) || path.starts_with(held))
        {
            return Err(format!(
                "another transfer is already writing `{}`; wait for it to end, or choose keep_both",
                held.display()
            ));
        }
        reserved.insert(path.clone());
        Ok(Settled {
            path,
            reserved: self.reserved.clone(),
        })
    }

    fn control(&self, progress: Arc<dyn Fn(u64) + Send + Sync>) -> Control {
        Control {
            cancel: self.token.clone(),
            progress: Some(progress),
            link_window: Some(LINK_WINDOW),
            resume: true,
        }
    }

    async fn transfer_file(
        &self,
        request: &StartFileTransferRequest,
        source_path: &str,
        destination: &Path,
        size: u64,
        mode: Option<u32>,
        replacing: bool,
    ) -> Result<(), String> {
        self.update(|job| {
            job.state = FileTransferState::Copying;
            job.bytes_total = size;
            job.files_total = 1;
            job.current = Some(source_path.to_owned());
        });
        let job = self.job.clone();
        let progress = Arc::new(move |landed: u64| {
            job.lock().expect("job state").bytes_done = landed;
        });
        let options = CopyOptions {
            overwrite: replacing,
            control: self.control(progress),
            ..Default::default()
        };
        let destination = destination.display().to_string();
        let copied = pull(
            &self.source,
            source_path,
            &self.machine,
            &destination,
            &options,
        )
        .await?;
        restore_mode(Path::new(&destination), mode)?;
        self.update(|job| {
            job.files_done = 1;
            job.bytes_done = copied.bytes;
            job.sha256 = Some(copied.sha256.clone());
        });
        if request.delete_source {
            self.begin_deleting().await?;
            self.linked(|| self.source.delete(source_path, Some(&copied.revision)))
                .await
                .map_err(|reason| delete_failed(source_path, &reason))?;
        }
        Ok(())
    }

    async fn transfer_folder(
        &self,
        request: &StartFileTransferRequest,
        source_path: &str,
        destination: &Path,
        root_mode: Option<u32>,
        replacing: bool,
    ) -> Result<(), String> {
        let (folders, files) = self.scan(source_path).await?;
        let stage = folder_stage(&request.source, destination);
        let outcome = self
            .copy_folder(
                request,
                &stage,
                destination,
                root_mode,
                replacing,
                &folders,
                &files,
            )
            .await;
        match &outcome {
            // A link that stayed down leaves the staged files, so the same transfer continues.
            Err(reason) if reason.contains("stayed unreachable") => {}
            Err(_) => {
                let stage = stage.clone();
                let _ = tokio::task::spawn_blocking(move || std::fs::remove_dir_all(stage)).await;
            }
            Ok(()) => {}
        }
        outcome
    }

    #[allow(clippy::too_many_arguments)]
    async fn copy_folder(
        &self,
        request: &StartFileTransferRequest,
        stage: &Path,
        destination: &Path,
        root_mode: Option<u32>,
        replacing: bool,
        folders: &[Item],
        files: &[Item],
    ) -> Result<(), String> {
        let source_path = request.source.path.as_str();
        let (staged, wanted_folders, wanted_files) =
            (stage.to_owned(), folders.to_vec(), files.to_vec());
        tokio::task::spawn_blocking(move || prepare_stage(&staged, &wanted_folders, &wanted_files))
            .await
            .map_err(|_| "The transfer stopped unexpectedly; nothing was published".to_owned())??;
        self.update(|job| job.state = FileTransferState::Copying);
        let done = Arc::new(AtomicU64::new(0));
        let copied: Vec<(String, Copied)> = {
            let results = futures_util::stream::iter(files.to_vec())
                .map(|item| {
                    let done = done.clone();
                    async move { self.copy_item(source_path, stage, &item, &done).await }
                })
                .buffer_unordered(FILES_IN_FLIGHT);
            tokio::pin!(results);
            let mut copied = Vec::new();
            while let Some(result) = results.next().await {
                copied.push(result?);
            }
            copied
        };
        if self.token.is_raised() {
            return Err(CANCELED.to_owned());
        }
        // Deepest first, so a folder that is closed to its owner is closed after its content is set.
        let mut by_depth = folders.to_vec();
        by_depth.sort_by_key(|item| std::cmp::Reverse(item.relative.matches('/').count()));
        let (staged, target) = (stage.to_owned(), destination.to_owned());
        tokio::task::spawn_blocking(move || {
            for item in &by_depth {
                restore_mode(&staged.join(&item.relative), Some(item.mode))?;
            }
            restore_mode(&staged, root_mode)?;
            publish_folder(&staged, &target, replacing)
        })
        .await
        .map_err(|_| "The transfer stopped unexpectedly; nothing was published".to_owned())??;
        if request.delete_source {
            self.begin_deleting().await?;
            for (relative, copied) in &copied {
                let path = format!("{source_path}/{relative}");
                self.linked(|| self.source.delete(&path, Some(&copied.revision)))
                    .await
                    .map_err(|reason| delete_failed(&path, &reason))?;
            }
            // Deepest folders first, so each is empty by the time it is removed. A folder that
            // still holds what was not copied (a link) stays with it.
            let kept_some = self.job.lock().expect("job state").skipped > 0;
            let mut deepest: Vec<&Item> = folders.iter().collect();
            deepest.sort_by_key(|item| std::cmp::Reverse(item.relative.matches('/').count()));
            let paths = deepest
                .iter()
                .map(|item| format!("{source_path}/{}", item.relative))
                .chain(std::iter::once(source_path.to_owned()));
            for path in paths {
                match self.linked(|| self.source.delete(&path, None)).await {
                    Err(reason) if !kept_some => return Err(delete_failed(&path, &reason)),
                    _ => {}
                }
            }
        }
        Ok(())
    }

    async fn copy_item(
        &self,
        source_path: &str,
        stage: &Path,
        item: &Item,
        done: &Arc<AtomicU64>,
    ) -> Result<(String, Copied), String> {
        let source = format!("{source_path}/{}", item.relative);
        let target = stage.join(&item.relative).display().to_string();
        self.update(|job| job.current = Some(source.clone()));
        let landed_before = Arc::new(AtomicU64::new(0));
        let (job, done_total, last) = (self.job.clone(), done.clone(), landed_before.clone());
        let progress = Arc::new(move |landed: u64| {
            let previous = last.swap(landed, Ordering::SeqCst);
            let total = done_total.fetch_add(landed.saturating_sub(previous), Ordering::SeqCst)
                + landed.saturating_sub(previous);
            job.lock().expect("job state").bytes_done = total;
        });
        let mut options = CopyOptions {
            overwrite: false,
            control: self.control(progress),
            ..Default::default()
        };
        // A staged folder left by an earlier try may hold the file already; a different one is replaced.
        let copied = match pull(&self.source, &source, &self.machine, &target, &options).await {
            Err(reason) if reason.contains("already exists") => {
                options.overwrite = true;
                pull(&self.source, &source, &self.machine, &target, &options).await?
            }
            other => other?,
        };
        restore_mode(Path::new(&target), Some(item.mode))?;
        let leftover = item
            .size
            .saturating_sub(landed_before.load(Ordering::SeqCst));
        done.fetch_add(leftover, Ordering::SeqCst);
        self.update(|job| {
            job.files_done += 1;
            job.bytes_done = done.load(Ordering::SeqCst);
        });
        Ok((item.relative.clone(), copied))
    }

    /// Reads the source folder's tree: its folders and files, links and special files counted as skipped.
    async fn scan(&self, source_path: &str) -> Result<(Vec<Item>, Vec<Item>), String> {
        let mut folders = Vec::new();
        let mut files = Vec::new();
        let mut waiting = VecDeque::from([String::new()]);
        while let Some(relative) = waiting.pop_front() {
            let path = if relative.is_empty() {
                source_path.to_owned()
            } else {
                format!("{source_path}/{relative}")
            };
            let mut offset = 0;
            loop {
                let (entries, more) = self.linked(|| self.source.list(&path, offset)).await?;
                offset += entries.len() as u64;
                for entry in entries {
                    self.sort_entry(&relative, entry, &mut folders, &mut files, &mut waiting);
                }
                if !more {
                    break;
                }
            }
        }
        self.update(|job| {
            job.files_total = files.len() as u64;
            job.bytes_total = files.iter().map(|file| file.size).sum();
        });
        Ok((folders, files))
    }

    fn sort_entry(
        &self,
        parent: &str,
        entry: KernelDirectoryEntry,
        folders: &mut Vec<Item>,
        files: &mut Vec<Item>,
        waiting: &mut VecDeque<String>,
    ) {
        if entry.name.starts_with(STAGING_PREFIX) {
            return;
        }
        let relative = if parent.is_empty() {
            entry.name.clone()
        } else {
            format!("{parent}/{}", entry.name)
        };
        let item = Item {
            relative: relative.clone(),
            size: entry.size,
            mode: entry.mode,
        };
        match entry.kind {
            KernelFileKind::File => files.push(item),
            KernelFileKind::Directory => {
                folders.push(item);
                waiting.push_back(relative);
            }
            KernelFileKind::Symlink | KernelFileKind::Other => {
                self.update(|job| job.skipped += 1);
            }
        }
    }
}

/// A destination held by a transfer; it is released when the transfer lets go of it.
struct Settled {
    path: PathBuf,
    reserved: Arc<Mutex<BTreeSet<PathBuf>>>,
}

impl Drop for Settled {
    fn drop(&mut self) {
        self.reserved
            .lock()
            .expect("reserved destinations")
            .remove(&self.path);
    }
}

/// The hidden folder a transfer of a folder stages in, beside its destination. It is named by
/// the source (its machine too) and the destination, so only a transfer of the same item to the
/// same place continues what an earlier try staged.
pub(super) fn folder_stage(source: &FileTransferEndpoint, destination: &Path) -> PathBuf {
    let parent = destination.parent().unwrap_or(Path::new("/"));
    parent.join(format!(
        "{STAGING_PREFIX}d-{}",
        digest16(&format!(
            "{}\0{}\0{}",
            source.kernel.as_deref().unwrap_or(""),
            source.path,
            destination.display()
        ))
    ))
}

/// Makes the staging folder hold the source's tree of folders, private to the kernel's user, and
/// nothing the source no longer has: a retried transfer finds what an earlier try staged, and a
/// file or folder removed from the source meanwhile (or turned into the other kind) is dropped.
fn prepare_stage(stage: &Path, folders: &[Item], files: &[Item]) -> Result<(), String> {
    create_private_dirs(stage)?;
    let wanted_folders: HashSet<&str> = folders.iter().map(|item| item.relative.as_str()).collect();
    let wanted_files: HashSet<&str> = files.iter().map(|item| item.relative.as_str()).collect();
    let sidecars: HashSet<PathBuf> = files
        .iter()
        .map(|item| crate::agent_file_copy::staging_of(&stage.join(&item.relative)))
        .collect();
    drop_unwanted(stage, "", &wanted_folders, &wanted_files, &sidecars).map_err(|error| {
        format!(
            "The staged copy in `{}` could not be checked: {error}",
            stage.display()
        )
    })?;
    for folder in folders {
        create_private_dirs(&stage.join(&folder.relative))?;
    }
    Ok(())
}

fn drop_unwanted(
    directory: &Path,
    relative: &str,
    folders: &HashSet<&str>,
    files: &HashSet<&str>,
    sidecars: &HashSet<PathBuf>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let below = if relative.is_empty() {
            name.clone()
        } else {
            format!("{relative}/{name}")
        };
        let kind = std::fs::symlink_metadata(&path)?;
        let wanted = if name.starts_with(STAGING_PREFIX) {
            // The bytes of a file still being copied wait beside it for the next try.
            sidecars.contains(&path)
        } else if kind.is_dir() {
            folders.contains(below.as_str())
        } else {
            kind.is_file() && files.contains(below.as_str())
        };
        if !wanted {
            remove_any(&path)?;
        } else if kind.is_dir() && !name.starts_with(STAGING_PREFIX) {
            drop_unwanted(&path, &below, folders, files, sidecars)?;
        }
    }
    Ok(())
}

/// Puts the finished staging folder where the item belongs; an item it replaces is set aside
/// first and removed only once the new one is in place.
fn publish_folder(stage: &Path, destination: &Path, replacing: bool) -> Result<(), String> {
    let aside = replacing.then(|| {
        destination.with_file_name(format!(
            "{STAGING_PREFIX}old-{}",
            digest16(&destination.display().to_string())
        ))
    });
    if let Some(aside) = &aside {
        let _ = remove_any(aside);
        std::fs::rename(destination, aside).map_err(|error| {
            format!("`{}` could not be replaced: {error}", destination.display())
        })?;
    }
    if let Err(error) = std::fs::rename(stage, destination) {
        if let Some(aside) = &aside {
            let _ = std::fs::rename(aside, destination);
        }
        return Err(format!(
            "`{}` could not be put in place: {error}",
            destination.display()
        ));
    }
    if let Some(aside) = aside {
        let _ = remove_any(&aside);
    }
    Ok(())
}

fn remove_any(path: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(path)?.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// A copy keeps the permissions of what it copies (its staging files and folders are private
/// until then); a copy whose permissions cannot be restored is not published.
fn restore_mode(path: &Path, mode: Option<u32>) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let Some(mode) = mode else {
        return Ok(());
    };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o7777)).map_err(
        |error| {
            format!(
                "The permissions of `{}` could not be restored: {error}",
                path.display()
            )
        },
    )
}

/// `path` and its missing parents, private to the kernel's user until the copy is whole.
fn create_private_dirs(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|error| format!("`{}` could not be created: {error}", path.display()))
}

fn digest16(text: &str) -> String {
    Sha256::digest(text.as_bytes())[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn delete_failed(path: &str, reason: &str) -> String {
    let reason = reason.replace(UNREACHABLE_ADVICE, "");
    format!("The copy is complete and verified, but `{path}` could not be deleted from the source: {reason}. Nothing else was removed")
}

impl Entry {
    /// Queues the job's record as it stands now.
    fn save(&self, recorder: &Recorder) -> Pending {
        let job = self.job.lock().expect("job state");
        recorder.save(Record {
            job: job.clone(),
            request: self.request.clone(),
        })
    }
}

/// Forgets the oldest finished jobs beyond what is kept; the ids of those forgotten.
fn trim(entries: &mut VecDeque<Entry>) -> Vec<String> {
    let mut removed = Vec::new();
    while entries.len() > FINISHED_KEPT {
        let Some(position) = entries
            .iter()
            .position(|entry| entry.job.lock().expect("job state").state.is_finished())
        else {
            break;
        };
        if let Some(entry) = entries.remove(position) {
            removed.push(entry.job.lock().expect("job state").job_id.clone());
        }
    }
    removed
}

/// `path` with the links of its existing folders resolved; what does not exist yet stays as it is.
fn resolved(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut base = path;
    loop {
        if let Ok(real) = base.canonicalize() {
            return rest.iter().rev().fold(real, |at, part| at.join(part));
        }
        match (base.parent(), base.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_owned());
                base = parent;
            }
            _ => return path.to_owned(),
        }
    }
}
