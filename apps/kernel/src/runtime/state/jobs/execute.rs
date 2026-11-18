use super::never_fit::Shortfall;
use super::pending_return::PendingReturn;
use super::remote::Remote;
use super::*;
use crate::local::OffloadPolicy;
use crate::room_exec::{ExecOutcome, ExecOutput, ExecStream, OUTPUT_LOSS_NOTICE};
use std::io::Write;
use zeroize::Zeroizing;

/// Output is protected as complete lines, including values that cross relay chunk boundaries.
pub(super) struct Output {
    pending: Vec<u8>,
    pub(super) tail: String,
    file: std::fs::File,
    pub(super) log: String,
    discarding_line: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct OutputCheckpoint {
    pending: Vec<u8>,
    tail: String,
    discarding_line: bool,
}

impl Output {
    fn path(state: &KernelRuntimeState, record: &Record) -> PathBuf {
        state
            .owned
            .config_projection
            .snapshot()
            .private_runtime_state_root()
            .join("jobs")
            .join(&record.job_id)
            .join("output.log")
    }

    pub(super) fn existing_log(state: &KernelRuntimeState, record: &Record) -> Option<String> {
        let path = Self::path(state, record);
        path.is_file().then(|| path.to_string_lossy().into_owned())
    }

    pub(super) fn new(state: &KernelRuntimeState, record: &Record) -> Result<Self, DaemonError> {
        let path = Self::path(state, record);
        std::fs::create_dir_all(path.parent().expect("job output has a directory"))
            .map_err(|e| ledger::error(e.to_string()))?;
        let file = std::fs::File::create(&path)
            .map_err(|e| ledger::error(format!("Cannot keep job output: {e}")))?;
        Ok(Self {
            pending: vec![],
            tail: String::new(),
            file,
            log: path.to_string_lossy().into_owned(),
            discarding_line: false,
        })
    }

    pub(super) fn take_checkpoint(&mut self) -> OutputCheckpoint {
        OutputCheckpoint {
            pending: std::mem::take(&mut self.pending),
            tail: std::mem::take(&mut self.tail),
            discarding_line: self.discarding_line,
        }
    }

    pub(super) fn reopen(
        state: &KernelRuntimeState,
        record: &Record,
        checkpoint: OutputCheckpoint,
        file: std::fs::File,
    ) -> Self {
        Self {
            pending: checkpoint.pending,
            tail: checkpoint.tail,
            file,
            log: Self::path(state, record).to_string_lossy().into_owned(),
            discarding_line: checkpoint.discarding_line,
        }
    }

    pub(super) fn keep_log(&self) -> std::io::Result<std::os::fd::OwnedFd> {
        self.file.try_clone().map(Into::into)
    }

    fn push(
        &mut self,
        state: &KernelRuntimeState,
        room: &str,
        bytes: &[u8],
        finish: bool,
    ) -> Result<(), DaemonError> {
        let bytes = if self.discarding_line {
            match bytes.iter().position(|byte| *byte == b'\n') {
                Some(end) => {
                    self.discarding_line = false;
                    &bytes[end + 1..]
                }
                None => {
                    return Ok(());
                }
            }
        } else {
            bytes
        };
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self
            .pending
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|i| i + 1)
            .or_else(|| (finish && !self.pending.is_empty()).then_some(self.pending.len()))
        {
            let line: Vec<_> = self.pending.drain(..end).collect();
            let mut text = if line.len() > 1024 * 1024 {
                "[job output line withheld: over 1 MiB]".into()
            } else {
                super::super::agent_process_output::sanitize(&line)
            };
            text = state
                .owned
                .room_secret_observations
                .scrub(room, text)
                .unwrap_or_else(|_| "[sensitive job output withheld]".into());
            text.push('\n');
            self.tail.push_str(&text);
            if self.tail.len() > TAIL_BYTES {
                let mut start = self.tail.len() - TAIL_BYTES;
                while !self.tail.is_char_boundary(start) {
                    start += 1;
                }
                self.tail.drain(..start);
            }
            self.file
                .write_all(text.as_bytes())
                .map_err(|e| ledger::error(format!("Cannot keep job output: {e}")))?;
        }
        // A producer cannot consume unbounded memory with one unterminated line.
        if self.pending.len() > 1024 * 1024 {
            self.pending.clear();
            self.discarding_line = true;
            self.file
                .write_all(b"[job output line withheld: over 1 MiB]\n")
                .map_err(|e| ledger::error(e.to_string()))?;
        }
        Ok(())
    }
    /// Writes a notice of the kernel's own. A line the producer left unfinished ends first and the
    /// discard of an over-long line stops, so the notice is neither joined to it nor swallowed.
    pub(super) fn notice(
        &mut self,
        state: &KernelRuntimeState,
        room: &str,
        text: &str,
    ) -> Result<(), DaemonError> {
        self.discarding_line = false;
        self.push(state, room, &[], true)?;
        self.push(state, room, text.as_bytes(), true)
    }
    pub(super) fn page(
        &mut self,
        state: &KernelRuntimeState,
        room: &str,
        page: &ExecOutput,
    ) -> Result<(), DaemonError> {
        for chunk in &page.chunks {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&chunk.data_base64)
                .map_err(|_| ledger::error("Job output is not valid base64"))?;
            // Eviction interrupts producer line framing, including an overlong line being discarded.
            if chunk.stream == ExecStream::Stderr
                && bytes.as_slice() == OUTPUT_LOSS_NOTICE.as_bytes()
            {
                self.notice(state, room, OUTPUT_LOSS_NOTICE)?;
            } else {
                self.push(state, room, &bytes, false)?;
            }
        }
        if page.outcome.is_some() {
            self.push(state, room, &[], true)?;
        }
        Ok(())
    }
}

