//! MP-08 / MP-09 / MP-10 / MP-11: durable commands, independent of a provider tool timeout.
use super::wake_driver::{Cancel, Liveness, Recovery, WakeDriver, WakeOccurrence};
use super::*;
use crate::durable_state::agent_lifecycle::{
    self as ledger, AgentTaskExecution, AgentWake, WakeEnd,
};
use crate::local::{JobOutcome, JobPlacement, JobRanOn, JobResult, RunJobRequest};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

mod execute;
mod remote;
#[cfg(test)]
mod tests;

pub(super) const KIND: &str = "job";
const TAIL_BYTES: usize = 64 * 1024;

#[derive(Clone, Serialize, Deserialize)]
struct Record {
    job_id: String,
    wake_id: String,
    registration_id: String,
    room_id: String,
    agent_id: String,
    request: RunJobRequest,
    workspace: PathBuf,
    cwd: PathBuf,
    placement: JobPlacement,
    started_at_ms: u64,
    state: String,
    allocation_ref: Option<String>,
    capability_revision: Option<String>,
    machine_label: Option<String>,
    address: Option<String>,
    result: Option<JobResult>,
    #[serde(default)]
    exec_id: Option<String>,
    #[serde(default)]
    class_id: Option<String>,
    #[serde(default)]
    rate_per_minute: Option<crate::local::JobMachineRate>,
    #[serde(default)]
    estimate_micros: Option<u64>,
    #[serde(default)]
    deleted: bool,
    #[serde(default)]
    remote_available: bool,
    #[serde(default)]
    remote_refusal: Option<String>,
}

struct Pending {
    state: KernelRuntimeState,
    record: Record,
    run: crate::provider::RuntimeProviderRun,
}

pub(super) struct JobDriver {
    state: KernelRuntimeState,
    pending: std::sync::Mutex<BTreeMap<String, Pending>>,
    live: Arc<std::sync::Mutex<BTreeMap<String, watch::Sender<bool>>>>,
    remote_requested: std::sync::Mutex<BTreeSet<String>>,
    owner_stops: std::sync::Mutex<BTreeSet<String>>,
    remote_samples:
        std::sync::Mutex<BTreeMap<String, Vec<crate::runtime::process_resources::SessionProcess>>>,
    remote_origins: std::sync::Mutex<BTreeMap<String, crate::runtime::kernel_gateway::Origin>>,
}

impl JobDriver {
    fn path(&self, id: &str) -> Result<PathBuf, DaemonError> {
        if !id.starts_with("job-") || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err(ledger::error("invalid job_id"));
        }
        Ok(self
            .state
            .owned
            .config_projection
            .snapshot()
            .private_runtime_state_root()
            .join("jobs")
            .join(format!("{id}.json")))
    }
    fn read(&self, id: &str) -> Result<Record, DaemonError> {
        let bytes = std::fs::read(self.path(id)?)
            .map_err(|_| ledger::error("this agent has no such job"))?;
        serde_json::from_slice(&bytes).map_err(|_| ledger::error("job record is unreadable"))
    }
    fn save(&self, record: &Record) -> Result<(), DaemonError> {
        save_record(&self.state, record)
    }
    fn own(&self, id: &str, task: &AgentTaskExecution) -> Result<Record, DaemonError> {
        let record = self.read(id)?;
        if record.room_id != task.room_id || record.agent_id != task.agent_id {
            return Err(ledger::error("this agent has no such job"));
        }
        Ok(record)
    }
}

