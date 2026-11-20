use super::*;

impl KernelRuntimeState {
    pub(in crate::runtime::state) async fn dispatch_job_tool(
        &self,
        run: Option<&crate::provider::RuntimeProviderRun>,
        name: &str,
        args: &serde_json::Value,
        task: &AgentTaskExecution,
    ) -> Result<serde_json::Value, DaemonError> {
        let driver = self.job_driver()?;
        if name == "chariox.jobs.list" {
            return Ok(self.own_job_list(task));
        }
        if name != "chariox.jobs.run" {
            let id = args["job_id"]
                .as_str()
                .ok_or_else(|| ledger::error("job_id required"))?;
            let mut record = driver.own(id, task)?;
            let mut collection_error = None;
            let mut release_notice = None;
            if record.state == "finished" && !record.deleted {
                match self.held_job_release(&record).await {
                    Some(release @ super::held::HeldRelease::Deleted) => {
                        self.mark_held_job_deleted(&mut record);
                        if let Some(result) = &mut record.result {
                            result.output_tail.push_str(release.notice());
                        }
                        driver.save(&record)?;
                    }
                    Some(release @ super::held::HeldRelease::Deleting) => {
                        release_notice = Some(release.notice().trim_end());
                    }
                    None => {}
                }
            }

            // Whatever placement the job ended with, a saved allocation receipt keeps its
            // cleanup owed until the service confirms the deletion.
            if record.state == "finished"
                && record.has_remote_trace()
                && !record.deleted
                && release_notice.is_none()
                && (self.remote_jobs_enabled() || record.request.machine.is_some())
            {
                match record.remote {
                    RemoteStage::Finished { .. } if name == "chariox.jobs.cancel" => {
                        self.give_up_pending_return(&mut record);
                        driver.save(&record)?;
                    }
                    RemoteStage::Finished { .. } => {
                        match tokio::time::timeout(
                            Duration::from_secs(120),
                            self.collect_pending_return(&mut record),
                        )
                        .await
                        {
                            Ok(Ok(())) => {}
                            Ok(Err(error)) => collection_error = Some(error.to_string()),
                            Err(_) => {
                                collection_error = Some("Collecting the files timed out".into())
                            }
                        }
                    }
                    _ => {}
                }
                // Retries the removal of the workspace a held machine still keeps for the job.
                if record.request.machine.is_some() && !record.remote.holds_worker() {
                    let _ =
                        tokio::time::timeout(Duration::from_secs(30), self.settle_or_warn(&record))
                            .await;
                }
                if let (Some(allocation), Some(revision), false) = (
                    &record.allocation_ref,
                    &record.capability_revision,
                    record.remote.holds_worker(),
                ) {
                    // Status stays responsive while the service is unavailable;
                    // repeated reads can reconcile a confirmed deletion later.
                    if matches!(
                        tokio::time::timeout(
                            Duration::from_secs(3),
                            self.job_compute_release_ref(allocation, revision)
                        )
                        .await,
                        Ok(Ok(()))
                    ) {
                        record.deleted = true;
                        driver.save(&record)?;
                    }
                }
            }
            if name == "chariox.jobs.cancel" && record.result.is_none() {
                let wake = self
                    .owned
                    .durable_state_store
                    .agent_wake(&record.wake_id)?
                    .ok_or_else(|| ledger::error("job completion source missing"))?;
                self.cancel_agent_wake(&wake, &task.task_id, Some(&task.prompt_id))
                    .await?;
            }
            let mut value = self.job_response(driver.read(id)?)?;
            if let Some(error) = collection_error {
                value["return_error"] = error.into();
            }
            if let Some(notice) = release_notice {
                value["machine_status"] = notice.into();
            }
            return Ok(value);
        }
        let mut request = parse_request(args)?;
        let wake = new_job_wake(task, &request)?;
        let Some(run) = run else {
            return self.start_placed_job(task, wake, request).await;
        };
        let run = self.job_build_run(run.id())?;
        let machine = match &mut request.machine {
            Some(JobMachineTarget::Allocation(allocation)) => {
                Some(self.held_machine_for_run(&run, allocation).await?)
            }
            Some(JobMachineTarget::Enrolled { kernel, .. }) => {
                self.check_enrolled_job_agent(&task.agent_id, kernel)?;
                let harness = self
                    .agent_harness_name(&task.room_id, &task.agent_id)
                    .unwrap_or_else(|| "Chariox".into());
                let machine = self.enrolled_job_machine(kernel, &harness).await?;
                *kernel = machine.kernel_id.clone();
                Some(machine)
            }
            None => None,
        };
        let (workspace, cwd) = job_workspace(&run, &request)?;
        self.approve_agent_process(&run, &wake, &cwd, &task.prompt_id)
            .await?;
        self.authorize_current_external_command()?;
        let mut record = Record::new(&wake, request.clone(), workspace, cwd);
        let id = record.job_id.clone();
        if let Some(machine) = &machine {
            record.placement = JobPlacement::Remote;
            record.machine_label = Some(machine.machine_label.clone());
            record.address = machine.address.clone();
        }
        if let Some(reach) = (self.remote_jobs_enabled() && machine.is_none())
            .then(|| self.reach_for_id(&format!("{}{}", super::reach::REACH_PREFIX, run.id())))
            .flatten()
        {
            if let Ok(Some(class)) = tokio::time::timeout(
                Duration::from_secs(3),
                self.job_compute_quote(&effective_needs(&record), &reach.origin()),
            )
            .await
            {
                record.estimated_minutes = self
                    .job_duration_estimate(&record)
                    .map(|duration| duration.saturating_add(59_999) / 60_000);
                record.estimate_micros = record
                    .estimated_minutes
                    .map(|minutes| class.rate_per_minute.amount_micros.saturating_mul(minutes));
                record.memory_mib = Some(class.usable_memory_mib);
                record.class_id = Some(class.class_id);
                record.rate_per_minute = Some(class.rate_per_minute);
                record.remote_available = true;
            }
        }
        let admission = crate::runtime::process_resources::LaunchAdmission::default();
        let busy = admission.would_wait(
            &format!("{}:placement", id),
            &wake.command,
            &record.cwd,
            u64::from(record.request.needs.memory_gib.unwrap_or(0)) * 1024 * 1024 * 1024,
        );
        let heavy = record.request.needs.memory_gib.is_some()
            || record.request.needs.cpus.is_some_and(|cpus| cpus > 1)
            || admission.is_heavy(&wake.command, &record.cwd);
        let remote = self.remote_jobs_enabled()
            && match self.effective_remote_jobs_policy(&record.room_id, &record.agent_id)? {
                crate::local::OffloadPolicy::Off => false,
                crate::local::OffloadPolicy::WhenBusy => busy,
                crate::local::OffloadPolicy::AlwaysHeavy => heavy,
            };
        if remote {
            record.placement = JobPlacement::Remote;
        }
        driver.save(&record)?;
        driver.pending.lock().expect("job pending").insert(
            wake.id.clone(),
            Pending {
                state: self.clone(),
                record,
            },
        );
        if let Err(error) = self.arm_wake_source(task, &wake) {
            driver.pending.lock().expect("job pending").remove(&wake.id);
            return Err(error);
        }
        self.refresh_wake_projection(&task.room_id);
        self.wait_for_job(&id, request.wait_s).await
    }

