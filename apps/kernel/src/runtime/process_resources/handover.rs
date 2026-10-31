//! Jobs a workload left running (`setsid nohup job &`) stay their owner's
//! across a hot reload: the next program watches the same processes under the
//! same owner, limit and ids, and they end only with their owner or on a real
//! shutdown.
use super::*;

/// One released workload's remaining processes, as the next program adopts them.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct ReleasedJobs {
    unit: String,
    listing: projection::Listing,
    /// The memory domain they run in, which the next program takes over.
    cgroup: Option<std::path::PathBuf>,
    tree: sampling::Owned,
}

impl ReleasedJobs {
    pub(crate) fn pids(&self) -> Vec<u32> {
        self.tree.alive().collect()
    }
}

/// Every released workload that still has a process, for a kernel that
/// replaces its program in place.
pub(crate) fn hand_over_released() -> Vec<ReleasedJobs> {
    let registry = registry().lock().unwrap_or_else(|e| e.into_inner());
    registry
        .values()
        .filter_map(std::sync::Weak::upgrade)
        .filter_map(|state| {
            let observation = state.lock().unwrap_or_else(|e| e.into_inner());
            if !observation.released {
                return None;
            }
            let jobs = ReleasedJobs {
                unit: observation.unit.clone(),
                listing: observation.listed.clone()?,
                cgroup: observation.cgroup.clone(),
                tree: observation.owned.clone()?,
            };
            (!jobs.pids().is_empty()).then_some(jobs)
        })
        .collect()
}

/// Watches the handed-over jobs again. One that cannot be watched is stopped
/// with its owner's next stop or a restart, like any job the kernel lost track of.
pub(crate) fn adopt_released(handed_over: Vec<ReleasedJobs>) {
    for jobs in handed_over {
        let tree = sampling::ProcessTree::restored(jobs.tree);
        let Some(pid) = tree.alive_pid() else {
            continue;
        };
        let started = PreparedDomain::adopted(&jobs.unit, jobs.cgroup.as_deref(), pid)
            .and_then(|domain| domain.start_released(tree, jobs.listing));
        if let Err(error) = started {
            crate::logging::error_with_fields(
                "process_resources.handover",
                "jobs handed over by the previous program could not be watched again",
                serde_json::json!({"pid": pid, "error": error.to_string()}),
            );
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::BufRead;

    fn wait_until(what: &str, condition: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !condition() {
            assert!(std::time::Instant::now() < deadline, "{what}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    #[test]
    fn a_job_an_ended_agent_workload_left_is_watched_again_by_the_next_program() {
        let session = "handover-room";
        let owner = SessionProcessOwner::Agent("handover-agent".into());
        let mut workload = std::process::Command::new("sh")
            .args(["-c", "sleep 60 & echo $!; sleep 1"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        std::io::BufReader::new(workload.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let job: u32 = line.trim().parse().unwrap();
        let mut domain =
            PreparedDomain::watchdog("handover-unit", None, super::super::tests::roomy())
                .start(workload.id())
                .unwrap();
        domain.bind_key("handover-run");
        let listed_job = || {
            samples_for_key("handover-run")
                .into_iter()
                .find(|row| row.pid == job)
        };
        wait_until("the watch never saw the job", || listed_job().is_some());
        let id = listed_job().unwrap().process_id;
        domain.release(session, owner.clone(), None);
        workload.wait().unwrap();

        // The previous program's end: what it hands over crosses as JSON, and
        // its own watch is gone with it.
        let handed = serde_json::to_vec(&hand_over_released())
            .and_then(|body| serde_json::from_slice::<Vec<ReleasedJobs>>(&body))
            .unwrap()
            .into_iter()
            .filter(|jobs| jobs.listing.session_id == session)
            .collect::<Vec<_>>();
        assert_eq!(handed.len(), 1, "the released job is handed over");
        assert_eq!(handed[0].pids(), vec![job]);
        registry()
            .lock()
            .unwrap()
            .retain(|_, state| !state.ptr_eq(&Arc::downgrade(&domain.state)));
        assert!(
            released_keys_for_session(session).is_empty(),
            "the previous watch is out of the registry"
        );

        adopt_released(handed);
        let keys = released_keys_for_session(session);
        assert_eq!(keys.len(), 1, "the job is listed again");
        assert_eq!(keys[0].0, owner);
        let key = keys[0].1.clone();
        wait_until("the adopted watch never sampled the job", || {
            samples_for_key(&key).iter().any(|row| row.pid == job)
        });
        assert_eq!(
            samples_for_key(&key)
                .into_iter()
                .find(|row| row.pid == job)
                .map(|row| row.process_id),
            Some(id),
            "Health lists the job under the id it had"
        );

        stop_released(Some(session), Some(&owner));
        wait_until("the owner's stop did not end the job", || {
            !std::path::Path::new(&format!("/proc/{job}")).exists()
                || std::fs::read_to_string(format!("/proc/{job}/stat"))
                    .is_ok_and(|stat| stat.contains(") Z "))
        });
    }
}
