//! Authenticated job transport reuses gateway requests and binary Room exec pipes.
use super::*;
use crate::local::*;
use crate::room_exec::gateway_placement::{create_target_room, GrantedSend};
use crate::room_exec::{ExecOutcome, ExecOutput};
use crate::runtime::kernel_gateway::forward_with_retry;
use std::io::Read;

/// A job's place on its machine: the home directory and the workspace root below it.
pub(super) struct Located {
    home: String,
    pub(super) root: String,
}

#[derive(Clone)]
pub(super) struct Remote {
    grant: Arc<dyn GrantedSend>,
    room: String,
    job_id: String,
}

impl Remote {
    pub(super) fn new(grant: Arc<dyn GrantedSend>, room: String, job_id: String) -> Self {
        Self {
            grant,
            room,
            job_id,
        }
    }
    pub(super) fn room(&self) -> &str {
        &self.room
    }
    async fn call(
        &self,
        suffix: &str,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let answer = forward_with_retry(format!("job:{}:{suffix}", self.job_id), |id| {
            self.grant.send(id, &request)
        })
        .await
        .map_err(|e| {
            ledger::error(format!(
                "Job machine request failed: {}",
                e.transport_error().message
            ))
        })?;
        serde_json::from_value(answer)
            .map_err(|_| ledger::error("Job machine returned an unreadable response"))
    }
    async fn operation(
        &self,
        suffix: &str,
        operation: RunCommandOperation,
    ) -> Result<RunCommandResult, DaemonError> {
        match self
            .call(
                suffix,
                LocalDaemonRequest::RunCommand(RunCommandRequest {
                    target: crate::local::CommandTarget::RoomWorkspace {
                        session_id: self.room.clone(),
                    },
                    acting_agent: None,
                    operation,
                }),
            )
            .await?
        {
            LocalDaemonResponse::RunCommand { result } => Ok(result),
            _ => Err(ledger::error("Job machine returned another operation")),
        }
    }
    pub(super) async fn open(
        &self,
        command: Vec<String>,
        needs: Option<JobNeeds>,
        suffix: &str,
    ) -> Result<String, DaemonError> {
        let operation = match needs {
            Some(needs) => RunCommandOperation::ExecOpenJob { command, needs },
            None => RunCommandOperation::ExecOpen {
                command,
                directory: None,
                timeout_ms: None,
            },
        };
        match self.operation(suffix, operation).await? {
            RunCommandResult::ExecOpened { exec_id } => Ok(exec_id),
            _ => Err(ledger::error("Job machine did not start the command")),
        }
    }
    pub(super) async fn write(
        &self,
        exec_id: &str,
        offset: u64,
        bytes: &[u8],
    ) -> Result<u64, DaemonError> {
        match self
            .operation(
                &format!("{exec_id}:input:{offset}"),
                RunCommandOperation::ExecWrite {
                    exec_id: exec_id.into(),
                    offset,
                    data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
                },
            )
            .await?
        {
            RunCommandResult::ExecInput { next_offset } => Ok(next_offset),
            _ => Err(ledger::error("Job machine did not accept command input")),
        }
    }
    pub(super) async fn close_stdin(&self, exec_id: &str, offset: u64) -> Result<(), DaemonError> {
        match self
            .operation(
                &format!("{exec_id}:stdin-close"),
                RunCommandOperation::ExecStdinClose {
                    exec_id: exec_id.into(),
                    offset,
                },
            )
            .await?
        {
            RunCommandResult::ExecInput { .. } => Ok(()),
            _ => Err(ledger::error("Job machine did not close command input")),
        }
    }
    pub(super) async fn read(&self, exec_id: &str, cursor: u64) -> Result<ExecOutput, DaemonError> {
        match self
            .operation(
                &format!("{exec_id}:read:{cursor}:{:016x}", rand::random::<u64>()),
                RunCommandOperation::ExecRead {
                    exec_id: exec_id.into(),
                    after_cursor: cursor,
                    wait_ms: 250,
                },
            )
            .await?
        {
            RunCommandResult::ExecOutput { output } => Ok(output),
            _ => Err(ledger::error("Job machine did not return command output")),
        }
    }
    pub(super) async fn samples(
        &self,
    ) -> Result<Vec<crate::runtime::process_resources::SessionProcess>, DaemonError> {
        match self
            .call(
                &format!("health:{}", crate::session::unix_epoch_ms() / 1000),
                LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
                    session_id: self.room.clone(),
                }),
            )
            .await?
        {
            LocalDaemonResponse::SessionState { processes, .. } => Ok(processes),
            _ => Err(ledger::error(
                "Job machine did not return its process samples",
            )),
        }
    }

    pub(super) async fn cancel(&self, exec_id: &str) -> Result<(), DaemonError> {
        self.operation(
            &format!("{exec_id}:cancel"),
            RunCommandOperation::ExecCancel {
                exec_id: exec_id.into(),
            },
        )
        .await
        .map(|_| ())
    }
    async fn utility(&self, command: Vec<String>, suffix: &str) -> Result<(), DaemonError> {
        let id = self.open(command, None, suffix).await?;
        self.close_stdin(&id, 0).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        let mut cursor = 0;
        loop {
            if tokio::time::Instant::now() >= deadline {
                self.cancel(&id).await?;
                return Err(ledger::error("Job workspace preparation timed out"));
            }
            let page = self.read(&id, cursor).await?;
            cursor = page.next_cursor;
            if let Some(outcome) = page.outcome {
                return if outcome == (ExecOutcome::Exited { code: 0 }) {
                    Ok(())
                } else {
                    Err(ledger::error(
                        "Cannot prepare the job workspace on the remote machine",
                    ))
                };
            }
        }
    }
    /// Deletes the job's uploaded workspace, in a room of its own so it works when the job's rooms
    /// are not at hand any more. The machine's gateway keeps the answer of every state-changing
    /// request under its command id, so each attempt names its own: a retry after a failure would
    /// otherwise be answered with the old failure and never run. Within one attempt the ids stay
    /// the same, so the gateway's own retry of a lost reply is still answered, not run twice.
    pub(super) async fn remove_workspace(&self, root: &str) -> Result<(), DaemonError> {
        let malformed = || ledger::error("Job workspace path is malformed");
        let (job_dir, _) = root.rsplit_once('/').ok_or_else(malformed)?;
        let (jobs_dir, _) = job_dir.rsplit_once('/').ok_or_else(malformed)?;
        let attempt = format!("{}:cleanup:{:032x}", self.job_id, rand::random::<u128>());
        let room = create_target_room(&*self.grant, &attempt, Some(jobs_dir)).await?;
        Self::new(self.grant.clone(), room, attempt)
            .utility(
                vec!["/bin/rm".into(), "-rf".into(), "--".into(), job_dir.into()],
                "cleanup",
            )
            .await
    }
    /// Where the job's workspace goes on the machine, known before any byte is uploaded so it can
    /// be saved first.
    pub(super) async fn locate(
        grant: Arc<dyn GrantedSend>,
        job_id: &str,
    ) -> Result<Located, DaemonError> {
        let probe = Self::new(grant, String::new(), job_id.into());
        let home = match probe
            .call(
                "home",
                LocalDaemonRequest::StatKernelFile(StatKernelFileRequest {
                    path: "~/".into(),
                    with_sha256: false,
                }),
            )
            .await?
        {
            LocalDaemonResponse::KernelFileStat { stat } if stat.exists => stat.path,
            _ => return Err(ledger::error("Job machine has no home directory")),
        };
        let root = format!(
            "{}/chariox-jobs/{job_id}/workspace",
            home.trim_end_matches('/')
        );
        Ok(Located { home, root })
    }
    pub(super) async fn prepare(
        grant: Arc<dyn GrantedSend>,
        record: &Record,
        located: &Located,
        manifest: &crate::jobs::Manifest,
        cancelled: &watch::Receiver<bool>,
    ) -> Result<Self, DaemonError> {
        let Located { home, root } = located;
        let room =
            create_target_room(&*grant, &format!("{}:prepare", record.job_id), Some(home)).await?;
        let preparation = Self::new(grant.clone(), room, record.job_id.clone());
        let relative_cwd = record
            .cwd
            .strip_prefix(&record.workspace)
            .map_err(|_| ledger::error("Job directory escaped workspace"))?;
        let mut directories =
            BTreeSet::from([root.clone(), format!("{root}/{}", relative_cwd.display())]);
        for path in manifest.keys() {
            crate::jobs::relative(path)?;
            if let Some(parent) = Path::new(path)
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                directories.insert(format!("{root}/{}", parent.display()));
            }
        }
        let directories: Vec<_> = directories.into_iter().collect();
        for (index, paths) in directories.chunks(60).enumerate() {
            if *cancelled.borrow() {
                return Err(ledger::error("Job cancelled during workspace upload"));
            }
            let mut command = vec!["/bin/mkdir".into(), "-p".into(), "--".into()];
            command.extend(paths.iter().cloned());
            preparation
                .utility(command, &format!("mkdir:{index}"))
                .await?;
        }
        for (index, (path, state)) in manifest.iter().enumerate() {
            let local = crate::jobs::checked_path(&record.workspace, path)?;
            let mut file = std::fs::File::open(&local)
                .map_err(|e| ledger::error(format!("Cannot upload job file: {e}")))?;
            let size = file
                .metadata()
                .map_err(|e| ledger::error(e.to_string()))?
                .len();
            let mut offset = 0;
            loop {
                if *cancelled.borrow() {
                    return Err(ledger::error("Job cancelled during workspace upload"));
                }
                let mut chunk = vec![0; KERNEL_FILE_MAX_CHUNK_BYTES as usize];
                let read = file
                    .read(&mut chunk)
                    .map_err(|e| ledger::error(e.to_string()))?;
                chunk.truncate(read);
                match preparation
                    .call(
                        &format!("upload:{index}:{offset}"),
                        LocalDaemonRequest::WriteKernelFile(WriteKernelFileRequest {
                            path: format!("{root}/{path}"),
                            offset,
                            data_base64: base64::engine::general_purpose::STANDARD.encode(chunk),
                            total_size: size,
                            sha256: Some(state.sha256.clone()),
                            overwrite: false,
                        }),
                    )
                    .await?
                {
                    LocalDaemonResponse::KernelFileWritten {
                        received, complete, ..
                    } => {
                        offset = received;
                        if complete {
                            break;
                        }
                    }
                    _ => return Err(ledger::error("Job machine did not accept workspace bytes")),
                }
                if read == 0 {
                    return Err(ledger::error(
                        "Job source changed during upload; retry after saving the workspace",
                    ));
                }
            }
        }
        // Uploads preserve bytes; source scripts also need their executable permission.
        let executables: Vec<_> = manifest
            .iter()
            .filter(|(_, state)| state.executable)
            .map(|(path, _)| format!("{root}/{path}"))
            .collect();
        for (index, paths) in executables.chunks(60).enumerate() {
            let mut command = vec!["/bin/chmod".into(), "+x".into(), "--".into()];
            command.extend(paths.iter().cloned());
            preparation
                .utility(command, &format!("chmod:{index}"))
                .await?;
        }
        let relative_cwd = record
            .cwd
            .strip_prefix(&record.workspace)
            .map_err(|_| ledger::error("Job directory escaped workspace"))?;
        let cwd = if relative_cwd.as_os_str().is_empty() {
            root.clone()
        } else {
            format!("{root}/{}", relative_cwd.display())
        };
        let room =
            create_target_room(&*grant, &format!("{}:command", record.job_id), Some(&cwd)).await?;
        Ok(Self::new(grant, room, record.job_id.clone()))
    }

    pub(super) async fn return_files(
        &self,
        root: &str,
        record: &Record,
        base: &crate::jobs::Manifest,
        uploaded: &crate::jobs::Manifest,
    ) -> Result<(Vec<String>, Vec<String>), DaemonError> {
        let rules = crate::workspace_live_sync_ignore::workspace_live_sync_user_ignore_patterns(
            &record.workspace,
        );
        let mut pending = vec![String::new()];
        let mut remote = BTreeMap::new();
        let mut total = 0u64;
        while let Some(directory) = pending.pop() {
            let path = if directory.is_empty() {
                root.to_owned()
            } else {
                format!("{root}/{directory}")
            };
            let mut offset = 0;
            loop {
                let (entries, more) = match self
                    .call(
                        &format!("return-list:{directory}:{offset}"),
                        LocalDaemonRequest::ListKernelDirectory(ListKernelDirectoryRequest {
                            path: path.clone(),
                            offset,
                        }),
                    )
                    .await?
                {
                    LocalDaemonResponse::KernelDirectoryListed {
                        entries, truncated, ..
                    } => (entries, truncated),
                    _ => {
                        return Err(ledger::error(
                            "Job machine did not return a workspace listing",
                        ))
                    }
                };
                offset += entries.len() as u64;
                for entry in entries {
                    let relative = if directory.is_empty() {
                        entry.name
                    } else {
                        format!("{directory}/{}", entry.name)
                    };
                    crate::jobs::relative(&relative)?;
                    if crate::jobs::hard_excluded(&relative) {
                        continue;
                    }
                    let ignored =
                        crate::workspace_live_sync_ignore::workspace_live_sync_ignored_path(
                            &relative, &rules,
                        );
                    let output = record.request.outputs.iter().any(|pattern| crate::workspace_live_sync_ignore::workspace_live_sync_ignore_pattern_matches(pattern, &relative));
                    if entry.kind == KernelFileKind::Directory {
                        if !ignored
                            || record
                                .request
                                .outputs
                                .iter()
                                .any(|pattern| crate::workspace_live_sync_ignore::workspace_live_sync_pattern_may_match_below(pattern, &relative))
                        {
                            pending.push(relative);
                        }
                    } else if entry.kind == KernelFileKind::File && (!ignored || output) {
                        total = total.saturating_add(entry.size);
                        if total > 2 * 1024 * 1024 * 1024 || remote.len() >= 100_000 {
                            return Err(ledger::error(
                                "Job return exceeds the 2 GiB or 100000 file limit",
                            ));
                        }
                        let stat = match self
                            .call(
                                &format!("return-stat:{relative}"),
                                LocalDaemonRequest::StatKernelFile(StatKernelFileRequest {
                                    path: format!("{root}/{relative}"),
                                    with_sha256: true,
                                }),
                            )
                            .await?
                        {
                            LocalDaemonResponse::KernelFileStat { stat } => stat,
                            _ => {
                                return Err(ledger::error("Job machine did not return a file hash"))
                            }
                        };
                        if let Some(sha256) = stat.sha256 {
                            let mode = stat.mode.unwrap_or(entry.mode);
                            remote.insert(
                                relative,
                                crate::jobs::FileState {
                                    sha256,
                                    executable: mode & 0o111 != 0,
                                },
                            );
                        }
                    }
                }
                if !more {
                    break;
                }
            }
        }
        let changed: BTreeSet<_> = uploaded
            .keys()
            .chain(remote.keys())
            .filter(|path| base.get(*path) != remote.get(*path))
            .cloned()
            .collect();
        let (mut files, mut conflicts) = (vec![], vec![]);
        for path in changed {
            let returned = if let Some(expected) = remote.get(&path) {
                let mut bytes = vec![];
                loop {
                    let offset = bytes.len() as u64;
                    match self
                        .call(
                            &format!("return-read:{path}:{offset}"),
                            LocalDaemonRequest::ReadKernelFile(ReadKernelFileRequest {
                                path: format!("{root}/{path}"),
                                offset,
                                length: KERNEL_FILE_MAX_CHUNK_BYTES,
                            }),
                        )
                        .await?
                    {
                        LocalDaemonResponse::KernelFileChunk {
                            data_base64, eof, ..
                        } => {
                            bytes.extend(
                                base64::engine::general_purpose::STANDARD
                                    .decode(data_base64)
                                    .map_err(|_| ledger::error("Invalid job file encoding"))?,
                            );
                            if bytes.len() > 2 * 1024 * 1024 * 1024 {
                                return Err(ledger::error("Job returned an oversized file"));
                            }
                            if eof {
                                break;
                            }
                        }
                        _ => return Err(ledger::error("Job machine did not return file bytes")),
                    }
                }
                use sha2::Digest;
                if format!("{:x}", sha2::Sha256::digest(&bytes)) != expected.sha256 {
                    return Err(ledger::error(
                        "Job return changed while being read; its hash does not match",
                    ));
                }
                Some(crate::jobs::ReturnedFile {
                    bytes,
                    executable: expected.executable,
                })
            } else {
                None
            };
            let applied = crate::jobs::apply_return(
                &record.workspace,
                &record.job_id,
                &path,
                base.get(&path),
                returned.as_ref(),
            )?;
            if applied {
                files.push(path);
            } else {
                conflicts.push(path);
            }
        }
        Ok((files, conflicts))
    }
}
