//! The shared ceiling (PC-06, PC-07): nobody is stopped while the pool has
//! room; under pressure exactly one command yields, never its owner's root.
use super::*;

const ALLOC: &str = "import sys,time
start,rate,top=map(int,sys.argv[1:4])
b=[bytearray(start*1048576)] if start else []
total=start
while rate and total<top:
    b.append(bytearray(8*1048576)); total+=8; time.sleep(8/rate)
time.sleep(60)
";

pub(super) struct Workload {
    child: std::process::Child,
    pub(super) domain: ProcessDomain,
    pub(super) dir: std::path::PathBuf,
}
impl Workload {
    /// A shell root running named commands; each is `name start_mib rate_mib_s top_mib`.
    pub(super) fn start(prepared: PreparedDomain, commands: &[(&str, u64, u64, u64)]) -> Self {
        let commands = commands
            .iter()
            .map(|(name, start, rate, top)| {
                (
                    *name,
                    format!("python3 \"$0/alloc.py\" {start} {rate} {top}"),
                )
            })
            .collect::<Vec<_>>();
        Self::run(prepared, &commands)
    }
    /// A shell root running named shell commands in the background; `$0` is
    /// the workload's directory, holding `alloc.py`.
    pub(super) fn run(prepared: PreparedDomain, commands: &[(&str, String)]) -> Self {
        Self::run_under(prepared, false, commands)
    }
    /// The same under a launcher, as a provider's wrapper runs its server:
    /// a non-shell root whose shell starts the commands.
    pub(super) fn launched(prepared: PreparedDomain, commands: &[(&str, String)]) -> Self {
        Self::run_under(prepared, true, commands)
    }
    fn run_under(prepared: PreparedDomain, launcher: bool, commands: &[(&str, String)]) -> Self {
        let dir = std::env::temp_dir().join(format!("pc-{:016x}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("alloc.py"), ALLOC).unwrap();
        let mut script = String::new();
        for (name, command) in commands {
            script.push_str(&format!(
                "{command} 2>\"$0/{name}.err\" & echo $! > \"$0/{name}.pid\"; "
            ));
        }
        script.push_str("wait");
        let mut argv = vec!["-c".into(), script, dir.to_string_lossy().into_owned()];
        if launcher {
            // A launcher whose server starts the shell running the commands.
            let server = "import subprocess,sys; sys.exit(subprocess.call(sys.argv[1:]))";
            argv.splice(
                0..0,
                [
                    "-c".to_string(),
                    format!("import subprocess,sys; sys.exit(subprocess.call([sys.executable, '-c', '{server}'] + sys.argv[1:]))"),
                    "/bin/sh".into(),
                ],
            );
        }
        let (program, args) =
            prepared.command(if launcher { "python3" } else { "/bin/sh" }.into(), argv);
        let child = std::process::Command::new(program)
            .args(args)
            .spawn()
            .unwrap();
        let domain = prepared.start(child.id()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while commands
            .iter()
            .any(|(name, ..)| !dir.join(format!("{name}.pid")).exists())
        {
            assert!(
                std::time::Instant::now() < deadline,
                "commands did not start"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        Self { child, domain, dir }
    }
    pub(super) fn pid(&self, name: &str) -> i32 {
        std::fs::read_to_string(self.dir.join(format!("{name}.pid")))
            .unwrap()
            .trim()
            .parse::<i32>()
            .unwrap()
    }
    pub(super) fn alive(&mut self, name: &str) -> bool {
        let pid = self.pid(name);
        // The test's own children: reap one that ended so it reads as gone.
        let mut status = 0;
        unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        crate::test_support::process_alive(pid)
    }
    /// Admission stopped the command (`SIGSTOP`): it waits.
    #[cfg(target_os = "linux")]
    pub(super) fn waiting(&self, name: &str) -> bool {
        std::fs::read_to_string(format!("/proc/{}/stat", self.pid(name)))
            .ok()
            .and_then(|stat| {
                stat.rsplit_once(')')
                    .and_then(|(_, rest)| rest.split_whitespace().next().map(str::to_string))
            })
            .is_some_and(|state| state == "T")
    }
    pub(super) fn root_alive(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }
    /// The stopped command was told why on its own error stream. Only Linux
    /// can reopen another process's stream; elsewhere the stop is silent.
    fn assert_told(&self, name: &str, reason: &str) {
        #[cfg(target_os = "linux")]
        {
            let notice =
                std::fs::read_to_string(self.dir.join(format!("{name}.err"))).unwrap_or_default();
            assert!(
                notice.contains(reason),
                "{name} was not told why: {notice:?}"
            );
        }
        #[cfg(not(target_os = "linux"))]
        let _ = (name, reason);
    }
}
impl Drop for Workload {
    // The domain, dropped after this, ends what the root left running.
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub(super) fn pool(ceiling_mib: u64) -> Arc<pool::Pool> {
    Arc::new(pool::Pool::new(ceiling_mib * MIB, false))
}

pub(super) fn domain(pool: &Arc<pool::Pool>, unit: &str, cgroup: bool) -> PreparedDomain {
    if cgroup {
        let prepared = PreparedDomain::in_pool(unit, None, pool.clone());
        #[cfg(target_os = "linux")]
        assert!(prepared.cgroup.is_some(), "cgroup delegation required");
        prepared
    } else {
        PreparedDomain::watchdog(unit, None, pool.clone())
    }
}

pub(super) fn settle(seconds: u64) {
    std::thread::sleep(Duration::from_secs(seconds));
}

fn lone_owner(cgroup: bool) {
    let pool = pool(768);
    // 640 of 768 MiB: most of the ceiling, and far above any per-agent default.
    let mut lone = Workload::start(domain(&pool, "lone", cgroup), &[("big", 640, 0, 0)]);
    settle(3);
    assert!(
        lone.alive("big"),
        "a lone owner was stopped while the pool had room"
    );
    assert!(lone.root_alive());
    assert_eq!(lone.domain.memory_limit_bytes(), None);
}

#[cfg(unix)]
#[test]
fn a_lone_owner_uses_most_of_the_ceiling_untouched() {
    lone_owner(false);
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires a writable cgroup-v2 delegated parent"]
fn a_lone_owner_uses_most_of_the_ceiling_untouched_in_its_cgroup() {
    lone_owner(true);
}

fn six_owners(cgroup: bool) {
    let pool = pool(768);
    let mut first = Workload::start(domain(&pool, "first", cgroup), &[("big", 300, 0, 0)]);
    settle(1);
    // Five more owners: the share falls to 128 MiB, far below the first's 300.
    let mut others = (0..5)
        .map(|n| {
            Workload::start(
                domain(&pool, &format!("other-{n}"), cgroup),
                &[("small", 20, 0, 0)],
            )
        })
        .collect::<Vec<_>>();
    settle(3);
    assert!(
        first.alive("big"),
        "launching agents interrupted a running one"
    );
    assert!(others.iter_mut().all(|other| other.alive("small")));
}

#[cfg(unix)]
#[test]
fn starting_owners_never_interrupts_one_above_its_new_share() {
    six_owners(false);
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires a writable cgroup-v2 delegated parent"]
fn starting_owners_never_interrupts_one_above_its_new_share_in_cgroups() {
    six_owners(true);
}

/// A sits furthest above its share; B grows from `start` MiB at `rate` MiB/s
/// to 1000 MiB; C is small. B passes the pressure line near 540 MiB, while A
/// (800 MiB) is still the larger; with A's big command gone the pool has room.
fn pressure(
    cgroup: bool,
    start: u64,
    rate: u64,
    seconds: u64,
) -> (Workload, Workload, Workload, Arc<pool::Pool>) {
    let pool = pool(1536);
    let a = Workload::start(
        domain(&pool, "a", cgroup),
        &[("big", 800, 0, 0), ("idle", 0, 0, 0)],
    );
    let c = Workload::start(domain(&pool, "c", cgroup), &[("small", 20, 0, 0)]);
    settle(1);
    let b = Workload::start(
        domain(&pool, "b", cgroup),
        &[("idle", 0, 0, 0), ("grow", start, rate, 1000)],
    );
    settle(seconds);
    (a, b, c, pool)
}

fn slow_growth_is_paused_before_anything_is_stopped(cgroup: bool) {
    let (mut a, mut b, mut c, _pool) = pressure(cgroup, 300, 48, 9);
    assert!(
        a.alive("big") && a.alive("idle") && a.root_alive(),
        "a command was stopped where the growing one could wait"
    );
    assert!(b.root_alive() && b.alive("grow") && b.alive("idle"));
    #[cfg(target_os = "linux")]
    assert!(
        b.waiting("grow"),
        "the newest growing command was not paused"
    );
    assert!(c.alive("small"));
    assert_eq!(a.domain.memory_limit_bytes(), None);
    // Room frees: the paused command continues by itself.
    unsafe { libc::kill(a.pid("big"), libc::SIGKILL) };
    settle(3);
    #[cfg(target_os = "linux")]
    assert!(!b.waiting("grow"), "the paused command did not continue");
    assert!(b.alive("grow"));
}

#[cfg(unix)]
#[test]
fn pressure_pauses_the_newest_growing_command_before_stopping_any() {
    slow_growth_is_paused_before_anything_is_stopped(false);
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires a writable cgroup-v2 delegated parent"]
fn pressure_pauses_the_newest_growing_command_before_stopping_any_in_cgroups() {
    slow_growth_is_paused_before_anything_is_stopped(true);
}

fn runaway_is_paused_before_the_line(cgroup: bool) {
    // 512 MiB/s: the PC-06 runaway.
    let (mut a, mut b, mut c, pool) = pressure(cgroup, 0, 512, 4);
    assert!(
        b.alive("grow"),
        "the runaway was stopped where it could wait"
    );
    #[cfg(target_os = "linux")]
    assert!(b.waiting("grow"), "the fast runaway kept running");
    assert!(
        b.root_alive() && b.alive("idle"),
        "the runaway's owner lost more than the command"
    );
    assert!(a.alive("big") && a.alive("idle") && a.root_alive());
    assert!(c.alive("small"));
    #[cfg(target_os = "linux")]
    if cgroup {
        let budget = pool.budget().unwrap();
        let events = std::fs::read_to_string(budget.path().join("memory.events")).unwrap();
        assert!(
            events.lines().any(|line| line == "oom_kill 0"),
            "the OOM killer acted before the controller: {events}"
        );
    }
    let _ = pool;
}

#[cfg(unix)]
#[test]
fn a_fast_runaway_is_paused_and_nothing_is_stopped() {
    runaway_is_paused_before_the_line(false);
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires a writable cgroup-v2 delegated parent"]
fn a_fast_runaway_is_paused_before_the_oom_killer() {
    runaway_is_paused_before_the_line(true);
}

fn owner_cap(cgroup: bool) {
    let pool = super::tests::roomy();
    let prepared = if cgroup {
        PreparedDomain::in_pool("capped", Some(128 * MIB), pool)
    } else {
        PreparedDomain::watchdog("capped", Some(128 * MIB), pool)
    };
    let mut capped = Workload::start(prepared, &[("idle", 0, 0, 0), ("grow", 0, 64, 400)]);
    settle(6);
    assert!(
        !capped.alive("grow"),
        "the command over the owner's cap kept running"
    );
    capped.assert_told(
        "grow",
        "stopped this command at its owner-set memory cap of 128 MiB",
    );
    assert!(
        capped.root_alive() && capped.alive("idle"),
        "the cap stopped more than one command"
    );
    assert_eq!(capped.domain.memory_limit_bytes(), None);
}

#[cfg(unix)]
#[test]
fn an_owner_set_cap_stops_one_command_of_that_owner() {
    owner_cap(false);
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires a writable cgroup-v2 delegated parent"]
fn an_owner_set_cap_stops_one_command_of_that_owner_in_its_cgroup() {
    owner_cap(true);
}
