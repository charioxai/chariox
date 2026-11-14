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
                .execute_job(wake.clone(), pending.record, cancelled)
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
            // A worker that still holds files not yet returned stays until they are collected.
            if let (Some(allocation), Some(revision), false) = (
                &record.allocation_ref,
                &record.capability_revision,
                record.return_pending,
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
                && !record.return_pending
            {
                match state.job_compute_release_job_ref(&record.job_id).await {
                    Ok(()) => record.deleted = true,
                    Err(error) => {
                        tracing::warn!(%error, "job restart allocation reconciliation pending")
                    }
                }
            }
            let result = record.result.clone().unwrap_or_else(|| JobResult {
                exit_code: None,
                output_tail: if record.return_pending {
                    "The kernel restarted while this job's files were being returned. Its remote machine is kept: check this job with chariox.jobs.status to collect them.".into()
                } else {
                    "The kernel restarted; this job was not relaunched.".into()
                },
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
                outcome: if record.return_pending {
                    JobOutcome::OutsideEnd
                } else {
                    JobOutcome::MachineLost
                },
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
    pub(in crate::runtime::state) fn register_job_wake_driver(&mut self) {
        let driver = Arc::new(JobDriver {
            state: self.clone(),
            pending: Default::default(),
            live: Default::default(),
            remote_requested: Default::default(),
            owner_stops: Default::default(),
            remote_samples: Default::default(),
            remote_origins: Default::default(),
            return_grants: Default::default(),
        });
        let source: Arc<dyn WakeDriver> = driver.clone();
        self.register_wake_driver(Arc::downgrade(&source));
        self.job_wake_driver = Some(driver);
    }

    pub(super) async fn finish_job(
        &self,
        wake: &AgentWake,
        record: &mut Record,
        result: JobResult,
    ) {
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
                    payload: completion_payload(record, &result),
                    summary: Some(
                        format!("{} · {:?}", wake.label, result.outcome)
                            .chars()
                            .take(120)
                            .collect(),
                    ),
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