impl WakeDriver for JobDriver {
    fn kind(&self) -> &'static str {
        KIND
    }
    fn arm(&self, wake: &AgentWake) -> Result<(), DaemonError> {
        let pending = self
            .pending
            .lock()
            .expect("job pending")
            .remove(&wake.id)
            .ok_or_else(|| ledger::error("job launch intent missing"))?;
        let (cancel, cancelled) = watch::channel(false);
        self.live
            .lock()
            .expect("job live")
            .insert(wake.id.clone(), cancel);
        self.state.wake_sink(KIND).verified(&wake.id)?;
        let live = self.live.clone();
        let wake = wake.clone();
        tokio::spawn(async move {
            pending
                .state
                .execute_job(wake.clone(), pending.record, pending.run, cancelled)
                .await;
            live.lock().expect("job live").remove(&wake.id);
        });
        Ok(())
    }
    fn cancel(&self, wake: &AgentWake) -> Result<Cancel, DaemonError> {
        if let Some(cancel) = self.live.lock().expect("job live").get(&wake.id) {
            cancel.send_replace(true);
            Ok(Cancel::Pending)
        } else {
            Ok(Cancel::Done)
        }
    }
    fn recover(&self, wake: &AgentWake) -> Recovery {
        let Some(id) = wake.detail["job_id"].as_str() else {
            return Recovery::Lost("job identity missing".into());
        };
        let Ok(mut record) = self.read(id) else {
            return Recovery::Lost("job record missing".into());
        };
        let state = self.state.clone();
        let wake = wake.clone();
        let (cancel, _cancelled) = watch::channel(false);
        self.live
            .lock()
            .expect("job live")
            .insert(wake.id.clone(), cancel);
        let live = self.live.clone();
        tokio::spawn(async move {
            if record.placement == JobPlacement::Local {
                let markers = BTreeMap::from([(record.job_id.clone(), record.started_at_ms)]);
                if let Err(error) = tokio::task::spawn_blocking(move || {
                    super::agent_process_group::reap_orphans(&markers)
                })
                .await
                .unwrap_or_else(|error| Err(std::io::Error::other(error.to_string())))
                {
                    tracing::warn!(%error, "job restart process settlement could not be confirmed");
                }
            }
            if let (Some(allocation), Some(revision)) =
                (&record.allocation_ref, &record.capability_revision)
            {
                if let Err(error) = state.job_compute_release_ref(allocation, revision).await {
                    tracing::warn!(%error, "job restart release pending; service lease remains the cleanup backstop");
                }
            }
            if record.placement == JobPlacement::Remote && record.allocation_ref.is_none() {
                if let Err(error) = state.job_compute_release_job_ref(&record.job_id).await {
                    tracing::warn!(%error, "job restart allocation reconciliation pending");
                }
            }
            let result = record.result.clone().unwrap_or_else(|| JobResult {
                exit_code: None,
                output_tail: "The kernel restarted; this job was not relaunched.".into(),
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
                duration_ms: crate::session::unix_epoch_ms().saturating_sub(record.started_at_ms),
                outcome: JobOutcome::MachineLost,
            });
            state.finish_job(&wake, &mut record, result).await;
            live.lock().expect("job live").remove(&wake.id);
        });
        Recovery::Reattached
    }
    fn liveness(&self, wake: &AgentWake) -> Liveness {
        if self.live.lock().expect("job live").contains_key(&wake.id) {
            Liveness::Live
        } else {
            Liveness::Lost("job supervisor ended without a result".into())
        }
    }
}

