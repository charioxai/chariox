use super::*;

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
        self.launch(wake.clone(), pending)
    }
    fn cancel(&self, wake: &AgentWake) -> Result<Cancel, DaemonError> {
        if let Some(cancel) = self.live.lock().expect("job live").get(&wake.id) {
            cancel.send_replace(true);
            Ok(Cancel::Pending)
        } else if self
            .state
            .owned
            .durable_state_store
            .leased_wake_binding(&wake.id)?
            .is_some()
        {
            Ok(Cancel::Pending)
        } else {
            Ok(Cancel::Done)
        }
    }
    fn recover(&self, wake: &AgentWake) -> Recovery {
        if self
            .state
            .owned
            .durable_state_store
            .leased_wake_binding(&wake.id)
            .is_ok_and(|binding| binding.is_some())
        {
            return Recovery::Reattached;
        }
        let Some(id) = wake.detail["job_id"].as_str() else {
            return Recovery::Lost("job identity missing".into());
        };
        let Ok(mut record) = self.read(id) else {
            return Recovery::Lost("job record missing".into());
        };
        let Some(state) = self.recovery_state() else {
            return Recovery::Lost("job service unavailable".into());
        };
        let wake = wake.clone();
        let (cancel, cancelled) = watch::channel(
            (record.placement == JobPlacement::Local || record.request.machine.is_some())
                && wake.state == "cancelling",
        );
        {
            let mut live = self.live.lock().expect("job live");
            if record.placement == JobPlacement::Local && live.contains_key(&wake.id) {
                return Recovery::Reattached;
            }
            live.insert(wake.id.clone(), cancel);
        }
        let local = crate::hot_reload::take_local_job(&record.job_id).and_then(|handover| {
            let active = (record.placement == JobPlacement::Local && record.result.is_none())
                .then(|| self.local.enter())
                .flatten();
            if let Some(active) = active {
                return Some((handover, active));
            }
            handover.discard();
            None
        });
        let live = self.live.clone();
        tokio::spawn(async move {
            if let Some((local, active)) = local {
                match local.restore(&state, &record) {
                    Ok((output, progress)) => {
                        state
                            .execute_job(
                                wake.clone(),
                                record,
                                cancelled,
                                Some((output, active, progress)),
                            )
                            .await;
                        live.lock().expect("job live").remove(&wake.id);
                        return;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "local job could not resume across the kernel reload")
                    }
                }
            }
            // Left by an earlier run with remote jobs on: end the job with its reason and leave
            // its worker, files and allocation exactly as saved.
            // A job on a held machine never depended on the switch: the agent named a machine it holds.
            if !state.remote_jobs_enabled()
                && record.has_remote_trace()
                && record.request.machine.is_none()
            {
                let result = remote_jobs_off_result(&record);
                state.finish_job(&wake, &mut record, result).await;
                live.lock().expect("job live").remove(&wake.id);
                return;
            }
            // The command still runs on its worker: follow it to its end before anything else.
            if matches!(record.remote, RemoteStage::Running { .. }) {
                state
                    .execute_job(wake.clone(), record, cancelled, None)
                    .await;
                live.lock().expect("job live").remove(&wake.id);
                return;
            }
            // A command not yet confirmed open is stopped with its worker, as nothing is known
            // of its files.
            if record.remote == RemoteStage::Planned {
                state.give_up_pending_return(&mut record);
            }
            // A held machine keeps what the job uploaded there: remove it unless files are still to
            // be collected from it. Also finishes a cleanup an earlier run began.
            if record.request.machine.is_some() && !record.remote.holds_worker() {
                state.settle_or_warn(&record).await;
            }
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
            // A worker that still holds files not yet returned stays until they are collected.
            if let (Some(allocation), Some(revision), false) = (
                &record.allocation_ref,
                &record.capability_revision,
                record.remote.holds_worker(),
            ) {
                match state.job_compute_release_ref(allocation, revision).await {
                    Ok(()) => record.deleted = true,
                    Err(error) => {
                        tracing::warn!(%error, "job restart release pending; service lease remains the cleanup backstop")
                    }
                }
            }
            if record.placement == JobPlacement::Remote
                && record.allocation_ref.is_none()
                && record.request.machine.is_none()
                && !record.remote.holds_worker()
            {
                match state.job_compute_release_job_ref(&record.job_id).await {
                    Ok(()) => record.deleted = true,
                    Err(error) => {
                        tracing::warn!(%error, "job restart allocation reconciliation pending")
                    }
                }
            }
            let result = record.result.clone().unwrap_or_else(|| {
                let (output_tail, log) = if record.placement == JobPlacement::Local {
                    super::local::restart_output(&state, &record)
                } else {
                    let reason = if record.remote.holds_worker() {
                        "The kernel restarted after this job's command ended on its remote machine. The machine is kept so no file it produced is lost: check this job with chariox.jobs.status to collect them, or cancel it to give them up."
                    } else {
                        "The kernel restarted; this job was not relaunched."
                    };
                    (reason.into(), String::new())
                };
                JobResult {
                    exit_code: None,
                    output_tail,
                    log,
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
                    outcome: if record.remote.holds_worker() {
                        JobOutcome::OutsideEnd
                    } else {
                        JobOutcome::MachineLost
                    },
                }
            });
            state.finish_job(&wake, &mut record, result).await;
            live.lock().expect("job live").remove(&wake.id);
        });
        Recovery::Reattached
    }
    fn reconcile_ended(&self) {
        let Some(state) = self.recovery_state() else {
            return;
        };
        let jobs = state
            .owned
            .config_projection
            .snapshot()
            .private_runtime_state_root()
            .join("jobs");
        let Ok(entries) = std::fs::read_dir(&jobs) else {
            return;
        };
        // A plan on disk is what is left to clean; only settled held-machine jobs are cleaned here
        // (one whose files are still to be collected keeps its plan for chariox.jobs.status).
        let owed: Vec<Record> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().join("return.json").is_file())
            .filter_map(|entry| self.read(entry.file_name().to_str()?).ok())
            .filter(|record| {
                record.state == "finished"
                    && record.request.machine.is_some()
                    && !record.remote.holds_worker()
            })
            .collect();
        if owed.is_empty() {
            return;
        }
        tokio::spawn(async move {
            for record in owed {
                state.settle_or_warn(&record).await;
            }
        });
    }
    fn liveness(&self, wake: &AgentWake) -> Liveness {
        if self.live.lock().expect("job live").contains_key(&wake.id)
            || self
                .state
                .owned
                .durable_state_store
                .leased_wake_binding(&wake.id)
                .is_ok_and(|binding| binding.is_some())
        {
            Liveness::Live
        } else {
            Liveness::Lost("job supervisor ended without a result".into())
        }
    }
}