fn selected_environment(
    state: &KernelRuntimeState,
    record: &Record,
) -> Result<BTreeMap<String, Zeroizing<String>>, DaemonError> {
    let session = state.owned.session_store.get_session(&record.room_id)?;
    if record.request.env.is_empty() {
        return Ok(BTreeMap::new());
    }
    let project = state
        .owned
        .session_store
        .environment_project(&session)
        .ok_or_else(|| {
            ledger::error(
                "The job names environment entries but this Room has no Project Environment",
            )
        })?;
    let environment = state
        .environment_store()
        .load(project.id())?
        .ok_or_else(|| ledger::error("This Project has no saved Environment"))?;
    let variables: BTreeMap<_, _> =
        crate::project_environment::parse_variables(&environment.variables)
            .map_err(ledger::error)?
            .into_iter()
            .collect();
    let config = state.owned.config_projection.snapshot();
    let mut selected = BTreeMap::new();
    for name in &record.request.env {
        if crate::project_environment::project_environment_protected_name(name) {
            return Err(ledger::error(
                "The job requested a protected environment name",
            ));
        }
        let value = if environment.secrets.iter().any(|secret| secret == name) {
            let vault = crate::secret::project_environment_vault(&config)?;
            vault.find_secret(&crate::project_environment::project_environment_vault_service(project.id()), name)?.ok_or_else(|| ledger::error(format!("Project Environment entry {name} is missing; fill it in the Environment first")))?
        } else {
            variables.get(name).cloned().ok_or_else(|| {
                ledger::error(format!("{name} is not a Project Environment entry"))
            })?
        };
        if value.contains('\0') {
            return Err(ledger::error(format!("Project Environment entry {name} contains a null byte; update it before running the job")));
        }
        selected.insert(name.to_string(), Zeroizing::new(value));
    }
    Ok(selected)
}

