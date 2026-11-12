use super::*;

impl KernelRuntimeState {
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

    pub(in crate::runtime::state) fn job_record_allocation(
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

    pub(in crate::runtime::state) fn job_record_deleted(
        &self,
        job_id: &str,
    ) -> Result<(), DaemonError> {
        let driver = self
            .job_wake_driver
            .as_ref()
            .ok_or_else(|| ledger::error("Job service unavailable"))?;
        let mut record = driver.read(job_id)?;
        record.deleted = true;
        save_record(self, &record)
    }

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
        self.authorize_current_external_command()?;
        if caller.caller_id.starts_with(super::reach::REACH_PREFIX) {
            let reach = self
                .reach_for_id(&caller.caller_id)
                .ok_or_else(|| ledger::error("The agent's job-control authority ended"))?;
            if reach.agent_id != record.agent_id {
                return Err(ledger::error("An agent controls only its own job"));
            }
        } else {
            self.ensure_room_control(
                room,
                caller
                    .user_id
                    .as_deref()
                    .unwrap_or(crate::session::DEFAULT_LOCAL_USER_ID),
            )?;
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

    pub(crate) fn job_acquisition_subject(
        &self,
        job_id: &str,
    ) -> Result<(String, String, String), DaemonError> {
        let driver = self
            .job_wake_driver
            .as_ref()
            .ok_or_else(|| ledger::error("Job service unavailable"))?;
        let record = driver.read(job_id)?;
        if record.result.is_some() {
            return Err(ledger::error("This job has already ended"));
        }
        let wake = self
            .owned
            .durable_state_store
            .agent_wake(&record.wake_id)?
            .ok_or_else(|| ledger::error("Job completion source missing"))?;
        if !matches!(wake.state.as_str(), "starting" | "running")
            || driver
                .live
                .lock()
                .expect("job live")
                .get(&record.wake_id)
                .is_none_or(|cancel| *cancel.borrow())
        {
            return Err(ledger::error("This job has been cancelled"));
        }
        let task = self
            .owned
            .durable_state_store
            .agent_task(&wake.task_id)?
            .ok_or_else(|| ledger::error("Job task missing"))?;
        if matches!(
            task.state,
            ledger::ExecutionState::Cancelled | ledger::ExecutionState::Done
        ) {
            return Err(ledger::error("This job's task ended"));
        }
        let agent = self.owned.agent_store.get_agent(&record.agent_id)?;
        if agent.archived() || agent.session_id() != record.room_id {
            return Err(ledger::error(
                "The job's agent is no longer active in this Room",
            ));
        }
        self.ensure_room_running(&record.room_id)?;
        let session = self.owned.session_store.get_session(&record.room_id)?;
        if session.owner_user_id() != task.owner_user_id {
            return Err(ledger::error("The job's Room ownership changed"));
        }
        Ok((
            record.room_id,
            record.agent_id,
            session.owner_user_id().to_owned(),
        ))
    }
}
