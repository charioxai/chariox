//! Authenticated job transport reuses gateway requests and binary Room exec pipes.
use super::*;
use crate::local::*;
use crate::room_exec::gateway_placement::{create_target_room, GrantedSend};
use crate::room_exec::{ExecOutcome, ExecOutput};
use crate::runtime::kernel_gateway::forward_with_retry;
use std::io::Read;

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
        operation: RoomTerminalOperation,
    ) -> Result<RoomTerminalResult, DaemonError> {
        match self
            .call(
                suffix,
                LocalDaemonRequest::RoomTerminal(RoomTerminalRequest {
                    session_id: self.room.clone(),
                    acting_agent: None,
                    host_agent: None,
                    operation,
                }),
            )
            .await?
        {
            LocalDaemonResponse::RoomTerminal { result } => Ok(result),
            _ => Err(ledger::error("Job machine returned another operation")),
        }
    }
    pub(super) async fn open(
        &self,
        command: Vec<String>,
        job: bool,
        suffix: &str,
    ) -> Result<String, DaemonError> {
        let operation = if job {
            RoomTerminalOperation::ExecOpenJob { command }
        } else {
            RoomTerminalOperation::ExecOpen { command }
        };
        match self.operation(suffix, operation).await? {
            RoomTerminalResult::ExecOpened { exec_id } => Ok(exec_id),
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
                RoomTerminalOperation::ExecWrite {
                    exec_id: exec_id.into(),
                    offset,
                    data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
                },
            )
            .await?
        {
            RoomTerminalResult::ExecInput { next_offset } => Ok(next_offset),
            _ => Err(ledger::error("Job machine did not accept command input")),
        }
    }
    pub(super) async fn close_stdin(&self, exec_id: &str, offset: u64) -> Result<(), DaemonError> {
        match self
            .operation(
                &format!("{exec_id}:stdin-close"),
                RoomTerminalOperation::ExecStdinClose {
                    exec_id: exec_id.into(),
                    offset,
                },
            )
            .await?
        {
            RoomTerminalResult::ExecInput { .. } => Ok(()),
            _ => Err(ledger::error("Job machine did not close command input")),
        }
    }
    pub(super) async fn read(&self, exec_id: &str, cursor: u64) -> Result<ExecOutput, DaemonError> {
        match self
            .operation(
                &format!("{exec_id}:read:{cursor}:{:016x}", rand::random::<u64>()),
                RoomTerminalOperation::ExecRead {
                    exec_id: exec_id.into(),
                    after_cursor: cursor,
                    wait_ms: 250,
                },
            )
            .await?
        {
            RoomTerminalResult::ExecOutput { output } => Ok(output),
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
            RoomTerminalOperation::ExecCancel {
                exec_id: exec_id.into(),
            },
        )
        .await
        .map(|_| ())
    }
    async fn utility(&self, command: Vec<String>, suffix: &str) -> Result<(), DaemonError> {
        let id = self.open(command, false, suffix).await?;
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
    pub(super) async fn prepare(
        grant: Arc<dyn GrantedSend>,
        record: &Record,
        manifest: &crate::jobs::Manifest,
        cancelled: &watch::Receiver<bool>,
    ) -> Result<(Self, String), DaemonError> {
        let probe = Self::new(grant.clone(), String::new(), record.job_id.clone());
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
        let room =
            create_target_room(&*grant, &format!("{}:prepare", record.job_id), Some(&home)).await?;
        let preparation = Self::new(grant.clone(), room, record.job_id.clone());
        let root = format!(
            "{}/chariox-jobs/{}/workspace",
            home.trim_end_matches('/'),
            record.job_id
        );
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
        for (index, (path, hash)) in manifest.iter().enumerate() {
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
                            sha256: Some(hash.clone()),
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let executables: Vec<_> = manifest
                .keys()
                .filter(|path| {
                    std::fs::metadata(record.workspace.join(path))
                        .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
                })
                .map(|path| format!("{root}/{path}"))
                .collect();
            for (index, paths) in executables.chunks(60).enumerate() {
                let mut command = vec!["/bin/chmod".into(), "+x".into(), "--".into()];
                command.extend(paths.iter().cloned());
                preparation
                    .utility(command, &format!("chmod:{index}"))
                    .await?;
            }
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
        Ok((Self::new(grant, room, record.job_id.clone()), root))
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
        let mut modes = BTreeMap::new();
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
                                .any(|pattern| pattern.starts_with(&format!("{relative}/")))
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
                        if let Some(hash) = stat.sha256 {
                            modes.insert(relative.clone(), stat.mode.unwrap_or(entry.mode) & 0o777);
                            remote.insert(relative, hash);
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
            let bytes = if let Some(expected) = remote.get(&path) {
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
                if format!("{:x}", sha2::Sha256::digest(&bytes)) != *expected {
                    return Err(ledger::error(
                        "Job return changed while being read; its hash does not match",
                    ));
                }
                Some(bytes)
            } else {
                None
            };
            let applied = crate::jobs::apply_return(
                &record.workspace,
                &record.job_id,
                &path,
                base.get(&path).map(String::as_str),
                bytes.as_deref(),
            )?;
            #[cfg(unix)]
            if bytes.is_some() {
                use std::os::unix::fs::PermissionsExt;
                let target = if applied {
                    record.workspace.join(&path)
                } else {
                    record
                        .workspace
                        .join(format!(".chariox/jobs/{}/conflicts/{path}", record.job_id))
                };
                std::fs::set_permissions(
                    target,
                    std::fs::Permissions::from_mode(modes.get(&path).copied().unwrap_or(0o644)),
                )
                .map_err(|e| ledger::error(format!("Cannot return job file permissions: {e}")))?;
            }
            if applied {
                files.push(path);
            } else {
                conflicts.push(path);
            }
        }
        Ok((files, conflicts))
    }
}
