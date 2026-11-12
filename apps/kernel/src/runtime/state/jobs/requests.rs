use super::*;

impl KernelRuntimeState {
    pub(in crate::runtime::state) async fn dispatch_job_tool(
        &self,
        run: Option<&crate::provider::RuntimeProviderRun>,
        name: &str,
        args: &serde_json::Value,
        task: &AgentTaskExecution,
    ) -> Result<serde_json::Value, DaemonError> {
        let driver = self
            .job_wake_driver
            .as_ref()
            .ok_or_else(|| ledger::error("job service unavailable"))?;
        if name != "chariox.jobs.run" {
            let id = args["job_id"]
                .as_str()
                .ok_or_else(|| ledger::error("job_id required"))?;
            let record = driver.own(id, task)?;
            if name == "chariox.jobs.cancel" && record.result.is_none() {
                let wake = self
                    .owned
                    .durable_state_store
                    .agent_wake(&record.wake_id)?
                    .ok_or_else(|| ledger::error("job completion source missing"))?;
                self.cancel_agent_wake(&wake, &task.task_id, Some(&task.prompt_id))
                    .await?;
            }
            return public_record(driver.read(id)?);
        }
        let run =
            run.ok_or_else(|| ledger::error("jobs require the home agent's active build run"))?;
        let run = self.watched_process_run(run.id())?;
        if run.permission_level() != crate::provider::AgentPermissionLevel::Yolo
            && !self.sudo_task_authorizes_run(&run).await
        {
            return Err(ledger::error(
                "jobs require an unrestricted build agent with unattended command permission",
            ));
        }
        let mut arguments = args.clone();
        let object = arguments
            .as_object_mut()
            .ok_or_else(|| ledger::error("job arguments must be an object"))?;
        object.remove("task_id");
        object.remove("origin_prompt_id");
        let request: RunJobRequest = serde_json::from_value(arguments)
            .map_err(|e| ledger::error(format!("Invalid job request: {e}")))?;
        let command = validate(&request)?;
        let workspace = run
            .working_directory()
            .ok_or_else(|| ledger::error("job workspace missing"))?
            .canonicalize()
            .map_err(|e| ledger::error(e.to_string()))?;
        let cwd = super::agent_process_watch::workspace_cwd(&run, request.cwd.as_deref())?;
        let mut wake = super::tool_dispatch::agent_wakes::new_wake(
            task,
            KIND,
            super::agent_process_output::sanitize(
                crate::runtime::process_resources::command_line(&command).as_bytes(),
            )
            .chars()
            .take(200)
            .collect(),
            crate::session::unix_epoch_ms(),
        );
        wake.state = "starting".into();
        wake.command = command;
        self.approve_agent_process(&run, &wake, &cwd, &task.prompt_id)
            .await?;
        self.authorize_current_external_command()?;
        let id = format!("job-{:032x}", rand::random::<u128>());
        let mut record = Record {
            job_id: id.clone(),
            wake_id: wake.id.clone(),
            registration_id: wake.registration_id.clone(),
            room_id: task.room_id.clone(),
            agent_id: task.agent_id.clone(),
            request: request.clone(),
            workspace,
            cwd,
            placement: JobPlacement::Local,
            started_at_ms: crate::session::unix_epoch_ms(),
            running_at_ms: None,
            state: "starting".into(),
            allocation_ref: None,
            capability_revision: None,
            machine_label: None,
            address: None,
            result: None,
            exec_id: None,
            class_id: None,
            rate_per_minute: None,
            estimate_micros: None,
            deleted: false,
            remote_available: false,
            remote_refusal: None,
            estimated_minutes: None,
            memory_mib: None,
        };
        if let Some(reach) =
            self.reach_for_id(&format!("{}{}", super::reach::REACH_PREFIX, run.id()))
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
        let remote = match self.effective_remote_jobs_policy(&record.room_id, &record.agent_id)? {
            crate::local::OffloadPolicy::Off => false,
            crate::local::OffloadPolicy::WhenBusy => busy,
            crate::local::OffloadPolicy::AlwaysHeavy => heavy,
        };
        if remote {
            record.placement = JobPlacement::Remote;
        }
        wake.detail = serde_json::json!({"job_id":id});
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
        let deadline = tokio::time::Instant::now() + Duration::from_secs(request.wait_s);
        loop {
            let record = driver.read(&id)?;
            if record.result.is_some() || tokio::time::Instant::now() >= deadline {
                return public_record(record);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

pub(super) fn validate(request: &RunJobRequest) -> Result<Vec<String>, DaemonError> {
    if request.wait_s > 600 || request.timeout_s == 0 || request.timeout_s > 86_400 {
        return Err(ledger::error("wait_s must be 0-600 and timeout_s 1-86400"));
    }
    if request.needs.cpus == Some(0) || request.needs.memory_gib == Some(0) {
        return Err(ledger::error("Job resource needs must be positive"));
    }
    if let Some(cwd) = &request.cwd {
        crate::jobs::relative(cwd)?;
    }
    for output in &request.outputs {
        crate::jobs::relative(output)?;
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