impl KernelRuntimeState {
    pub(super) fn register_job_wake_driver(&mut self) {
        let driver = Arc::new(JobDriver {
            state: self.clone(),
            pending: Default::default(),
            live: Default::default(),
            remote_requested: Default::default(),
            owner_stops: Default::default(),
            remote_samples: Default::default(),
            remote_origins: Default::default(),
        });
        let source: Arc<dyn WakeDriver> = driver.clone();
        self.register_wake_driver(Arc::downgrade(&source));
        self.job_wake_driver = Some(driver);
    }

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
        };
        if let Some(reach) =
            self.reach_for_id(&format!("{}{}", super::reach::REACH_PREFIX, run.id()))
        {
            if let Ok(Some(class)) = tokio::time::timeout(
                Duration::from_secs(3),
                self.job_compute_quote(&record.request.needs, &reach.origin()),
            )
            .await
            {
                record.class_id = Some(class.class_id);
                record.rate_per_minute = Some(class.rate_per_minute);
                record.remote_available = true;
            }
        }
        let admission = crate::runtime::process_resources::LaunchAdmission::default();
        let busy = admission
            .reserve(
                &format!("{}:placement", id),
                &wake.command,
                &record.cwd,
                false,
            )
            .is_err();
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
                run,
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

    async fn finish_job(&self, wake: &AgentWake, record: &mut Record, result: JobResult) {
        record.state = "finished".into();
        record.result = Some(result.clone());

        let end = if result.outcome == JobOutcome::MachineLost {
            WakeEnd::Lost
        } else if result.exit_code == Some(0) && result.outcome == JobOutcome::Completed {
            WakeEnd::Succeeded
        } else {
            WakeEnd::Failed
        };
        loop {
            match save_record(self, record).and_then(|()| {
                self.wake_sink(KIND).occurred(WakeOccurrence {
                    wake_id: wake.id.clone(),
                    occurrence_id: format!("{}-finished", wake.id),
                    kind: "job_finished".into(),
                    payload: serde_json::json!({"job_id":record.job_id,"result":result}),
                    summary: Some(format!("{} · {:?}", wake.label, result.outcome)),
                    end: Some(end),
                })
            }) {
                Ok(()) => break,
                Err(error) => {
                    tracing::warn!(%error, "job result retained for durable completion retry");
                    if !self.wake_sink(KIND).awaits_occurrence(&wake.id) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }
}

fn public_record(record: Record) -> Result<serde_json::Value, DaemonError> {
    Ok(
        serde_json::json!({"job_id":record.job_id,"registration_id":record.registration_id,"placement":record.placement,"state":record.state,"machine":record.machine_label,"address":record.address,"class_id":record.class_id,"rate_per_minute":record.rate_per_minute,"estimate_micros":record.estimate_micros,"deleted":record.deleted,"result":record.result}),
    )
}

fn validate(request: &RunJobRequest) -> Result<Vec<String>, DaemonError> {
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

fn save_record(state: &KernelRuntimeState, record: &Record) -> Result<(), DaemonError> {
    let path = state
        .owned
        .config_projection
        .snapshot()
        .private_runtime_state_root()
        .join("jobs")
        .join(format!("{}.json", record.job_id));
    let parent = path.parent().expect("job record parent");
    std::fs::create_dir_all(parent).map_err(|e| ledger::error(e.to_string()))?;
    let temporary = parent.join(format!(".job-{:032x}", rand::random::<u128>()));
    std::fs::write(
        &temporary,
        serde_json::to_vec(record).map_err(|e| ledger::error(e.to_string()))?,
    )
    .and_then(|()| std::fs::rename(temporary, path))
    .map_err(|e| ledger::error(e.to_string()))?;
    state
        .owned
        .session_projection
        .record_session_change(&record.room_id);
    Ok(())
}

impl KernelRuntimeState {
    pub(crate) fn session_jobs(&self, room: &str) -> Vec<crate::local::JobView> {
        let directory = self
            .owned
            .config_projection
            .snapshot()
            .private_runtime_state_root()
            .join("jobs");
        let Ok(entries) = std::fs::read_dir(directory) else {
            return vec![];
        };
        let mut jobs: Vec<_> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|e| e == "json"))
            .filter_map(|entry| std::fs::read(entry.path()).ok())
            .filter_map(|bytes| serde_json::from_slice::<Record>(&bytes).ok())
            .filter(|record| record.room_id == room)
            .map(|record| crate::local::JobView {
                job_id: record.job_id,
                agent_id: record.agent_id,
                command: validate(&record.request)
                    .map(|argv| crate::runtime::process_resources::command_line(&argv))
                    .unwrap_or_else(|_| "Job".into()),
                state: record.state,
                placement: record.placement,
                started_at_ms: record.started_at_ms,
                machine_label: record.machine_label,
                address: record.address,
                class_id: record.class_id,
                rate_per_minute: record.rate_per_minute,
                estimate_micros: record.estimate_micros,
                deleted: record.deleted,
                result: record.result,
            })
            .collect();
        jobs.sort_by_key(|job| job.started_at_ms);
        jobs
    }

    pub(crate) async fn run_session_process_remotely(
        &self,
        session_id: &str,
        caller: &crate::runtime::command::KernelCaller,
        process_id: &str,
    ) -> Result<String, DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        let driver = self
            .job_wake_driver
            .as_ref()
            .ok_or_else(|| ledger::error("Job service unavailable"))?;
        let record = self.session_jobs(session_id).into_iter().filter_map(|view| driver.read(&view.job_id).ok()).find(|record| record.job_id == process_id || record.exec_id.as_deref() == Some(process_id)).ok_or_else(|| ledger::error("Only a queued command started with chariox.jobs.run can move to a remote machine"))?;
        let request = crate::local::RoomTerminalRequest {
            session_id: session_id.into(),
            acting_agent: None,
            host_agent: None,
            operation: crate::local::RoomTerminalOperation::ExecPlace {
                exec_id: String::new(),
                kernel: String::new(),
                directory: None,
            },
        };
        let placer = self.authorize_exec_place(caller, &request)?;
        if matches!(placer.requester, crate::room_exec::Requester::Agent(ref agent) if agent != &record.agent_id)
        {
            return Err(ledger::error("An agent may move only its own queued job"));
        }
        if session.owner_user_id().is_empty()
            || record.result.is_some()
            || record.placement == JobPlacement::Remote
        {
            return Err(ledger::error("This job is no longer waiting locally"));
        }
        let exec_id = record
            .exec_id
            .as_deref()
            .ok_or_else(|| ledger::error("This job is not queued yet; wait for its queued row"))?;
        self.owned
            .room_execs
            .cancel_queued(session_id, exec_id, || {
                driver
                    .remote_origins
                    .lock()
                    .expect("job move origins")
                    .insert(record.job_id.clone(), placer.origin);
                driver
                    .remote_requested
                    .lock()
                    .expect("job move")
                    .insert(record.job_id.clone());
            })?;
        Ok(record.job_id)
    }
}

