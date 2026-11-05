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
    /// Who launched each job: a person's keeps not waiting for room.
    submissions: Submissions,
}

impl ReleasedJobs {
    /// What runs in the memory domain right now. The last sample cannot list
    /// a child its parent forked just before ending, so the domain is the
    /// authority; a host without cgroups has only the sampled processes.
    fn domain_members(&self) -> Vec<u32> {
        #[cfg(target_os = "linux")]
        return self
            .cgroup
            .as_deref()
            .map(super::cgroup::members)
            .unwrap_or_default()
            .into_iter()
            .filter(|pid| *pid > 1)
            .collect();
        #[cfg(not(target_os = "linux"))]
        Vec::new()
    }

    pub(crate) fn pids(&self) -> Vec<u32> {
        let mut pids: Vec<u32> = self.tree.alive().collect();
        for pid in self.domain_members() {
            if !pids.contains(&pid) {
                pids.push(pid);
            }
        }
        pids
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
            let mut jobs = ReleasedJobs {
                unit: observation.unit.clone(),
                listing: observation.listed.clone()?,
                cgroup: observation.cgroup.clone(),
                tree: observation.owned.clone()?,
                submissions: observation.submissions.clone(),
            };
            jobs.tree.including(jobs.domain_members());
            (!jobs.pids().is_empty()).then_some(jobs)
        })
        .collect()
}

/// Watches the handed-over jobs again. One that cannot be watched is stopped
/// with its owner's next stop or a restart, like any job the kernel lost track of.
pub(crate) fn adopt_released(handed_over: Vec<ReleasedJobs>) {
    for mut jobs in handed_over {
        // Whatever ended or was forked during the swap, the domain knows.
        jobs.tree.including(jobs.domain_members());
        let ReleasedJobs {
            unit,
            listing,
            cgroup,
            tree,
            submissions,
        } = jobs;
        let tree = sampling::ProcessTree::restored(tree);
        if tree.alive_pid().is_none() {
            continue;
        }
        let root = tree.root();
        let started = PreparedDomain::adopted(&unit, cgroup.as_deref())
            .map(|domain| domain.launched_by(submissions))
            .and_then(|domain| domain.start_released(tree, listing));
        if let Err(error) = started {
            crate::logging::error_with_fields(
                "process_resources.handover",
                "jobs handed over by the previous program could not be watched again",
                serde_json::json!({"pid": root, "unit": unit, "error": error.to_string()}),
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

    /// The next program finds the job in the memory domain even though the
    /// recorded processes (the parent that forked it and ended) are gone.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires a writable cgroup-v2 delegated parent"]
    fn a_job_whose_recorded_processes_ended_is_adopted_from_its_memory_domain() {
        use std::io::Write;
        let session = "handover-domain-room";
        let owner = SessionProcessOwner::Agent("handover-domain-agent".into());
        let prepared =
            PreparedDomain::in_pool("handover-domain-unit", None, super::super::tests::roomy());
        assert!(prepared.cgroup.is_some(), "cgroup delegation required");
        let (program, args) = prepared.command(
            "/bin/sh".into(),
            vec![
                "-c".into(),
                "read go; sleep 60 </dev/null >/dev/null 2>&1 & echo $!".into(),
            ],
        );
        let mut workload = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut domain = prepared.start(workload.id()).unwrap();
        domain.bind_key("handover-domain-run");
        wait_until("the watch never sampled the workload", || {
            !samples_for_key("handover-domain-run").is_empty()
        });
        domain.release(session, owner.clone(), None);
        // What the last sample knew: the workload alone.
        let (cgroup, stale) = {
            let state = domain.state.lock().unwrap();
            (state.cgroup.clone(), state.owned.clone().unwrap())
        };
        workload.stdin.take().unwrap().write_all(b"go\n").unwrap();
        let mut line = String::new();
        std::io::BufReader::new(workload.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let job: u32 = line.trim().parse().unwrap();
        workload.wait().unwrap();
        let handed = ReleasedJobs {
            unit: "handover-domain-unit".into(),
            listing: projection::Listing {
                session_id: session.into(),
                owner: owner.clone(),
                opened_by: None,
            },
            cgroup,
            tree: stale,
            submissions: Submissions::new(true),
        };
        assert!(
            handed.pids().contains(&job),
            "the domain still holds the job"
        );
        registry()
            .lock()
            .unwrap()
            .retain(|_, state| !state.ptr_eq(&Arc::downgrade(&domain.state)));

        adopt_released(vec![handed]);
        let keys = released_keys_for_session(session);
        assert_eq!(keys.len(), 1, "the job is listed again");
        assert_eq!(keys[0].0, owner);
        let key = keys[0].1.clone();
        wait_until("the adopted watch never sampled the job", || {
            samples_for_key(&key).iter().any(|row| row.pid == job)
        });

        stop_released(Some(session), Some(&owner));
        wait_until("the owner's stop did not end the job", || {
            !std::path::Path::new(&format!("/proc/{job}")).exists()
                || std::fs::read_to_string(format!("/proc/{job}/stat"))
                    .is_ok_and(|stat| stat.contains(") Z "))
        });
    }
}