impl KernelRuntimeState {
    pub(super) async fn execute_job(
        &self,
        wake: AgentWake,
        mut record: Record,
        mut cancelled: watch::Receiver<bool>,
        local: Option<(
            Output,
            super::local::ActiveLocalJob,
            super::local::LocalProgress,
        )>,
    ) {
        let (mut output, progress) = match local {
            Some((output, active, progress)) => (output, Some((active, progress))),
            None => loop {
                match Output::new(self, &record) {
                    Ok(output) => break (output, None),
                    Err(error) if record.remote.awaits_command() && !*cancelled.borrow() => {
                        tracing::warn!(%error, job_id = %record.job_id, "job output log cannot be opened; retrying");
                        tokio::time::sleep(LOCAL_STORAGE_RETRY).await;
                    }
                    Err(error) => {
                        if record.remote.awaits_command() {
                            self.stop_held_command(&record).await;
                            self.give_up_pending_return(&mut record);
                            self.settle_or_warn(&record).await;
                            if let (Some(allocation), Some(revision)) =
                                (&record.allocation_ref, &record.capability_revision)
                            {
                                record.deleted = self
                                    .job_compute_release_ref(allocation, revision)
                                    .await
                                    .is_ok();
                            }
                        }
                        let result = failed(&record, error.to_string());
                        self.finish_job(&wake, &mut record, result).await;
                        return;
                    }
                }
            },
        };
        let result = if let Some((active, progress)) = progress {
            self.resume_local_job(&mut record, &mut output, &mut cancelled, active, progress)
                .await
        } else if matches!(record.remote, RemoteStage::Running { .. }) {
            self.resume_remote_job(&mut record, &mut output, &mut cancelled)
                .await
        } else {
            match selected_environment(self, &record) {
                Err(error) => Err(error),
                Ok(environment) => {
                    self.run_job(
                        &mut record,
                        &environment,
                        &mut output,
                        &mut cancelled,
                        false,
                    )
                    .await
                }
            }
        };
        if record.allocation_ref.is_none() {
            if let Some(receipt) = self
                .job_wake_driver
                .as_ref()
                .and_then(|driver| driver.read(&record.job_id).ok())
            {
                record.allocation_ref = receipt.allocation_ref;
                record.capability_revision = receipt.capability_revision;
            }
        }
        let _ = output.push(self, &record.room_id, &[], true);
        let mut result = result.unwrap_or_else(|error| {
            let _ = output.push(self, &record.room_id, error.to_string().as_bytes(), true);
            record.result.clone().unwrap_or_else(|| {
                failed(&record, "Job failed; see the protected output log".into())
            })
        });
        output.file.flush().ok();
        let config = self.owned.config_projection.snapshot();
        let artifact = crate::artifacts::OperationalArtifactStore::open(
            config.operational_artifact_root(),
            config.operational_artifact_index_path(),
        )
        .and_then(|store| {
            store.store_existing_file(crate::artifacts::StoreArtifactRequest {
                source_path: PathBuf::from(&output.log),
                display_name: format!("{}.log", record.job_id),
                source_kind: "job_output".into(),
                media_type: Some("text/plain".into()),
                enqueue_archive: false,
                session_id: Some(record.room_id.clone()),
                attachment_id: None,
                workspace_id: None,
                worktree_path: Some(record.workspace.to_string_lossy().into_owned()),
                metadata: BTreeMap::from([("job_id".into(), record.job_id.clone().into())]),
            })
        });
        if self.job_wake_driver.as_ref().is_some_and(|driver| {
            driver
                .owner_stops
                .lock()
                .expect("job stops")
                .remove(&record.job_id)
        }) {
            result.outcome = JobOutcome::OutsideEnd;
        }
        if let Some(driver) = &self.job_wake_driver {
            driver
                .remote_samples
                .lock()
                .expect("remote job samples")
                .remove(&record.job_id);
        }
        result.output_tail = output.tail;
        result.log = match artifact {
            Ok(artifact) => artifact.operational_path.to_string_lossy().into_owned(),
            Err(_) => output.log,
        };
        if record.result.is_none() {
            result.duration_ms = crate::session::unix_epoch_ms()
                .saturating_sub(record.running_at_ms.unwrap_or(record.started_at_ms));
        }
        self.finish_job(&wake, &mut record, result).await;
    }