    pub(super) fn job_driver(&self) -> Result<&JobDriver, DaemonError> {
        self.job_wake_driver
            .as_deref()
            .ok_or_else(|| ledger::error("job service unavailable"))
    }

    pub(super) fn job_build_run(
        &self,
        id: &str,
    ) -> Result<crate::provider::RuntimeProviderRun, DaemonError> {
        self.watched_process_run(id).map_err(|_| {
            ledger::error("Jobs require an active unrestricted Build turn on the execution kernel")
        })
    }

    pub(super) async fn wait_for_job(
        &self,
        id: &str,
        wait_s: u64,
    ) -> Result<serde_json::Value, DaemonError> {
        let driver = self.job_driver()?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(wait_s);
        loop {
            let record = driver.read(id)?;
            if record.state == "finished" || tokio::time::Instant::now() >= deadline {
                return self.job_response(record);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

fn parse_request(args: &serde_json::Value) -> Result<RunJobRequest, DaemonError> {
    let mut arguments = args.clone();
    let object = arguments
        .as_object_mut()
        .ok_or_else(|| ledger::error("job arguments must be an object"))?;
    object.remove("task_id");
    object.remove("origin_prompt_id");
    serde_json::from_value(arguments)
        .map_err(|error| ledger::error(format!("Invalid job request: {error}")))
}

fn new_job_wake(
    task: &AgentTaskExecution,
    request: &RunJobRequest,
) -> Result<AgentWake, DaemonError> {
    let command = validate(request)?;
    let mut wake = super::super::tool_dispatch::agent_wakes::new_wake(
        task,
        KIND,
        super::super::agent_process_output::sanitize(
            crate::runtime::process_resources::command_line(&command).as_bytes(),
        )
        .chars()
        .take(200)
        .collect(),
        crate::session::unix_epoch_ms(),
    );
    wake.state = "starting".into();
    wake.command = command;
    wake.detail = serde_json::json!({"job_id": format!("job-{:032x}", rand::random::<u128>())});
    Ok(wake)
}

pub(super) fn job_workspace(
    run: &crate::provider::RuntimeProviderRun,
    request: &RunJobRequest,
) -> Result<(PathBuf, PathBuf), DaemonError> {
    let workspace = super::super::agent_process_watch::workspace_cwd(run, None)?;
    let cwd = match request.target_directory() {
        Some(directory) => Path::new(directory).join(request.cwd.as_deref().unwrap_or("")),
        None => super::super::agent_process_watch::workspace_cwd(run, request.cwd.as_deref())?,
    };
    Ok((workspace, cwd))
}

pub(super) fn validate(request: &RunJobRequest) -> Result<Vec<String>, DaemonError> {
    if request.wait_s > 600 || request.timeout_s == 0 || request.timeout_s > 86_400 {
        return Err(ledger::error("wait_s must be 0-600 and timeout_s 1-86400"));
    }
    if request.needs.cpus == Some(0) || request.needs.memory_gib == Some(0) {
        return Err(ledger::error("Job resource needs must be positive"));
    }
    if request.machine_only {
        if request.machine.is_none() {
            return Err(ledger::error("machine_only requires a named machine"));
        }
        if !request.outputs.is_empty()
            || (request.cwd.is_some() && request.target_directory().is_none())
        {
            return Err(ledger::error(
                "machine_only leaves files on the machine. Omit outputs and pass cwd only with a target directory.",
            ));
        }
    }
    if let Some(cwd) = &request.cwd {
        crate::jobs::relative(cwd)?;
    }
    for output in &request.outputs {
        crate::jobs::relative(output)?;
    }
    match &request.machine {
        Some(JobMachineTarget::Allocation(allocation))
            if allocation.is_empty()
                || allocation.len() > 200
                || allocation
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control()) =>
        {
            return Err(ledger::error(
                "Use the allocation_ref chariox.machines.acquire returned.",
            ));
        }
        Some(JobMachineTarget::Enrolled { kernel, .. })
            if kernel.trim().is_empty()
                || kernel.len() > 200
                || kernel.chars().any(char::is_control) =>
        {
            return Err(ledger::error(
                "Name the enrolled machine by kernel ID or name.",
            ));
        }
        _ => {}
    }
    if let Some(directory) = request.target_directory() {
        if !Path::new(directory).is_absolute() || directory.contains('\0') {
            return Err(ledger::error(
                "Use an absolute directory on the enrolled machine.",
            ));
        }
        if !request.outputs.is_empty() {
            return Err(ledger::error(
                "Jobs in an existing machine directory leave their files there. Use chariox.files.copy to bring them back.",
            ));
        }
    }
    if request.env.len() > 128
        || request.env.iter().any(|name| {
            name.is_empty()
                || name.len() > 200
                || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                || name.as_bytes()[0].is_ascii_digit()
        })
    {
        return Err(ledger::error("env accepts project environment names only"));
    }
    let command = match &request.command {
        crate::local::JobCommand::Argv(argv) => argv.clone(),
        crate::local::JobCommand::Shell(line) => vec!["/bin/sh".into(), "-lc".into(), line.clone()],
    };
    if command.is_empty()
        || command.len() > 64
        || command[0].is_empty()
        || command.iter().any(|s| s.len() > 4096 || s.contains('\0'))
    {
        return Err(ledger::error(
            "command requires 1-64 arguments, each at most 4096 bytes",
        ));
    }
    Ok(command)
}
