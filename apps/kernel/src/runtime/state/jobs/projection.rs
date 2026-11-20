use super::*;

impl KernelRuntimeState {
    pub(crate) fn session_jobs(&self, room: &str) -> Vec<crate::local::JobView> {
        let mut jobs: Vec<_> = summary::read(self, room)
            .into_iter()
            .map(|summary| summary.view)
            .collect();
        jobs.sort_by_key(|job| job.started_at_ms);
        jobs
    }

    pub(crate) fn queued_job_processes(
        &self,
        room: &str,
    ) -> Vec<crate::runtime::process_resources::SessionProcess> {
        self.queued_job_processes_for(room, None)
    }

    pub(super) fn queued_job_processes_for(
        &self,
        room: &str,
        agent: Option<&str>,
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
            if agent.is_some_and(|agent| job.agent_id != agent) {
                continue;
            }
            if job.result.is_some() || job.state == "finished" || job.read_error.is_some() {
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
                    .map(|rate| crate::local::RemoteJobEstimate {
                        amount_micros: record.estimate_micros.unwrap_or(rate.amount_micros),
                        currency: rate.currency,
                        kind: if record.estimate_micros.is_some() {
                            "estimate"
                        } else {
                            "per_minute"
                        }
                        .into(),
                        memory_mib: record.memory_mib.unwrap_or_else(|| {
                            u64::from(record.request.needs.memory_gib.unwrap_or(0)) * 1024
                        }),
                        minutes: record.estimated_minutes,
                    }),
                placement: None,
            });
        }
        rows
    }

    pub(super) fn job_duration_estimate(&self, record: &Record) -> Option<u64> {
        let command_hash = summary::command_hash(&record.request.command);
        let mut durations: Vec<_> = summary::read(self, &record.room_id)
            .into_iter()
            .filter(|previous| previous.matches(record, &command_hash))
            .filter_map(|previous| {
                previous
                    .view
                    .result
                    .filter(|result| {
                        result.outcome == JobOutcome::Completed && result.exit_code == Some(0)
                    })
                    .map(|result| (previous.view.started_at_ms, result.duration_ms))
            })
            .collect();
        if durations.len() < 2 {
            return None;
        }
        durations.sort_unstable_by_key(|(started, _)| *started);
        let previous = durations[durations.len() - 2].1;
        let latest = durations[durations.len() - 1].1;
        if previous.max(latest) > previous.min(latest).saturating_mul(3) / 2 {
            return None;
        }
        Some(previous.saturating_add(latest) / 2)
    }
}

pub(super) fn effective_needs(record: &Record) -> crate::local::JobNeeds {
    let mut needs = record.request.needs.clone();
    if needs.memory_gib.is_none() {
        let command = validate(&record.request).unwrap_or_default();
        needs.memory_gib = crate::runtime::process_resources::LaunchAdmission::default()
            .expected_memory(&command, &record.cwd)
            .map(|bytes| {
                bytes
                    .saturating_mul(5)
                    .div_ceil(4 * 1024 * 1024 * 1024)
                    .min(u64::from(u32::MAX)) as u32
            });
    }
    needs
}