    async fn run_job(
        &self,
        record: &mut Record,
        environment: &BTreeMap<String, Zeroizing<String>>,
        output: &mut Output,
        cancelled: &mut watch::Receiver<bool>,
        force_remote: bool,
    ) -> Result<JobResult, DaemonError> {
        if let Some(held) = record.request.machine.clone() {
            return self
                .run_job_on_held_machine(record, &held, environment, output, cancelled)
                .await;
        }
        let command = validate(&record.request)?;
        let admission = crate::runtime::process_resources::LaunchAdmission::default();
        let busy = admission.would_wait(
            &format!("{}:probe", record.job_id),
            &command,
            &record.cwd,
            u64::from(record.request.needs.memory_gib.unwrap_or(0)) * 1024 * 1024 * 1024,
        );
        let heavy = record.request.needs.memory_gib.is_some()
            || record.request.needs.cpus.is_some_and(|cpus| cpus > 1)
            || admission.is_heavy(&command, &record.cwd);
        let policy = self.effective_remote_jobs_policy(&record.room_id, &record.agent_id)?;
        let requested = self.job_wake_driver.as_ref().is_some_and(|driver| {
            driver
                .remote_requested
                .lock()
                .expect("job move")
                .remove(&record.job_id)
        });
        // A job this machine can never run still asks other machines first, whatever the policy.
        let shortfall = Shortfall::of(
            &record.request.needs,
            admission.capacity(),
            std::thread::available_parallelism().map_or(u32::MAX, |cpus| cpus.get() as u32),
        );
        let remote = self.remote_jobs_enabled()
            && (force_remote
                || requested
                || shortfall.is_some()
                || match policy {
                    OffloadPolicy::Off => false,
                    OffloadPolicy::WhenBusy => busy,
                    OffloadPolicy::AlwaysHeavy => heavy,
                });
        if *cancelled.borrow() {
            return Ok(cancelled_result(record));
        }
        if let (Some(shortfall), false) = (&shortfall, remote) {
            return self.refuse_never_fit(
                record,
                output,
                shortfall,
                "remote jobs are switched off on this kernel (CHARIOX_REMOTE_JOBS)",
            );
        }
        if remote {
            let requested_origin = self.job_wake_driver.as_ref().and_then(|driver| {
                driver
                    .remote_origins
                    .lock()
                    .expect("job move origins")
                    .remove(&record.job_id)
            });
            let origin = match requested_origin {
                Some(origin) => origin,
                None => {
                    self.job_acquisition_subject(&record.job_id)?;
                    crate::runtime::kernel_gateway::Origin {
                        grant_id: format!("job:{}", record.job_id),
                        harness: self
                            .agent_harness_name(&record.room_id, &record.agent_id)
                            .unwrap_or_else(|| "Chariox".into()),
                    }
                }
            };
            record.state = "allocating".into();
            record.placement = JobPlacement::Remote;
            save_record(self, record)?;
            let needs = effective_needs(record);
            let acquired = tokio::select! {
                acquired = self.job_compute_acquire(&record.job_id, &needs, record.request.timeout_s, &origin) => acquired,
                _ = cancelled.changed() => {
                    // The service records a cancellation fence before looking up the allocation,
                    // so a delayed acquire can never create a machine after this cleanup.
                    self.job_compute_release_job_ref(&record.job_id).await?;
                    record.deleted = true;
                    return Ok(cancelled_result(record));
                }
            };
            if let Some(receipt) = self
                .job_wake_driver
                .as_ref()
                .and_then(|driver| driver.read(&record.job_id).ok())
            {
                record.allocation_ref = receipt.allocation_ref;
                record.capability_revision = receipt.capability_revision;
                record.deleted |= receipt.deleted;
            }
            match acquired {
                Ok(machine) => {
                    let class = machine
                        .class
                        .as_ref()
                        .expect("a job allocation carries its class");
                    record.allocation_ref = Some(machine.allocation_ref.clone());
                    record.capability_revision = Some(class.capability_revision.clone());
                    record.machine_label = Some(machine.machine_label.clone());
                    record.address = machine.address.clone();
                    record.class_id = Some(class.class_id.clone());
                    record.rate_per_minute = Some(class.rate_per_minute.clone());
                    // The allocation budget is an upper bound, not a duration estimate.
                    save_record(self, record)?;
                    let result = self
                        .run_remote_job(record, &machine, environment, output, cancelled)
                        .await;
                    let was_cancelled = *cancelled.borrow();
                    return self
                        .complete_remote_job(record, output, result, was_cancelled)
                        .await;
                }
                Err(error) if shortfall.is_some() => {
                    let answer = if is_capacity_failure(&error) {
                        "no other machine can take it".into()
                    } else {
                        format!("asking another machine failed: {error}")
                    };
                    let shortfall = shortfall.as_ref().expect("guarded");
                    return self.refuse_never_fit(record, output, shortfall, &answer);
                }
                Err(error) if local_fallback_notice(&error).is_some() => {
                    let capacity = is_capacity_failure(&error);
                    record.placement = JobPlacement::Local;
                    record.remote_refusal =
                        (record.remote_available && capacity).then(|| "no_capacity".into());
                    record.state = "queued".into();
                    save_record(self, record)?;
                    if record.remote_available {
                        let notice = local_fallback_notice(&error).expect("checked by the guard");
                        output.push(self, &record.room_id, notice.as_bytes(), false)?;
                    }
                }
                Err(error) => return Err(error),
            }
        }
        match self
            .run_local_job(record, environment, output, cancelled)
            .await
        {
            Err(DaemonError::MachineRequest { code, .. }) if code == "job_move_remote" => {
                Box::pin(self.run_job(record, environment, output, cancelled, true)).await
            }
            result => result,
        }
    }

    /// The one result every client shows for a job no machine can ever run.
    fn refuse_never_fit(
        &self,
        record: &mut Record,
        output: &mut Output,
        shortfall: &Shortfall,
        answer: &str,
    ) -> Result<JobResult, DaemonError> {
        record.placement = JobPlacement::Local;
        record.remote_refusal = record.remote_available.then(|| "no_capacity".into());
        output.push(
            self,
            &record.room_id,
            shortfall.refusal(answer).as_bytes(),
            true,
        )?;
        let mut result = failed(record, String::new());
        result.outcome = JobOutcome::CapacityUnavailable;
        Ok(result)
    }

