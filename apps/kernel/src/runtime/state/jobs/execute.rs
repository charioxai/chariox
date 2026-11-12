use super::remote::Remote;
use super::*;
use crate::local::OffloadPolicy;
use crate::room_exec::{ExecOutcome, ExecOutput};
use std::io::Write;
use zeroize::Zeroizing;

/// Output is protected as complete lines, including values that cross relay chunk boundaries.
struct Output {
    pending: Vec<u8>,
    tail: String,
    file: std::fs::File,
    log: String,
    discarding_line: bool,
}

impl Output {
    fn new(state: &KernelRuntimeState, record: &Record) -> Result<Self, DaemonError> {
        let directory = state
            .owned
            .config_projection
            .snapshot()
            .private_runtime_state_root()
            .join("jobs")
            .join(&record.job_id);
        std::fs::create_dir_all(&directory).map_err(|e| ledger::error(e.to_string()))?;
        let path = directory.join("output.log");
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
            self.file
                .write_all(text.as_bytes())
                .map_err(|e| ledger::error(format!("Cannot keep job output: {e}")))?;
            self.tail.push_str(&text);
            if self.tail.len() > TAIL_BYTES {
                let mut start = self.tail.len() - TAIL_BYTES;
                while !self.tail.is_char_boundary(start) {
                    start += 1;
                }
                self.tail.drain(..start);
            }
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
    fn page(
        &mut self,
        state: &KernelRuntimeState,
        room: &str,
        page: &ExecOutput,
    ) -> Result<(), DaemonError> {
        for chunk in &page.chunks {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&chunk.data_base64)
                .map_err(|_| ledger::error("Job output is not valid base64"))?;
            self.push(state, room, &bytes, false)?;
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
        selected.insert(name.to_string(), Zeroizing::new(value));
    }
    Ok(selected)
}

impl KernelRuntimeState {
    pub(super) async fn execute_job(
        &self,
        wake: AgentWake,
        mut record: Record,
        run: crate::provider::RuntimeProviderRun,
        mut cancelled: watch::Receiver<bool>,
    ) {
        let environment = selected_environment(self, &record);
        let mut output = match Output::new(self, &record) {
            Ok(output) => output,
            Err(error) => {
                let result = failed(&record, error.to_string());
                self.finish_job(&wake, &mut record, result).await;
                return;
            }
        };
        let result = match environment {
            Err(error) => Err(error),
            Ok(environment) => {
                self.run_job(
                    &wake,
                    &mut record,
                    &run,
                    &environment,
                    &mut output,
                    &mut cancelled,
                    false,
                )
                .await
            }
        };
        let _ = output.push(self, &record.room_id, &[], true);
        let mut result = result.unwrap_or_else(|error| {
            let _ = output.push(self, &record.room_id, error.to_string().as_bytes(), true);
            failed(&record, "Job failed; see the protected output log".into())
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
        result.duration_ms = crate::session::unix_epoch_ms().saturating_sub(record.started_at_ms);
        self.finish_job(&wake, &mut record, result).await;
    }

    async fn run_job(
        &self,
        wake: &AgentWake,
        record: &mut Record,
        run: &crate::provider::RuntimeProviderRun,
        environment: &BTreeMap<String, Zeroizing<String>>,
        output: &mut Output,
        cancelled: &mut watch::Receiver<bool>,
        force_remote: bool,
    ) -> Result<JobResult, DaemonError> {
        let command = validate(&record.request)?;
        let admission = crate::runtime::process_resources::LaunchAdmission::default();
        let busy = admission
            .reserve(
                &format!("{}:probe", record.job_id),
                &command,
                &record.cwd,
                false,
            )
            .is_err();
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
        let remote = force_remote
            || requested
            || match policy {
                OffloadPolicy::Off => false,
                OffloadPolicy::WhenBusy => busy,
                OffloadPolicy::AlwaysHeavy => heavy,
            };
        if *cancelled.borrow() {
            return Ok(cancelled_result(record));
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
                None => self
                    .reach_for_id(&format!(
                        "{}{}",
                        super::super::reach::REACH_PREFIX,
                        run.id()
                    ))
                    .ok_or_else(|| {
                        ledger::error(
                            "The job's originating build run ended before remote allocation",
                        )
                    })?
                    .origin(),
            };
            record.state = "allocating".into();
            record.placement = JobPlacement::Remote;
            save_record(self, record)?;
            match self
                .job_compute_acquire(
                    &record.job_id,
                    &record.request.needs,
                    record.request.timeout_s,
                    &origin,
                )
                .await
            {
                Ok(machine) => {
                    record.allocation_ref = Some(machine.allocation_ref.clone());
                    record.capability_revision = Some(machine.capability_revision.clone());
                    record.machine_label = Some(machine.machine_label.clone());
                    record.address = machine.address.clone();
                    record.class_id = Some(machine.class_id.clone());
                    record.rate_per_minute = Some(machine.rate_per_minute.clone());
                    // The allocation budget is an upper bound, not a duration estimate.
                    save_record(self, record)?;
                    let result = self
                        .run_remote_job(record, &machine, environment, output, cancelled)
                        .await;
                    record.state = "deleting".into();
                    save_record(self, record)?;
                    let release = self.job_compute_release(&machine, &origin).await;
                    if let Err(error) = release {
                        return Err(ledger::error(format!(
                            "Job finished but remote machine cleanup is pending: {error}"
                        )));
                    }
                    record.deleted = true;
                    save_record(self, record)?;
                    return result;
                }
                Err(error) if is_capacity_failure(&error) => {
                    record.placement = JobPlacement::Local;
                    record.remote_refusal = record.remote_available.then(|| "no_capacity".into());
                    record.state = "queued".into();
                    save_record(self, record)?;
                    if record.remote_available {
                        output.push(
                            self,
                            &record.room_id,
                            b"No remote machine free; waiting locally.\n",
                            false,
                        )?;
                    }
                }
                Err(error) => return Err(error),
            }
        }
        match self
            .run_local_job(wake, record, environment, output, cancelled)
            .await
        {
            Err(DaemonError::MachineRequest { code, .. }) if code == "job_move_remote" => {
                Box::pin(self.run_job(wake, record, run, environment, output, cancelled, true))
                    .await
            }
            result => result,
        }
    }

    async fn run_local_job(
        &self,
        _wake: &AgentWake,
        record: &mut Record,
        environment: &BTreeMap<String, Zeroizing<String>>,
        output: &mut Output,
        cancelled: &mut watch::Receiver<bool>,
    ) -> Result<JobResult, DaemonError> {
        record.placement = JobPlacement::Local;
        let session = self.owned.session_store.get_session(&record.room_id)?;
        let execs = &self.owned.room_execs;
        let input = command_input(&record.request, environment, &record.job_id)?;
        let exec_id = execs.open_job_as(
            &record.room_id,
            session.owner_user_id(),
            &record.cwd,
            &validate(&record.request)?,
            Some(self.clone()),
            Some(record.agent_id.clone()),
        )?;
        record.exec_id = Some(exec_id.clone());
        save_record(self, record)?;
        let next = execs.write(
            &record.room_id,
            &exec_id,
            0,
            &base64::engine::general_purpose::STANDARD.encode(input.as_bytes()),
        )?;
        execs.close_stdin(&record.room_id, &exec_id, next)?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(record.request.timeout_s);
        let (mut cursor, mut stopped) = (0, None);
        loop {
            if stopped.is_none() && (*cancelled.borrow() || tokio::time::Instant::now() >= deadline)
            {
                stopped = Some(if *cancelled.borrow() {
                    JobOutcome::Cancelled
                } else {
                    JobOutcome::TimeLimit
                });
                execs.cancel(&record.room_id, &exec_id)?;
            }
            if let Some(summary) = execs
                .list(&record.room_id)
                .into_iter()
                .find(|summary| summary.exec_id == exec_id)
            {
                let state = if matches!(summary.state, crate::room_exec::ExecState::Queued { .. }) {
                    "queued"
                } else {
                    "running"
                };
                if record.state != state {
                    record.state = state.into();
                    save_record(self, record)?;
                }
            }
            let page = execs
                .read(
                    &record.room_id,
                    &exec_id,
                    cursor,
                    Duration::from_millis(250),
                )
                .await?;
            cursor = page.next_cursor;
            output.page(self, &record.room_id, &page)?;
            if let Some(outcome) = page.outcome {
                execs.close(&record.room_id, &exec_id)?;
                if self.job_wake_driver.as_ref().is_some_and(|driver| {
                    driver
                        .remote_requested
                        .lock()
                        .expect("job move")
                        .remove(&record.job_id)
                }) {
                    record.exec_id = None;
                    return Err(DaemonError::MachineRequest {
                        code: "job_move_remote".into(),
                        message: "Queued job moved".into(),
                    });
                }
                return Ok(result_from_exit(record, outcome, stopped));
            }
        }
    }

    async fn run_remote_job(
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
        let (remote, root) =
            Remote::prepare(machine.grant.clone(), record, &upload, cancelled).await?;
        if *cancelled.borrow() {
            return Ok(cancelled_result(record));
        }
        let exec_id = remote
            .open(validate(&record.request)?, true, "command-open")
            .await?;
        let input = command_input(&record.request, environment, &record.job_id)?;
        let next = remote.write(&exec_id, 0, input.as_bytes()).await?;
        remote.close_stdin(&exec_id, next).await?;
        self.job_compute_started(machine).await?;
        record.state = "running".into();
        save_record(self, record)?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(record.request.timeout_s);
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
                remote.cancel(&exec_id).await?;
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
                            machine_label: machine.machine_label.clone(),
                            address: machine.address.clone(),
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
            match remote.read(&exec_id, cursor).await {
                Ok(page) => {
                    disconnected = None;
                    cursor = page.next_cursor;
                    output.page(self, &record.room_id, &page)?;
                    if let Some(outcome) = page.outcome {
                        break outcome;
                    }
                }
                Err(error) => {
                    let since = disconnected.get_or_insert_with(tokio::time::Instant::now);
                    if since.elapsed() >= Duration::from_secs(600) {
                        return Err(error);
                    }
                    record.state = "reconnecting".into();
                    save_record(self, record)?;
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        };
        let mut result = result_from_exit(record, outcome, stopped);
        record.state = "returning".into();
        save_record(self, record)?;
        let (files, conflicts) = remote.return_files(&root, record, &base, &upload).await?;
        result.files_back = files;
        result.conflicts = conflicts;
        Ok(result)
    }
}

fn command_input(
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
        DaemonError::RelayTransport { code, .. } | DaemonError::MachineRequest { code, .. } => {
            matches!(
                code.as_str(),
                "capacity_unavailable" | "limit_reached" | "out_of_capacity" | "boot_failed"
            )
        }
        _ => false,
    }
}
fn cancelled_result(record: &Record) -> JobResult {
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
        outcome: JobOutcome::MachineLost,
    }
}
fn result_from_exit(record: &Record, exit: ExecOutcome, stopped: Option<JobOutcome>) -> JobResult {
    let mut result = failed(record, String::new());
    let signaled = matches!(&exit, ExecOutcome::Signaled { .. });
    result.exit_code = match exit {
        ExecOutcome::Exited { code } => Some(code),
        ExecOutcome::Signaled { signal } => Some(128 + signal),
        ExecOutcome::Failed { .. } => None,
    };
    result.outcome = stopped.unwrap_or(if result.exit_code.is_some() && !signaled {
        JobOutcome::Completed
    } else {
        JobOutcome::OutsideEnd
    });
    result
}
