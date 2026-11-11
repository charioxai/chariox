//! File transfers between machines as kernel-owned jobs. The kernel that is to hold the copy runs
//! the job: it reads the source slice by slice (from this machine or over the relay), stages the
//! copy beside its destination and publishes it only when it is whole and its digest matches.
//! The job outlives whoever asked for it; its state stays readable after it ends.
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use sha2::{Digest, Sha256};

use super::{FileSource, Machine};
use crate::agent_file_copy::{
    pull, CancelToken, Control, Copied, CopyOptions, LinkError, CANCELED,
};
use crate::local::{
    FileTransferConflict, FileTransferJob, FileTransferState, KernelDirectoryEntry, KernelFileKind,
    StartFileTransferRequest,
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
/// The hidden names a transfer stages under, beside its destination.
const STAGING_PREFIX: &str = ".chariox-copy-";

struct Entry {
    job: Arc<Mutex<FileTransferJob>>,
    cancel: tokio::sync::watch::Sender<bool>,
}

#[derive(Default)]
pub(crate) struct TransferJobs {
    entries: Mutex<VecDeque<Entry>>,
}

impl TransferJobs {
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
        {
            let mut entries = self.entries.lock().expect("transfer jobs");
            entries.push_back(Entry {
                job: job.clone(),
                cancel,
            });
            // Only finished jobs are forgotten, oldest first.
            while entries.len() > FINISHED_KEPT {
                let Some(position) = entries
                    .iter()
                    .position(|entry| entry.job.lock().expect("job state").state.is_finished())
                else {
                    break;
                };
                entries.remove(position);
            }
        }
        tokio::spawn(async move {
            let run = Run {
                job: job.clone(),
                token,
                source,
                machine,
            };
            let outcome = run.transfer(&request).await;
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
        });
        first
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
    token: CancelToken,
    source: FileSource,
    machine: Machine,
}

/// A file or folder of the source tree, by path below the transferred item.
#[derive(Clone)]
struct Item {
    relative: String,
    size: u64,
    mode: u32,
}

impl Run {
    fn update(&self, change: impl FnOnce(&mut FileTransferJob)) {
        change(&mut self.job.lock().expect("job state"));
    }

    async fn transfer(&self, request: &StartFileTransferRequest) -> Result<(), String> {
        let source_path = request.source.path.clone();
        let stat = self.source.stat(&source_path).await.map_err(link_reason)?;
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
        let Some(destination) = self.settle_name(&folder, &name, request.conflict)? else {
            self.update(|job| job.skipped += 1);
            return Ok(());
        };
        let replacing = std::fs::symlink_metadata(&destination.path).is_ok();
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
                self.transfer_folder(request, &source_path, &destination.path, replacing)
                    .await
            }
            _ => Err(format!(
                "`{source_path}` is not a file or a folder; links and special files are not copied"
            )),
        }
    }

    /// Where the item lands under `conflict`, or `None` when the name is taken and is to be left.
    fn settle_name(
        &self,
        folder: &Path,
        name: &str,
        conflict: FileTransferConflict,
    ) -> Result<Option<Settled>, String> {
        let wanted = folder.join(name);
        if std::fs::symlink_metadata(&wanted).is_err() {
            return Ok(Some(Settled { path: wanted }));
        }
        match conflict {
            FileTransferConflict::Fail => Err(format!(
                "`{}` already exists; choose replace, keep_both or skip",
                wanted.display()
            )),
            FileTransferConflict::Skip => Ok(None),
            FileTransferConflict::Replace => Ok(Some(Settled { path: wanted })),
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
                    .find(|candidate| std::fs::symlink_metadata(candidate).is_err())
                    .map(|path| Some(Settled { path }))
                    .ok_or_else(|| "no free name".to_owned())
            }
        }
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
        restore_mode(Path::new(&destination), mode);
        self.update(|job| {
            job.files_done = 1;
            job.bytes_done = copied.bytes;
            job.sha256 = Some(copied.sha256.clone());
        });
        if request.delete_source {
            self.update(|job| job.state = FileTransferState::Finishing);
            self.source
                .delete(source_path, Some(&copied.revision))
                .await
                .map_err(|error| delete_failed(source_path, &link_reason(error)))?;
        }
        Ok(())
    }

    async fn transfer_folder(
        &self,
        request: &StartFileTransferRequest,
        source_path: &str,
        destination: &Path,
        replacing: bool,
    ) -> Result<(), String> {
        let (folders, files) = self.scan(source_path).await?;
        let parent = destination.parent().unwrap_or(Path::new("/"));
        let stage = parent.join(format!(
            "{STAGING_PREFIX}d-{}",
            digest16(&format!("{source_path}\0{}", destination.display()))
        ));
        let outcome = self
            .copy_folder(
                request,
                source_path,
                &stage,
                destination,
                replacing,
                &folders,
                &files,
            )
            .await;
        match &outcome {
            // A link that stayed down leaves the staged files, so the same transfer continues.
            Err(reason) if reason.contains("stayed unreachable") => {}
            Err(_) => {
                let _ = std::fs::remove_dir_all(&stage);
            }
            Ok(()) => {}
        }
        outcome
    }

    #[allow(clippy::too_many_arguments)]
    async fn copy_folder(
        &self,
        request: &StartFileTransferRequest,
        source_path: &str,
        stage: &Path,
        destination: &Path,
        replacing: bool,
        folders: &[Item],
        files: &[Item],
    ) -> Result<(), String> {
        std::fs::create_dir_all(stage)
            .map_err(|error| format!("`{}` could not be created: {error}", stage.display()))?;
        for folder in folders {
            let path = stage.join(&folder.relative);
            std::fs::create_dir_all(&path)
                .map_err(|error| format!("`{}` could not be created: {error}", path.display()))?;
        }
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
        for item in folders {
            restore_mode(&stage.join(&item.relative), Some(item.mode));
        }
        publish_folder(stage, destination, replacing)?;
        if request.delete_source {
            self.update(|job| {
                job.state = FileTransferState::Finishing;
                job.current = None;
            });
            for (relative, copied) in &copied {
                let path = format!("{source_path}/{relative}");
                self.source
                    .delete(&path, Some(&copied.revision))
                    .await
                    .map_err(|error| delete_failed(&path, &link_reason(error)))?;
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
                match self.source.delete(&path, None).await {
                    Err(error) if !kept_some => {
                        return Err(delete_failed(&path, &link_reason(error)));
                    }
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
        restore_mode(Path::new(&target), Some(item.mode));
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
                let (entries, more) = tokio::select! {
                    listed = self.source.list(&path, offset) => listed.map_err(link_reason)?,
                    () = self.token_raised() => return Err(CANCELED.to_owned()),
                };
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

    async fn token_raised(&self) {
        self.token.raised().await;
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

struct Settled {
    path: PathBuf,
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

/// A copy keeps the permissions of what it copies (its staging file is private until then).
fn restore_mode(path: &Path, mode: Option<u32>) {
    use std::os::unix::fs::PermissionsExt;
    if let Some(mode) = mode {
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o7777));
    }
}

fn digest16(text: &str) -> String {
    Sha256::digest(text.as_bytes())[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn link_reason(error: LinkError) -> String {
    match error {
        LinkError::Offline(why) => format!("The source machine stayed unreachable ({why})"),
        LinkError::Refused(reason) => reason,
    }
}

fn delete_failed(path: &str, reason: &str) -> String {
    format!("The copy is complete and verified, but `{path}` could not be deleted from the source: {reason}. Nothing else was removed")
}