    pub(super) async fn complete_remote_job(
        &self,
        record: &mut Record,
        output: &mut Output,
        result: Result<JobResult, DaemonError>,
        cancelled: bool,
    ) -> Result<JobResult, DaemonError> {
        let result = match result {
            Err(_) if cancelled => uncollected_stop(self, record, output, JobOutcome::Cancelled),
            Err(error) if record.remote.awaits_command() => {
                // Its end was never read: stop it by deleting its worker (a held machine is not
                // deleted, so its command is stopped by name); nothing is collected.
                if let Some(notice) = self.stop_held_command(record).await {
                    let _ = output.push(self, &record.room_id, notice.as_bytes(), true);
                }
                self.give_up_pending_return(record);
                Err(error)
            }
            result => result,
        };
        let mut result = result.unwrap_or_else(|error| {
            let _ = output.push(self, &record.room_id, error.to_string().as_bytes(), true);
            if record.remote.holds_worker() {
                let _ = output.push(
                    self,
                    &record.room_id,
                    kept_worker_notice(record).as_bytes(),
                    true,
                );
            }
            failed(record, "Job failed; see the protected output log".into())
        });
        output
            .file
            .flush()
            .map_err(|e| ledger::error(e.to_string()))?;
        result.output_tail = output.tail.clone();
        result.log = output.log.clone();
        result.duration_ms = crate::session::unix_epoch_ms()
            .saturating_sub(record.running_at_ms.unwrap_or(record.started_at_ms));
        // Execution and file return are complete. A service restart during
        // deletion must not erase their result, including after kernel recovery.
        record.result = Some(result.clone());
        if record.remote.holds_worker() {
            save_record(self, record)?;
            return Ok(result);
        }
        if record.request.machine.is_some() {
            // The agent keeps holding the machine: nothing is released. The settled stage and the
            // whole result are saved before the uploaded workspace goes, so a restart between the
            // two finds a finished job whose cleanup is left, never a pending return.
            save_record(self, record)?;
            self.settle_or_warn(record).await;
            return Ok(result);
        }
        self.forget_pending_return(&record.job_id);
        record.state = "deleting".into();
        save_record(self, record)?;
        match self
            .job_compute_release_ref(
                record.allocation_ref.as_deref().expect("remote allocation"),
                record
                    .capability_revision
                    .as_deref()
                    .expect("remote capability"),
            )
            .await
        {
            Ok(()) => record.deleted = true,
            Err(error) => {
                let notice = format!("Job finished; remote machine cleanup is pending: {error}. Check this job again to retry cleanup.\n");
                output.push(self, &record.room_id, notice.as_bytes(), true)?;
            }
        }
        save_record(self, record)?;
        Ok(result)
    }

    pub(super) async fn run_remote_job(
        &self,
        record: &mut Record,
        machine: &super::super::job_compute::JobMachine,
        environment: &BTreeMap<String, Zeroizing<String>>,
        output: &mut Output,
        cancelled: &mut watch::Receiver<bool>,
    ) -> Result<JobResult, DaemonError> {
        if *cancelled.borrow() {
            return Ok(cancelled_result(record));
        }
        record.state = "uploading".into();
        save_record(self, record)?;
        let base = crate::jobs::snapshot(&record.workspace, &record.request.outputs)?;
        let upload = crate::jobs::snapshot(&record.workspace, &[])?;
        let located = Remote::locate(machine.grant.clone(), &record.job_id).await?;
        // From here the machine may come to hold the only copy of the outputs, and a held machine
        // keeps the uploaded workspace until it is removed: keep where it is and what collecting
        // needs before the first byte goes up, so nothing after it can lose them or leave them
        // behind. The worker is released only once the command's end is read and its files are
        // home, or it is stopped; a held machine's workspace is removed then.
        let plan = PendingReturn {
            root: located.root.clone(),
            kernel_id: machine.kernel_id.clone(),
            base,
            uploaded: upload,
        };
        plan.save(self, &record.job_id)?;
        record.advance(StageEvent::Plan)?;
        if let Err(error) = save_record(self, record) {
            // Nothing is uploaded yet, so nothing is left on the machine.
            self.give_up_pending_return(record);
            self.forget_pending_return(&record.job_id);
            return Err(error);
        }
        if let Some(driver) = &self.job_wake_driver {
            driver
                .return_grants
                .lock()
                .expect("job return grants")
                .insert(record.job_id.clone(), machine.grant.clone());
        }
        let remote = Remote::prepare(
            machine.grant.clone(),
            record,
            &located,
            &plan.uploaded,
            cancelled,
        )
        .await?;
        if *cancelled.borrow() {
            return Err(ledger::error("Job cancelled during workspace upload"));
        }
        let exec_id = remote
            .open(
                validate(&record.request)?,
                Some(effective_needs(record)),
                "command-open",
            )
            .await?;
        // The command runs from its open: keep which execution it is before anything else can fail.
        record.advance(StageEvent::Open {
            room: remote.room().into(),
            exec_id: exec_id.clone(),
        })?;
        record.running_at_ms = Some(crate::session::unix_epoch_ms());
        self.save_held_record(record, cancelled).await?;
        let input = command_input(&record.request, environment, &record.job_id)?;
        let next = remote.write(&exec_id, 0, input.as_bytes()).await?;
        remote.close_stdin(&exec_id, next).await?;
        self.job_compute_started(machine).await?;
        record.state = "running".into();
        self.save_held_record(record, cancelled).await?;
        self.supervise_remote(record, &remote, &plan, &exec_id, output, cancelled)
            .await
    }

