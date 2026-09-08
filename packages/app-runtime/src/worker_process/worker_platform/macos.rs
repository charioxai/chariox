use super::{Budget, ResourcePolicy, Sample, CHECK_INTERVAL};
use crate::worker_process::WorkerError;
use std::{
    mem::{size_of, MaybeUninit},
    time::Instant,
};

/// Accounting only. The caller must own the direct, unreaped child and retain
/// its sandbox, immutable roots and aggregate reservations until actual reap.
/// A PID or successful sample is never a preparation/containment credential.
pub(in crate::worker_process) struct MacResourceMonitor {
    pid: libc::pid_t,
    budget: Budget,
    timebase: libc::mach_timebase_info_data_t,
}

impl MacResourceMonitor {
    pub(in crate::worker_process) fn attach(
        pid: libc::pid_t,
        now: Instant,
    ) -> Result<Self, WorkerError> {
        let mut timebase = MaybeUninit::<libc::mach_timebase_info_data_t>::zeroed();
        if unsafe { libc::mach_timebase_info(timebase.as_mut_ptr()) } != 0 {
            return Err(WorkerError::ResourceTelemetry);
        }
        let timebase = unsafe { timebase.assume_init() };
        if timebase.numer == 0 || timebase.denom == 0 {
            return Err(WorkerError::ResourceTelemetry);
        }
        Ok(Self {
            pid,
            budget: Budget::new(ResourcePolicy::PHASE1, sample_owned(pid, &timebase)?, now)?,
            timebase,
        })
    }

    pub(in crate::worker_process) fn check_running(
        &mut self,
        pid: libc::pid_t,
        now: Instant,
    ) -> Result<(), WorkerError> {
        if pid != self.pid {
            return Err(WorkerError::ResourceTelemetry);
        }
        let elapsed = now
            .checked_duration_since(self.budget.sampled)
            .ok_or(WorkerError::ResourceTelemetry)?;
        if elapsed < CHECK_INTERVAL {
            return Ok(());
        }
        self.budget.observe(sample_owned(pid, &self.timebase)?, now)
    }

    #[cfg(test)]
    pub(in crate::worker_process) fn lower_memory_for_growth_fixture(&mut self) {
        assert!(self.budget.last.memory_bytes() < 64 * 1024 * 1024);
        self.budget.policy.memory_bytes = self.budget.last.memory_bytes() + 2 * 1024 * 1024;
    }
}

fn sample_owned(
    pid: libc::pid_t,
    timebase: &libc::mach_timebase_info_data_t,
) -> Result<Sample, WorkerError> {
    if pid <= 1 {
        return Err(WorkerError::ResourceTelemetry);
    }
    let mut identity = MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let mut task = MaybeUninit::<libc::proc_taskinfo>::zeroed();
    let mut usage = MaybeUninit::<libc::rusage_info_v4>::zeroed();
    // Fixed-size libproc calls only: no task_for_pid/debug entitlement, subprocess,
    // process enumeration or attacker-sized allocation. Available on our 13.5 floor.
    // Apple xnu bsd/sys/resource.h defines the footprint/start/cpu fields;
    // osfmk/kern/bsd_kern.c fill_task_rusage returns task_power_info's Mach
    // ticks. Convert them using the machine timebase, particularly on ARM64.
    let valid = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            identity.as_mut_ptr().cast(),
            size_of::<libc::proc_bsdinfo>() as i32,
        ) == size_of::<libc::proc_bsdinfo>() as i32
            && libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTASKINFO,
                0,
                task.as_mut_ptr().cast(),
                size_of::<libc::proc_taskinfo>() as i32,
            ) == size_of::<libc::proc_taskinfo>() as i32
            && libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V4, usage.as_mut_ptr().cast()) == 0
    };
    if !valid {
        return Err(WorkerError::ResourceTelemetry);
    }
    let (identity, task, usage) = unsafe {
        (
            identity.assume_init(),
            task.assume_init(),
            usage.assume_init(),
        )
    };
    if identity.pbi_pid != pid as u32
        || identity.pbi_ppid != unsafe { libc::getpid() } as u32
        || identity.pbi_uid != unsafe { libc::geteuid() }
        || task.pti_threadnum <= 0
        || usage.ri_proc_exit_abstime != 0
    {
        return Err(WorkerError::ResourceTelemetry);
    }
    Ok(Sample {
        start_time: usage.ri_proc_start_abstime,
        physical_bytes: usage.ri_phys_footprint,
        resident_bytes: usage.ri_resident_size.max(task.pti_resident_size),
        threads: task.pti_threadnum as u32,
        cpu_ns: super::cpu_nanoseconds(
            usage.ri_user_time,
            usage.ri_system_time,
            timebase.numer,
            timebase.denom,
        )?,
    })
}
