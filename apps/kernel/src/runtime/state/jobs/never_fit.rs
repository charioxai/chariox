//! A job that asks for more than this machine can ever give is refused once no other machine
//! takes it; a job that fits and only has to wait keeps waiting.
use crate::local::JobNeeds;

pub(super) struct Shortfall {
    memory: Option<(u32, u64)>,
    cpus: Option<(u32, u32)>,
}

impl Shortfall {
    /// `memory_capacity` is in bytes, `cpus` the cores this kernel may use.
    pub(super) fn of(needs: &JobNeeds, memory_capacity: u64, cpus: u32) -> Option<Self> {
        let memory = needs
            .memory_gib
            .filter(|gib| u64::from(*gib) * (1 << 30) > memory_capacity)
            .map(|gib| (gib, memory_capacity));
        let cpus = needs
            .cpus
            .filter(|needed| *needed > cpus)
            .map(|needed| (needed, cpus));
        (memory.is_some() || cpus.is_some()).then_some(Self { memory, cpus })
    }

    /// `answer` is why no other machine took the job.
    pub(super) fn refusal(&self, answer: &str) -> String {
        let (mut needs, mut has) = (vec![], vec![]);
        if let Some((gib, capacity)) = self.memory {
            needs.push(format!("{gib} GB of memory"));
            has.push(crate::runtime::process_resources::size(capacity));
        }
        if let Some((needed, available)) = self.cpus {
            needs.push(format!("{needed} CPUs"));
            has.push(format!("{available} CPUs"));
        }
        format!(
            "chariox: refused this job: it needs {} and this machine has {}, so it can never run here, and {answer}. To run it, add a machine with {} or lower the job's needs.\n",
            needs.join(" and "),
            has.join(" and "),
            needs.join(" and "),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1 << 30;

    fn needs(memory_gib: Option<u32>, cpus: Option<u32>) -> JobNeeds {
        JobNeeds { memory_gib, cpus }
    }

    #[test]
    fn a_job_within_the_machine_has_no_shortfall() {
        assert!(Shortfall::of(&needs(Some(4), Some(2)), 4 * GIB, 2).is_none());
        assert!(Shortfall::of(&needs(None, None), 0, 0).is_none());
    }

    #[test]
    fn memory_or_cpus_above_the_machine_name_what_is_needed_and_what_there_is() {
        let message = Shortfall::of(&needs(Some(64), Some(16)), 2 * GIB, 4)
            .unwrap()
            .refusal("no other machine answered");
        assert_eq!(
            message,
            "chariox: refused this job: it needs 64 GB of memory and 16 CPUs and this machine has 2 GB and 4 CPUs, so it can never run here, and no other machine answered. To run it, add a machine with 64 GB of memory and 16 CPUs or lower the job's needs.\n"
        );
    }
}