    /// After a restart: reattaches to the command the record names and waits for its end, so no
    /// file is accepted and the worker is not released while it still runs.
    async fn resume_remote_job(
        &self,
        record: &mut Record,
        output: &mut Output,
        cancelled: &mut watch::Receiver<bool>,
    ) -> Result<JobResult, DaemonError> {
        let RemoteStage::Running { room, exec_id } = record.remote.clone() else {
            return Err(ledger::error("The job has no running command to resume"));
        };
        let result = async {
            let plan = PendingReturn::load(self, &record.job_id)?;
            let since = tokio::time::Instant::now();
            let grant = loop {
                match self.return_grant(record, &plan.kernel_id).await {
                    Ok(grant) => break grant,
                    Err(error) if *cancelled.borrow() || since.elapsed() >= WORKER_PATIENCE => {
                        return Err(error)
                    }
                    Err(error) => {
                        tracing::warn!(%error, "job worker not reachable after restart; retrying");
                        tokio::time::sleep(Duration::from_secs(2)).await
                    }
                }
            };
            if let Some(driver) = &self.job_wake_driver {
                driver
                    .return_grants
                    .lock()
                    .expect("job return grants")
                    .insert(record.job_id.clone(), grant.clone());
            }
            let remote = Remote::new(grant, room, record.job_id.clone());
            self.supervise_remote(record, &remote, &plan, &exec_id, output, cancelled)
                .await
        }
        .await;
        let was_cancelled = *cancelled.borrow();
        self.complete_remote_job(record, output, result, was_cancelled)
            .await
    }

    /// Saves the record of a job whose worker holds its command or files. Local storage can come
    /// back, so a failed save waits for it instead of giving the worker up; only a stop ends it.
    async fn save_held_record(
        &self,
        record: &Record,
        cancelled: &watch::Receiver<bool>,
    ) -> Result<(), DaemonError> {
        loop {
            match save_record(self, record) {
                Ok(()) => return Ok(()),
                Err(error) if *cancelled.borrow() => return Err(error),
                Err(error) => {
                    tracing::warn!(%error, job_id = %record.job_id, "job record not saved; retrying");
                    tokio::time::sleep(LOCAL_STORAGE_RETRY).await;
                }
            }
        }
    }