impl KernelRuntimeState {
    pub(super) fn job_record_allocation(
        &self,
        job_id: &str,
        allocation_ref: &str,
        capability_revision: &str,
    ) -> Result<(), DaemonError> {
        let driver = self
            .job_wake_driver
            .as_ref()
            .ok_or_else(|| ledger::error("Job service unavailable"))?;
        let mut record = driver.read(job_id)?;
        record.allocation_ref = Some(allocation_ref.into());
        record.capability_revision = Some(capability_revision.into());
        save_record(self, &record)
    }
}

impl KernelRuntimeState {
    fn controlled_job(
        &self,
        room: &str,
        caller: &crate::runtime::command::KernelCaller,
        process_id: &str,
    ) -> Result<Option<Record>, DaemonError> {
        if !process_id.starts_with("job-") {
            return Ok(None);
        }
        let job_id = process_id.split(':').next().unwrap_or(process_id);
        let driver = self
            .job_wake_driver
            .as_ref()
            .ok_or_else(|| ledger::error("Job service unavailable"))?;
        let record = driver.read(job_id)?;
        if record.room_id != room {
            return Err(ledger::error("This job belongs to another Room"));
        }
        let request = crate::local::RoomTerminalRequest {
            session_id: room.into(),
            acting_agent: None,
            host_agent: None,
            operation: crate::local::RoomTerminalOperation::ExecPlace {
                exec_id: String::new(),
                kernel: String::new(),
                directory: None,
            },
        };
        let placer = self.authorize_exec_place(caller, &request)?;
        if matches!(placer.requester, crate::room_exec::Requester::Agent(ref agent) if agent != &record.agent_id)
        {
            return Err(ledger::error("An agent controls only its own job"));
        }
        Ok(Some(record))
    }
    pub(crate) fn cancel_session_job(
        &self,
        room: &str,
        caller: &crate::runtime::command::KernelCaller,
        process_id: &str,
    ) -> Result<bool, DaemonError> {
        let Some(record) = self.controlled_job(room, caller, process_id)? else {
            return Ok(false);
        };
        let driver = self.job_wake_driver.as_ref().expect("job driver");
        if record.result.is_none() {
            driver
                .owner_stops
                .lock()
                .expect("job stops")
                .insert(record.job_id);
            if let Some(cancel) = driver.live.lock().expect("job live").get(&record.wake_id) {
                cancel.send_replace(true);
            }
        }
        Ok(true)
    }
    pub(crate) fn start_session_job_now(
        &self,
        room: &str,
        caller: &crate::runtime::command::KernelCaller,
        process_id: &str,
    ) -> Result<bool, DaemonError> {
        let Some(record) = self.controlled_job(room, caller, process_id)? else {
            return Ok(false);
        };
        if record.placement != JobPlacement::Local || record.result.is_some() {
            return Err(ledger::error("This job is not waiting locally"));
        }
        let exec = record
            .exec_id
            .ok_or_else(|| ledger::error("This job is not queued yet"))?;
        self.owned.room_execs.start_now(room, &exec)?;
        Ok(true)
    }
    /// Queued launches have no PID. Their process id is the job receipt, so every control
    /// resolves the job before any operating-system signalling path can be reached.
    pub(crate) fn queued_job_processes(
        &self,
        room: &str,
    ) -> Vec<crate::runtime::process_resources::SessionProcess> {
        use crate::runtime::process_resources::{
            ProcessSample, SessionProcess, SessionProcessOwner, SessionProcessState, Waiting,
        };
        let Some(driver) = &self.job_wake_driver else {
            return vec![];
        };
        let execs = self.owned.room_execs.list(room);
        let now = crate::session::unix_epoch_ms();
        let mut rows = vec![];
        for job in self.session_jobs(room) {
            if job.result.is_some() {
                continue;
            }
            if let Some(samples) = driver
                .remote_samples
                .lock()
                .expect("remote job samples")
                .get(&job.job_id)
            {
                rows.extend(samples.clone());
            }
            let Ok(record) = driver.read(&job.job_id) else {
                continue;
            };
            if record.placement != JobPlacement::Local {
                continue;
            }
            let Some(exec) = execs
                .iter()
                .find(|exec| Some(&exec.exec_id) == record.exec_id.as_ref())
            else {
                continue;
            };
            let crate::room_exec::ExecState::Queued { wait } = &exec.state else {
                continue;
            };
            let position = rows
                .iter()
                .filter(|row| matches!(row.state, SessionProcessState::Held(_)))
                .count() as u32
                + 1;
            rows.push(SessionProcess {
                owner: SessionProcessOwner::Agent(record.agent_id),
                process: ProcessSample {
                    process_id: record.job_id,
                    pid: 0,
                    command: exec.command_line.clone(),
                    resident_set_bytes: 0,
                    cpu_millis: 0,
                    disk_io_bytes: None,
                    disk_written_bytes: None,
                    deleted_open_bytes: 0,
                    outlived: false,
                    sampled_at_ms: now,
                    age_ms: now.saturating_sub(record.started_at_ms),
                },
                state: SessionProcessState::Held(Waiting {
                    reason: wait.reason,
                    position,
                    need_bytes: wait.need_bytes,
                    free_bytes: wait.free_bytes,
                    command_line: exec.command_line.clone(),
                    cwd: Some(record.cwd.to_string_lossy().into_owned()),
                    since_ms: record.started_at_ms,
                }),
                remote_available: record.remote_available,
                remote_refusal: record.remote_refusal,
                estimate: record
                    .rate_per_minute
                    .map(|price| crate::local::RemoteJobEstimate {
                        amount_micros: price.amount_micros,
                        currency: price.currency,
                        kind: "per_minute".into(),
                        memory_mib: u64::from(record.request.needs.memory_gib.unwrap_or(0)) * 1024,
                        minutes: None,
                    }),
                placement: None,
            });
        }
        rows
    }
}