/// A job whose remote state this kernel will not act on while remote jobs are off.
fn remote_jobs_off_result(record: &Record) -> JobResult {
    JobResult {
        exit_code: None,
        output_tail: "This job ran on a remote machine, but remote jobs are switched off on this kernel (CHARIOX_REMOTE_JOBS), so it was not followed and its machine was left untouched. Start the kernel with remote jobs on to resume it.".into(),
        log: String::new(),
        files_back: vec![],
        conflicts: vec![],
        ran_on: JobRanOn {
            machine: record
                .machine_label
                .clone()
                .unwrap_or_else(|| "a remote machine".into()),
            ip: record.address.clone(),
        },
        duration_ms: crate::session::unix_epoch_ms().saturating_sub(record.started_at_ms),
        outcome: JobOutcome::MachineLost,
    }
}

impl JobDriver {
    pub(super) fn launch(&self, wake: AgentWake, pending: Pending) -> Result<(), DaemonError> {
        let (cancel, cancelled) = watch::channel(false);
        let live = self.live.clone();
        live.lock()
            .expect("job live")
            .insert(wake.id.clone(), cancel);
        if !pending.record.runs_for_lease(&wake) {
            if let Err(error) = self.state.wake_sink(KIND).verified(&wake.id) {
                live.lock().expect("job live").remove(&wake.id);
                return Err(error);
            }
        }
        tokio::spawn(async move {
            pending
                .state
                .execute_job(wake.clone(), pending.record, cancelled, None)
                .await;
            live.lock().expect("job live").remove(&wake.id);
        });
        Ok(())
    }

    /// A state that holds this driver and the job gateway, for work a restart resumes.
    fn recovery_state(&self) -> Option<KernelRuntimeState> {
        let (driver, router) = (
            self.this.get().and_then(std::sync::Weak::upgrade)?,
            self.router.get().and_then(std::sync::Weak::upgrade)?,
        );
        let mut state = self.state.with_job_router((*router).clone());
        state.job_wake_driver = Some(driver);
        Some(state)
    }
}