    /// Follows the command on the worker to its end, then brings its files home.
    async fn supervise_remote(
        &self,
        record: &mut Record,
        remote: &Remote,
        plan: &PendingReturn,
        exec_id: &str,
        output: &mut Output,
        cancelled: &mut watch::Receiver<bool>,
    ) -> Result<JobResult, DaemonError> {
        let started = record
            .running_at_ms
            .unwrap_or_else(crate::session::unix_epoch_ms);
        let left = (started + record.request.timeout_s * 1000)
            .saturating_sub(crate::session::unix_epoch_ms());
        let deadline = tokio::time::Instant::now() + Duration::from_millis(left);
        let (mut cursor, mut stopped) = (0, None);
        let mut disconnected = None;
        let mut last_samples = tokio::time::Instant::now() - Duration::from_secs(1);
        let outcome = loop {
            if stopped.is_none() && (*cancelled.borrow() || tokio::time::Instant::now() >= deadline)
            {
                stopped = Some(if *cancelled.borrow() {
                    JobOutcome::Cancelled
                } else {
                    JobOutcome::TimeLimit
                });
                if remote.cancel(exec_id).await.is_err() {
                    return uncollected_stop(
                        self,
                        record,
                        output,
                        stopped.expect("requested stop"),
                    );
                }
            }
            if last_samples.elapsed() >= Duration::from_secs(1) {
                if let Ok(mut samples) = remote.samples().await {
                    for sample in &mut samples {
                        sample.owner =
                            crate::runtime::process_resources::SessionProcessOwner::Agent(
                                record.agent_id.clone(),
                            );
                        sample.process.process_id =
                            format!("{}:{}", record.job_id, sample.process.process_id);
                        sample.placement = Some(crate::local::RemoteJobPlacement {
                            machine_label: record.machine_label.clone().unwrap_or_default(),
                            address: record.address.clone(),
                        });
                    }
                    if let Some(driver) = &self.job_wake_driver {
                        driver
                            .remote_samples
                            .lock()
                            .expect("remote job samples")
                            .insert(record.job_id.clone(), samples);
                    }
                    self.owned
                        .session_projection
                        .record_session_change(&record.room_id);
                }
                last_samples = tokio::time::Instant::now();
            }
            match remote.read(exec_id, cursor).await {
                Ok(page) => {
                    if disconnected.take().is_some() {
                        record.state = "running".into();
                        self.save_held_record(record, cancelled).await?;
                    }
                    cursor = page.next_cursor;
                    // Lost log lines are not worth the command or its files.
                    if let Err(error) = output.page(self, &record.room_id, &page) {
                        tracing::warn!(%error, job_id = %record.job_id, "remote job output not kept");
                    }
                    if let Some(outcome) = page.outcome {
                        break outcome;
                    }
                }
                Err(error) => {
                    if let Some(outcome) = stopped {
                        return uncollected_stop(self, record, output, outcome);
                    }
                    let since = disconnected.get_or_insert_with(tokio::time::Instant::now);
                    if since.elapsed() >= WORKER_PATIENCE {
                        return Err(error);
                    }
                    record.state = "reconnecting".into();
                    self.save_held_record(record, cancelled).await?;
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        };
        note_failed_end(self, record, output, &outcome)?;
        let mut result = result_from_exit(record, outcome, stopped);
        // The command's end is saved before any of its files are accepted.
        record.advance(StageEvent::End(result.outcome))?;
        record.state = "returning".into();
        self.save_held_record(record, cancelled).await?;
        match remote
            .return_files(&plan.root, record, &plan.base, &plan.uploaded)
            .await
        {
            Ok((files, conflicts)) => {
                result.files_back = files;
                result.conflicts = conflicts;
                record.advance(StageEvent::Home)?;
            }
            Err(error) => {
                let notice = format!(
                    "The command finished, but its files could not all be returned: {error}. {}",
                    kept_worker_notice(record)
                );
                if let Err(error) = output.push(self, &record.room_id, notice.as_bytes(), true) {
                    tracing::warn!(%error, job_id = %record.job_id, "remote job output not kept");
                }
                result.outcome = JobOutcome::OutsideEnd;
            }
        }
        Ok(result)
    }
}

/// How often a held job tries its local storage again after a write failed.
const LOCAL_STORAGE_RETRY: Duration = Duration::from_millis(if cfg!(test) { 100 } else { 2000 });

/// How long a worker may stay out of reach, while its command runs, before it is stopped and reaped.
const WORKER_PATIENCE: Duration = Duration::from_secs(if cfg!(test) { 3 } else { 600 });

pub(super) const HELD_COMMAND_MAY_RUN: &str = "The command may still run on the held machine; stop it from your Room before you release the machine.\n";

fn kept_worker_notice(record: &Record) -> &'static str {
    if record.request.machine.is_some() {
        "The files the command left on the held machine are kept so none are lost. Check this job with chariox.jobs.status to collect them; chariox.jobs.cancel gives them up. The machine stays yours: release it with chariox.machines.release when you are done.\n"
    } else {
        "The remote machine is kept so none are lost. Check this job with chariox.jobs.status to collect them; chariox.jobs.cancel gives them up. The machine is deleted once they are home, or when its lease ends.\n"
    }
}

pub(super) fn command_input(
    request: &RunJobRequest,
    values: &BTreeMap<String, Zeroizing<String>>,
    job_id: &str,
) -> Result<Zeroizing<String>, DaemonError> {
    let environment: BTreeMap<_, _> = values
        .iter()
        .map(|(key, value)| (key, value.as_str()))
        .collect();
    let mut input = Zeroizing::new(
        serde_json::to_string(
            &serde_json::json!({"argv":validate(request)?,"env":environment,"job_id":job_id}),
        )
        .map_err(|_| ledger::error("Cannot prepare job environment"))?,
    );
    input.push('\n');
    if input.len() > crate::room_exec::CHUNK_BYTES {
        return Err(ledger::error(
            "Selected project environment exceeds the job input limit",
        ));
    }
    Ok(input)
}
fn is_capacity_failure(error: &DaemonError) -> bool {
    match error {
        DaemonError::MachineRequest { code, .. } if code == "control_plane_unavailable" => true,
        DaemonError::RelayTransport { code, .. } | DaemonError::MachineRequest { code, .. } => {
            matches!(
                code.as_str(),
                "capacity_unavailable" | "limit_reached" | "out_of_capacity" | "boot_failed"
            )
        }
        _ => false,
    }
}
/// The sentence a job prints when the machine service cannot give it a machine and it runs here instead.
fn local_fallback_notice(error: &DaemonError) -> Option<&'static str> {
    if is_capacity_failure(error) {
        return Some("No remote machine free; waiting locally.\n");
    }
    match crate::runtime::cloud_api_client::cloud_error_code(error)? {
        "rate_limited" => Some("Remote machines are rate limited right now; waiting locally.\n"),
        "dependency_unavailable" => {
            Some("The remote machine service is unavailable right now; waiting locally.\n")
        }
        "internal_error" => {
            Some("The remote machine service had an internal error; waiting locally.\n")
        }
        _ => None,
    }
}
pub(super) fn cancelled_result(record: &Record) -> JobResult {
    let mut result = failed(record, String::new());
    result.outcome = JobOutcome::Cancelled;
    result
}
fn failed(record: &Record, message: String) -> JobResult {
    JobResult {
        exit_code: None,
        output_tail: message,
        log: String::new(),
        files_back: vec![],
        conflicts: vec![],
        ran_on: JobRanOn {
            machine: record
                .machine_label
                .clone()
                .unwrap_or_else(|| "this machine".into()),
            ip: record.address.clone(),
        },
        duration_ms: 0,
        outcome: if record.allocation_ref.is_some() {
            JobOutcome::MachineLost
        } else {
            JobOutcome::OutsideEnd
        },
    }
}
fn uncollected_stop(
    state: &KernelRuntimeState,
    record: &mut Record,
    output: &mut Output,
    outcome: JobOutcome,
) -> Result<JobResult, DaemonError> {
    // Only a command that was opened can still run on a held machine.
    let command_opened = matches!(record.remote, RemoteStage::Running { .. });
    state.give_up_pending_return(record);
    let notice = if outcome == JobOutcome::TimeLimit {
        "Time limit reached. Command output is partial and returned files are unavailable.\n"
    } else {
        "Job cancelled. Command output is partial and returned files are unavailable.\n"
    };
    output.notice(state, &record.room_id, notice)?;
    if record.request.machine.is_some() && command_opened {
        output.notice(state, &record.room_id, HELD_COMMAND_MAY_RUN)?;
    }
    let mut result = failed(record, String::new());
    result.outcome = outcome;
    Ok(result)
}
/// A command that ended without an exit status says why in its output, so the job never ends
/// with no exit code and no reason.
pub(super) fn note_failed_end(
    state: &KernelRuntimeState,
    record: &Record,
    output: &mut Output,
    exit: &ExecOutcome,
) -> Result<(), DaemonError> {
    if let Some(notice) = failed_end_notice(exit) {
        output.notice(state, &record.room_id, &notice)?;
    }
    Ok(())
}
pub(super) fn failed_end_notice(exit: &ExecOutcome) -> Option<String> {
    match exit {
        ExecOutcome::Failed { message } => Some(format!(
            "The command ended without an exit status: {message}\n"
        )),
        _ => None,
    }
}
pub(super) fn result_from_exit(
    record: &Record,
    exit: ExecOutcome,
    stopped: Option<JobOutcome>,
) -> JobResult {
    let mut result = failed(record, String::new());
    let signaled = matches!(&exit, ExecOutcome::Signaled { .. });
    let memory_limited = matches!(&exit, ExecOutcome::MemoryLimit { .. });
    result.exit_code = match exit {
        ExecOutcome::Exited { code } => Some(code),
        ExecOutcome::Signaled { signal } => Some(128 + signal),
        ExecOutcome::MemoryLimit { .. } => Some(137),
        ExecOutcome::Failed { .. } => None,
    };
    result.outcome = stopped.unwrap_or(if memory_limited {
        JobOutcome::MemoryLimit
    } else if result.exit_code.is_some() && !signaled {
        JobOutcome::Completed
    } else {
        JobOutcome::OutsideEnd
    });
    result
}
