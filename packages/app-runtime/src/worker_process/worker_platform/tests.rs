use super::*;
use std::time::Duration;

fn sample(cpu_ns: u64) -> Sample {
    Sample {
        start_time: 42,
        physical_bytes: ResourcePolicy::PHASE1.memory_bytes,
        resident_bytes: 1,
        threads: ResourcePolicy::PHASE1.threads,
        cpu_ns,
    }
}

#[test]
fn mach_cpu_ticks_are_converted_before_applying_the_one_cpu_budget() {
    assert_eq!(
        cpu_nanoseconds(20_000_000, 4_000_000, 125, 3),
        Ok(1_000_000_000)
    );
    assert_eq!(cpu_nanoseconds(600, 400, 1, 1), Ok(1000));
    assert_eq!(
        cpu_nanoseconds(1, 1, 0, 1),
        Err(WorkerError::ResourceTelemetry)
    );
    assert_eq!(
        cpu_nanoseconds(1, 1, 1, 0),
        Err(WorkerError::ResourceTelemetry)
    );
    assert_eq!(
        cpu_nanoseconds(u64::MAX, u64::MAX, 125, 3),
        Err(WorkerError::ResourceTelemetry)
    );
}

#[test]
fn exact_memory_thread_bounds_and_telemetry_identity_are_enforced() {
    let now = Instant::now();
    assert!(Budget::new(ResourcePolicy::PHASE1, sample(0), now).is_ok());
    for (changed, expected) in [
        (
            Sample {
                physical_bytes: sample(0).physical_bytes + 1,
                ..sample(0)
            },
            WorkerError::MemoryLimit,
        ),
        (
            Sample {
                resident_bytes: sample(0).physical_bytes + 1,
                ..sample(0)
            },
            WorkerError::MemoryLimit,
        ),
        (
            Sample {
                threads: 65,
                ..sample(0)
            },
            WorkerError::ThreadLimit,
        ),
        (
            Sample {
                threads: 0,
                ..sample(0)
            },
            WorkerError::ResourceTelemetry,
        ),
        (
            Sample {
                start_time: 0,
                ..sample(0)
            },
            WorkerError::ResourceTelemetry,
        ),
    ] {
        assert!(
            matches!(Budget::new(ResourcePolicy::PHASE1, changed, now), Err(error) if error == expected)
        );
    }
    let mut budget = Budget::new(ResourcePolicy::PHASE1, sample(1), now).unwrap();
    assert_eq!(
        budget.observe(
            Sample {
                start_time: 43,
                ..sample(1)
            },
            now
        ),
        Err(WorkerError::ResourceTelemetry)
    );
    assert_eq!(
        budget.observe(sample(0), now),
        Err(WorkerError::ResourceTelemetry)
    );
    assert_eq!(
        budget.observe(sample(1), now - CHECK_INTERVAL),
        Err(WorkerError::ResourceTelemetry)
    );
}

#[test]
fn one_cpu_and_burst_are_bounded_without_idle_credit_accumulation() {
    let now = Instant::now();
    let mut budget = Budget::new(ResourcePolicy::PHASE1, sample(0), now).unwrap();
    // Sustained one-core work retains its 250ms burst across 100ms samples.
    for index in 1..=20_u64 {
        budget
            .observe(
                sample(index * 100_000_000),
                now + CHECK_INTERVAL * index as u32,
            )
            .unwrap();
    }
    assert_eq!(budget.cpu_credit_ns, CPU_BURST_NS);
    budget
        .observe(sample(2_350_000_000), now + Duration::from_millis(2100))
        .unwrap();
    assert_eq!(budget.cpu_credit_ns, 0);
    assert_eq!(
        budget.observe(sample(2_450_000_001), now + Duration::from_millis(2200)),
        Err(WorkerError::CpuLimit)
    );

    let mut idle = Budget::new(ResourcePolicy::PHASE1, sample(0), now).unwrap();
    idle.observe(sample(0), now + Duration::from_secs(3600))
        .unwrap();
    assert_eq!(idle.cpu_credit_ns, CPU_BURST_NS);
    assert_eq!(
        idle.observe(sample(350_000_001), now + Duration::from_millis(3_600_100)),
        Err(WorkerError::CpuLimit)
    );
}