impl KernelRuntimeState {
    pub(in crate::runtime::state) fn register_job_wake_driver(&mut self) {
        let driver = Arc::new(JobDriver {
            state: self.clone(),
            this: Default::default(),
            router: Default::default(),
            pending: Default::default(),
            live: Default::default(),
            local: Default::default(),
            remote_moves: Default::default(),
            owner_stops: Default::default(),
            cancel_causes: Default::default(),
            remote_samples: Default::default(),
            return_grants: Default::default(),
        });
        let _ = driver.this.set(Arc::downgrade(&driver));
        let source: Arc<dyn WakeDriver> = driver.clone();
        self.register_wake_driver(Arc::downgrade(&source));
        self.job_wake_driver = Some(driver);
    }

    /// Lets a restarted kernel's job recovery reach the job gateway through its router.
    pub(crate) fn bind_job_recovery(&self, router: &Arc<crate::runtime::router::CommandRouter>) {
        if let Some(driver) = &self.job_wake_driver {
            let _ = driver.router.set(Arc::downgrade(router));
            // Claim the inherited children before the transport can start a new reload.
            // The wake scheduler may still be starting when the kernel accepts that reload.
            for id in crate::hot_reload::local_job_wakes() {
                match self.job_wake_for_recovery(&id) {
                    Ok(Some(wake))
                        if wake.kind == KIND
                            && matches!(
                                wake.state.as_str(),
                                "starting" | "running" | "cancelling"
                            ) =>
                    {
                        if let Recovery::Lost(reason) = driver.recover(&wake) {
                            tracing::warn!(%reason, "could not recover the inherited local job");
                        }
                    }
                    Ok(_) => tracing::warn!("the inherited local job has no active wake"),
                    Err(error) => tracing::warn!(%error, "could not load the inherited job's wake"),
                }
            }
            crate::hot_reload::discard_unclaimed_local_jobs();
            self.recover_placed_jobs(driver);
        }
    }

    pub(super) async fn finish_job(
        &self,
        wake: &AgentWake,
        record: &mut Record,
        result: JobResult,
    ) {
        record.state = "finished".into();
        record.result = Some(result.clone());

        if record.runs_for_lease(wake) {
            while let Err(error) = save_record(self, record) {
                tracing::warn!(%error, "placed job result retained for durable save");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            return;
        }

        loop {
            match save_record(self, record).and_then(|()| {
                self.report_job_completion(wake, record, &result, completion_end(&result))
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
    pub(super) fn report_job_completion(
        &self,
        wake: &AgentWake,
        record: &Record,
        result: &JobResult,
        end: WakeEnd,
    ) -> Result<(), DaemonError> {
        self.wake_sink(KIND).occurred(WakeOccurrence {
            wake_id: wake.id.clone(),
            occurrence_id: format!("{}-finished", wake.id),
            kind: "job_finished".into(),
            payload: completion_payload(record, result),
            summary: Some(
                format!("{} · {:?}", wake.label, result.outcome)
                    .chars()
                    .take(120)
                    .collect(),
            ),
            end: Some(end),
        })
    }
}

pub(super) fn completion_end(result: &JobResult) -> WakeEnd {
    if result.outcome == JobOutcome::MachineLost {
        WakeEnd::Lost
    } else if result.exit_code == Some(0) && result.outcome == JobOutcome::Completed {
        WakeEnd::Succeeded
    } else {
        WakeEnd::Failed
    }
}

pub(super) fn completion_payload(record: &Record, result: &JobResult) -> serde_json::Value {
    let payload = serde_json::json!({"job_id":record.job_id,"result":result});
    if serde_json::to_vec(&payload).is_ok_and(|bytes| bytes.len() <= 192 * 1024) {
        return payload;
    }
    let mut bounded = result.clone();
    loop {
        let payload = serde_json::json!({"job_id":record.job_id,"result":bounded,"result_truncated":true,"read_full_result":{"tool":"chariox.jobs.status","job_id":record.job_id},"files_back_count":result.files_back.len(),"conflicts_count":result.conflicts.len()});
        if serde_json::to_vec(&payload).is_ok_and(|bytes| bytes.len() <= 192 * 1024) {
            return payload;
        }
        if bounded.files_back.len() >= bounded.conflicts.len() && !bounded.files_back.is_empty() {
            bounded.files_back.truncate(bounded.files_back.len() / 2);
        } else if !bounded.conflicts.is_empty() {
            bounded.conflicts.truncate(bounded.conflicts.len() / 2);
        } else {
            return serde_json::json!({"job_id":record.job_id,"result_truncated":true,"read_full_result":{"tool":"chariox.jobs.status","job_id":record.job_id}});
        }
    }
}
