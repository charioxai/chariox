//! The shared ceiling (PC-06, PC-07): nobody is stopped while the pool has
//! room; under pressure exactly one command yields, never its owner's root.
use super::*;

/// On Linux a fixture dies with its parent, stopped or not: a test that is
/// killed must not leave commands behind, and a stopped orphan never exits.
/// Other platforms rely on the workload's tree cleanup.
pub(super) const DIE_WITH_PARENT: &str =
    r#"import ctypes,sys;sys.platform == "linux" and ctypes.CDLL(None).prctl(1,9);"#;

/// A disk writer (4 MiB every half second): it writes at most 256 MiB in its
/// life and ends with its parent or its scratch directory.
pub(super) fn writer_command() -> String {
    format!(
        "python3 -c '{DIE_WITH_PARENT}import os,sys,time\nf=open(sys.argv[1],\"wb\")\nfor _ in range(64):\n if not os.path.isdir(os.path.dirname(sys.argv[1])): break\n f.write(os.urandom(4194304)); f.flush(); os.fsync(f.fileno()); time.sleep(0.5)' \"$0/out\""
    )
}

const ALLOC: &str = "import ctypes,sys,time
if sys.platform == \"linux\": ctypes.CDLL(None).prctl(1,9)
start,rate,top=map(int,sys.argv[1:4])
b=[bytearray(start*1048576)] if start else []
total=start
while rate and total<top:
    b.append(bytearray(8*1048576)); total+=8; time.sleep(8/rate)
time.sleep(60)
";

/// Exercise both workloads with the platform and libc surface macOS provides.
#[test]
fn allocation_and_parent_death_fixtures_run_without_linux_prctl() {
    let check = r#"import ctypes,sys,time
source,prefix,platform=sys.argv[1:]
sys.platform=platform
calls=[]
class Libc:
    def prctl(self,*args): calls.append(args)
ctypes.CDLL=lambda _: Libc() if platform == "linux" else object()
time.sleep=lambda _: None
sys.argv=["alloc.py","1","8","9"]
scope={}
exec(source,scope)
assert scope["total"] == 9
assert sum(map(len,scope["b"])) == 9*1048576
exec(prefix+"hook_finished=True",scope)
assert scope["hook_finished"]
assert calls == ([(1,9),(1,9)] if platform == "linux" else [])
"#;
    for platform in ["darwin", "linux"] {
        let output = std::process::Command::new("python3")
            .args(["-c", check, ALLOC, DIE_WITH_PARENT, platform])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{platform}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

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
        Self::run_under(prepared, false, false, commands)
    }
    /// The same under a launcher, as a provider's wrapper runs its server:
    /// a non-shell root whose shell starts the commands.
    pub(super) fn launched(prepared: PreparedDomain, commands: &[(&str, String)]) -> Self {
        Self::run_under(prepared, true, false, commands)
    }
    /// The commands wait for `go()`: the test sets the domain's policy first,
    /// as a terminal does before its shell runs a line.
    pub(super) fn gated(prepared: PreparedDomain, commands: &[(&str, String)]) -> Self {
        Self::run_under(prepared, false, true, commands)
    }
    /// Another program's watch of the same workload: a hot reload starts one
    /// while the first program's ends with its `exec`.
    pub(super) fn watched_again(&self, prepared: PreparedDomain) -> ProcessDomain {
        prepared.start(self.child.id()).unwrap()
    }
    pub(super) fn go(&self, commands: &[&str]) {
        std::fs::write(self.dir.join("go"), "").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while commands
            .iter()
            .any(|name| !self.dir.join(format!("{name}.pid")).exists())
        {
            assert!(
                std::time::Instant::now() < deadline,
                "commands did not start"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn run_under(
        prepared: PreparedDomain,
        launcher: bool,
        gated: bool,
        commands: &[(&str, String)],
    ) -> Self {
        let dir = std::env::temp_dir().join(format!("pc-{:016x}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("alloc.py"), ALLOC).unwrap();
        let mut script = String::new();
        if gated {
            script.push_str("while [ ! -e \"$0/go\" ]; do sleep 0.05; done; ");
        }
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
        let mut command = std::process::Command::new(program);
        command.args(args);
        // SAFETY: only `prctl` runs between fork and exec. The root dies with
        // the test that started it; its commands die with it.
        #[cfg(target_os = "linux")]
        unsafe {
            use std::os::unix::process::CommandExt;
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                Ok(())
            });
        }
        let child = command.spawn().unwrap();
        let domain = prepared.start(child.id()).unwrap();
        if gated {
            return Self { child, domain, dir };
        }
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
        self.exit_status().is_none()
    }
    pub(super) fn exit_status(&mut self) -> Option<std::process::ExitStatus> {
        self.child.try_wait().unwrap()
    }
    /// What the command was told on its own error stream.
    #[cfg(target_os = "linux")]
    pub(super) fn told(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.join(format!("{name}.err"))).unwrap_or_default()
    }
    /// The stopped command was told why on its own error stream. Only Linux
    /// can reopen another process's stream; elsewhere the stop is silent.
    pub(super) fn assert_told(&self, name: &str, reason: &str) {
        #[cfg(target_os = "linux")]
        {
            let notice = self.told(name);
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
    // The domain, dropped after this, ends what the root left running, but it
    // may not have sampled a command yet, and one admission stopped outlives
    // the root: it reparents to PID 1 and, for a writer, fills the disk. So
    // the whole tree goes with the root, `SIGKILL` being the only signal a
    // stopped process obeys.
    fn drop(&mut self) {
        let tree = super::session_sweep::descendants(self.child.id() as i32);
        let _ = self.child.kill();
        let _ = self.child.wait();
        for pid in &tree {
            // SAFETY: a positive PID read from the process table a moment ago.
            unsafe { libc::kill(*pid, libc::SIGKILL) };
        }
        // Nothing may outlive the scratch directory its command line names.
        for pid in processes_naming(&self.dir) {
            // SAFETY: a positive PID of a process whose command line names this workload's directory.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut stray = processes_naming(&self.dir);
        while !stray.is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            stray = processes_naming(&self.dir);
        }
        let _ = std::fs::remove_dir_all(&self.dir);
        if !std::thread::panicking() {
            assert!(
                stray.is_empty(),
                "commands outlived the workload: {stray:?}"
            );
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while tree
                .iter()
                .any(|pid| crate::test_support::process_alive(*pid))
            {
                assert!(
                    std::time::Instant::now() < deadline,
                    "a workload's commands outlived it: {tree:?}"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

/// The processes whose command line names `dir` (the workload's scratch directory).
#[cfg(target_os = "linux")]
fn processes_naming(dir: &std::path::Path) -> Vec<i32> {
    let name = dir.to_string_lossy().into_owned();
    let me = std::process::id() as i32;
    std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<i32>().ok())
        .filter(|pid| *pid != me)
        .filter(|pid| {
            std::fs::read(format!("/proc/{pid}/cmdline"))
                .is_ok_and(|bytes| String::from_utf8_lossy(&bytes).contains(&name))
        })
        .collect()
}
#[cfg(not(target_os = "linux"))]
fn processes_naming(_dir: &std::path::Path) -> Vec<i32> {
    Vec::new()
}

/// Commands admission stopped, under a root that waits on them: what a
/// disk-limit cell leaves behind when its workload is dropped.
#[cfg(unix)]
#[test]
fn dropping_a_workload_ends_the_commands_admission_stopped() {
    let pool = pool(768);
    let workload = Workload::run(
        domain(&pool, "stopped", false),
        &[("job", "sh -c 'sleep 60 & wait'".into())],
    );
    let tree = super::session_sweep::descendants(workload.child.id() as i32);
    assert!(tree.len() >= 2, "the command did not start: {tree:?}");
    for pid in &tree {
        // SAFETY: a positive PID of this test's own process tree.
        unsafe { libc::kill(*pid, libc::SIGSTOP) };
    }
    drop(workload);
    std::thread::sleep(Duration::from_millis(500));
    let left = tree
        .iter()
        .filter(|pid| crate::test_support::process_alive(**pid))
        .collect::<Vec<_>>();
    assert!(
        left.is_empty(),
        "stopped commands outlived the workload: {left:?}"
    );
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
    pressure_in(pool(1536), cgroup, start, rate, seconds)
}

fn pressure_in(
    pool: Arc<pool::Pool>,
    cgroup: bool,
    start: u64,
    rate: u64,
    seconds: u64,
) -> (Workload, Workload, Workload, Arc<pool::Pool>) {
    let a = Workload::start(
        domain(&pool, "a", cgroup),
        &[("big", 800, 0, 0), ("idle", 0, 0, 0)],
    );
    let c = Workload::start(domain(&pool, "c", cgroup), &[("small", 20, 0, 0)]);
    // The holder's startup allocation must not contribute pressure attributed to the grower.
    let big_pid = u32::try_from(a.pid("big")).expect("the holder PID should be positive");
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let ready = a
            .domain
            .state
            .lock()
            .unwrap()
            .commands
            .iter()
            .any(|command| {
                command.pid == big_pid
                    && command.resident >= 800 * MIB
                    && command.measured_growth == 0
            });
        if ready {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the holder never finished its startup allocation"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
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

/// The running command neither grows nor finishes, and the paused one holds
/// its memory: nothing would ever free. After the stall window the paused
/// command ends with its reason, and the running one is left alone.
#[cfg(target_os = "linux")]
#[test]
fn a_pause_that_frees_nothing_ends_the_paused_command_after_the_stall_window() {
    let pool = pool(1536);
    pool.ledger.lock().unwrap().stall_window = Duration::from_secs(4);
    let a = Workload::start(
        domain(&pool, "a", false),
        &[("big", 800, 0, 0), ("idle", 0, 0, 0)],
    );
    settle(1);
    let mut b = Workload::start(
        domain(&pool, "b", false),
        &[("idle", 0, 0, 0), ("grow", 300, 48, 1000)],
    );
    // The window runs from the pause: wait for it, then check inside it.
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while !b.waiting("grow") && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(b.waiting("grow"), "the growing command was never paused");
    settle(2);
    assert!(b.alive("grow"), "the stall window passed too soon");
    settle(10);
    let mut a = a;
    assert!(
        a.alive("big") && a.root_alive(),
        "the running command ended"
    );
    assert!(
        !b.alive("grow"),
        "the paused command held its memory for ever under a full pool"
    );
}

/// The program of an agent's tool call starts a grower and ends: the grower is
/// then a command of its own, seen for the first time. It must read as the
/// grower it is, or a pass that finds the pool full cannot pause it and ends
/// the idle command beside it instead.
#[cfg(target_os = "linux")]
#[test]
fn a_command_a_shell_leaves_behind_keeps_the_growth_it_had_inside_it() {
    let pool = pool(4096);
    let wrapper = "sh -c 'sed 2d \"$1/alloc.py\" > \"$1/grow.py\"; python3 \"$1/grow.py\" 300 100 3000 & echo $! > \"$1/grow.pid\"; exec sleep 2' sh \"$0\"";
    let workload = Workload::run(
        domain(&pool, "agent", false),
        &[("wrapper", wrapper.into())],
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !workload.dir.join("grow.pid").exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the grower did not start"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(50));
    let grower = std::fs::read_to_string(workload.dir.join("grow.pid"))
        .unwrap()
        .trim()
        .parse::<u32>()
        .unwrap();
    // Its parent ends two seconds after starting it: the first time the grower
    // is a command of its own, it has been growing all along.
    let first = loop {
        assert!(
            std::time::Instant::now() < deadline,
            "the grower never became a command of its own"
        );
        let seen = pool
            .members()
            .iter()
            .flat_map(|member| member.lock().unwrap().commands.clone())
            .find(|command| command.pid == grower);
        if let Some(command) = seen {
            break command;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(
        first.growth >= admission::GROWING,
        "a growing command read as idle on its first sample: {} bytes/s",
        first.growth
    );
}

/// One shell command of an agent starts an idle holder and a grower and
/// waits for them: they are two commands, so the grower (the newest) is paused
/// where the pool is full, and the holder and the shell go on. Were the shell
/// one command it would be the oldest, never paused, and ended with both.
#[cfg(target_os = "linux")]
#[test]
fn the_jobs_of_one_shell_command_are_paused_one_by_one() {
    let pool = pool(1536);
    let jobs = "sh -c 'python3 \"$1/alloc.py\" 800 0 0 & echo $! > \"$1/big.pid\"; python3 \"$1/alloc.py\" 300 48 1000 & echo $! > \"$1/grow.pid\"; wait' sh \"$0\"";
    let mut workload = Workload::run(domain(&pool, "agent", false), &[("jobs", jobs.into())]);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !workload.dir.join("grow.pid").exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the jobs did not start"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    settle(11);
    assert!(
        workload.alive("big") && workload.alive("grow") && workload.alive("jobs"),
        "a job or its shell was ended where the growing job could wait"
    );
    assert!(workload.waiting("grow"), "the growing job was not paused");
    assert!(!workload.waiting("big"), "the idle holder was paused");
}

/// A command paused on a pool that is full by accounting but never stalls
/// (an idle holder keeps its memory) is not left waiting for ever: after the
/// bounded wait it ends with the memory-stop reason, and the holder goes on.
#[cfg(target_os = "linux")]
#[test]
fn a_command_paused_on_a_full_pool_that_never_stalls_ends_after_the_bounded_wait() {
    let (mut a, mut b, _c, _pool) = pressure(false, 300, 48, 9);
    assert!(
        b.waiting("grow"),
        "the newest growing command was not paused"
    );
    settle(3);
    assert!(
        b.alive("grow") && b.waiting("grow"),
        "the pause ended before its bounded wait"
    );
    {
        let mut observation = b.domain.state.lock().unwrap();
        for entry in observation
            .admission
            .values_mut()
            .filter(|e| e.wait.is_some())
        {
            entry.since_ms = crate::session::unix_epoch_ms() - 61_000;
        }
    }
    settle(3);
    assert!(
        !b.alive("grow"),
        "the paused command stayed stopped with no outcome"
    );
    b.assert_told("grow", "stopped this waiting command");
    // The failure names the limit that bound it and the room it left.
    b.assert_told("grow", "under the 1.5 GB shared ceiling");
    assert!(a.alive("big") && a.alive("idle") && a.root_alive() && b.root_alive());
}

/// The size in `notice` that follows `before`, in MB as the notice words it
/// (`size`: "147 MB", "1.5 GB").
#[cfg(target_os = "linux")]
fn size_after(notice: &str, before: &str) -> f64 {
    let mut words = notice
        .split_once(before)
        .unwrap_or_else(|| panic!("the notice has no {before:?}: {notice:?}"))
        .1
        .split_whitespace();
    let number = words
        .next()
        .and_then(|number| number.parse::<f64>().ok())
        .unwrap_or_else(|| panic!("no number after {before:?}: {notice:?}"));
    match words.next() {
        Some("GB") => number * 1024.0,
        Some("MB") => number,
        unit => panic!("no size unit after {before:?}: {unit:?} in {notice:?}"),
    }
}

/// A launch reservation, not what runs, keeps the paused command from
/// continuing: the pool is far from full by resident memory. The notice
/// carries the figures the decision used (what the command needs, what room
/// was left), so that a reader sees why it ended; resident memory against the
/// ceiling would show a pool that is not full.
#[cfg(target_os = "linux")]
#[test]
fn a_pause_held_by_a_launch_reservation_ends_with_the_room_and_need_that_decided() {
    let (a, mut b, _c, pool) = pressure(false, 300, 48, 9);
    assert!(
        b.waiting("grow"),
        "the newest growing command was not paused"
    );
    pool.launches.lock().unwrap().insert(
        "binary".into(),
        super::launch::Reservation {
            memory: 1200 * MIB,
            writes: 0,
            volume: 1,
        },
    );
    // The largest command ends: resident memory is now far below the ceiling.
    unsafe { libc::kill(a.pid("big"), libc::SIGKILL) };
    settle(3);
    assert!(
        b.alive("grow") && b.waiting("grow"),
        "the reservation did not hold the paused command"
    );
    {
        let mut observation = b.domain.state.lock().unwrap();
        for entry in observation
            .admission
            .values_mut()
            .filter(|e| e.wait.is_some())
        {
            entry.since_ms = crate::session::unix_epoch_ms() - 61_000;
        }
    }
    settle(3);
    assert!(
        !b.alive("grow"),
        "the paused command stayed stopped with no outcome"
    );
    let notice = b.told("grow");
    let need = size_after(&notice, "it still needs ");
    let left = size_after(&notice, "only ");
    assert!(
        left < need,
        "the notice does not explain the block: {notice:?}"
    );
    assert!(
        notice.contains("under the 1.5 GB shared ceiling"),
        "{notice:?}"
    );
    assert!(!notice.contains("MiB in use"), "{notice:?}");
    assert!(b.root_alive());
}

/// The same shell, after one of its two jobs ended while the other was
/// paused: the shell runs one job now, but that job stays a command of its
/// own, with its wait. Were it folded back into the shell, its entry would
/// vanish stopped, nothing would ever continue it, and the shell's `wait`
/// would hang.
#[cfg(target_os = "linux")]
#[test]
fn a_paused_job_resumes_when_its_sibling_job_ends() {
    let pool = pool(1536);
    let jobs = "sh -c 'python3 \"$1/alloc.py\" 800 0 0 & echo $! > \"$1/big.pid\"; python3 \"$1/alloc.py\" 300 48 1000 & echo $! > \"$1/grow.pid\"; wait' sh \"$0\"";
    let mut workload = Workload::run(domain(&pool, "agent", false), &[("jobs", jobs.into())]);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !workload.dir.join("grow.pid").exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the jobs did not start"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    settle(11);
    assert!(workload.waiting("grow"), "the growing job was not paused");
    unsafe { libc::kill(workload.pid("big"), libc::SIGKILL) };
    settle(4);
    assert!(
        workload.alive("grow") && workload.alive("jobs"),
        "the job or its shell was ended where room had freed"
    );
    assert!(
        !workload.waiting("grow"),
        "the paused job stayed stopped after its sibling ended, so its shell waits for ever"
    );
}

/// The same shell when a hot reload comes after the sibling ended: the next
/// program watches the shell with one job left and learns the boundary only
/// from the paused command it was handed. Without it the job folds into the
/// shell, and nothing continues the stopped process.
#[cfg(target_os = "linux")]
#[test]
fn a_paused_job_resumes_in_the_watch_a_hot_reload_starts_after_its_sibling_ended() {
    let pool = pool(1536);
    let jobs = "sh -c 'python3 \"$1/alloc.py\" 800 0 0 & echo $! > \"$1/big.pid\"; python3 \"$1/alloc.py\" 300 48 1000 & echo $! > \"$1/grow.pid\"; wait' sh \"$0\"";
    let mut workload = Workload::run(domain(&pool, "agent", false), &[("jobs", jobs.into())]);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !workload.dir.join("grow.pid").exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the jobs did not start"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    settle(11);
    assert!(workload.waiting("grow"), "the growing job was not paused");
    // The previous program's end: what it hands over crosses as JSON, and its
    // watch does nothing from here, so nothing continues the job when its
    // sibling ends.
    let held: Vec<HeldCommand> =
        serde_json::from_slice(&serde_json::to_vec(&pool.held_commands()).unwrap()).unwrap();
    workload.domain.bind_key("hot-reload-previous");
    let _previous_program = control::pause_sampling_for_key("hot-reload-previous");
    unsafe { libc::kill(workload.pid("big"), libc::SIGKILL) };
    settle(2);
    assert!(workload.waiting("grow"), "nothing watched, yet it moved");

    // The next program: its own pool, and a watch of the same shell that sees
    // one job left.
    adopt_admission(held);
    let _next_program = workload.watched_again(domain(&self::pool(1536), "agent", false));
    settle(5);
    assert!(
        workload.alive("grow") && workload.alive("jobs"),
        "the job or its shell was ended where room had freed"
    );
    assert!(
        !workload.waiting("grow"),
        "the paused job stayed stopped in the watch started after its sibling ended"
    );
}

/// Pauses that outlived the bounded wait are ended only while room is still
/// missing for them: when a holder leaves and there is room for all of them,
/// the resume pass continues each, and none ends.
#[cfg(target_os = "linux")]
#[test]
fn overdue_pauses_continue_when_room_returned_for_all_of_them() {
    let pool = pool(1536);
    let holder = Workload::start(domain(&pool, "a", false), &[("big", 800, 0, 0)]);
    // The oldest heavy command is never paused: the growers start only once
    // the pool has seen the holder at its size, so the holder is that one.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !holder
        .domain
        .state
        .lock()
        .unwrap()
        .commands
        .iter()
        .any(|command| command.resident >= 700 * MIB)
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the holder never reached its size"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    // They grow from nothing, so the rate a pause records is their steady
    // one: a command that starts big reads as growing fast, and a pause
    // continues only with room to keep that rate for two seconds.
    let mut growers = Workload::start(
        domain(&pool, "b", false),
        &[("one", 0, 48, 1000), ("two", 0, 48, 1000)],
    );
    let both_paused = |growers: &Workload| growers.waiting("one") && growers.waiting("two");
    let deadline = std::time::Instant::now() + Duration::from_secs(40);
    while !both_paused(&growers) {
        assert!(
            std::time::Instant::now() < deadline,
            "the two growers were not both paused"
        );
        settle(1);
    }
    // Together they could not have grown back: only the holder's room frees.
    // The holder's watch is held at its last sample, which still counts `big`;
    // it samples again two seconds after the kill.
    holder.domain.bind_key("overdue-room-returned-holder");
    let held = control::pause_sampling_for_key("overdue-room-returned-holder");
    unsafe { libc::kill(holder.pid("big"), libc::SIGKILL) };
    std::thread::spawn(move || {
        settle(2);
        drop(held);
    });
    // Age the pauses only once the pool has seen the room return: the
    // growers' watches decide on the holder's last sample.
    let big = holder.pid("big") as u32;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while holder
        .domain
        .state
        .lock()
        .unwrap()
        .samples
        .iter()
        .any(|sample| sample.pid == big)
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the holder's watch never saw its command go"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    {
        let mut observation = growers.domain.state.lock().unwrap();
        for entry in observation
            .admission
            .values_mut()
            .filter(|e| e.wait.is_some())
        {
            entry.since_ms = crate::session::unix_epoch_ms() - 61_000;
        }
    }
    settle(6);
    assert!(
        growers.alive("one") && growers.alive("two"),
        "a paused command was ended where the room had returned for all of them"
    );
    assert!(
        !growers.waiting("one") && !growers.waiting("two"),
        "a paused command did not continue"
    );
    assert!(growers.root_alive());
}

/// A released workload whose root ended: the tool shell that survived it is no
/// longer under the harness, yet the next program learns its job boundary from
/// the paused command it was handed, and continues that command with room.
#[cfg(target_os = "linux")]
#[test]
fn a_paused_job_of_a_released_workload_resumes_after_its_root_ended() {
    let pool = pool(1536);
    let jobs = "sh -c 'python3 \"$1/alloc.py\" 800 0 0 & echo $! > \"$1/big.pid\"; python3 \"$1/alloc.py\" 300 48 1000 & echo $! > \"$1/grow.pid\"; wait' sh \"$0\"";
    let mut workload = Workload::run(domain(&pool, "agent", false), &[("jobs", jobs.into())]);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !workload.dir.join("grow.pid").exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the jobs did not start"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    settle(11);
    assert!(workload.waiting("grow"), "the growing job was not paused");
    let held: Vec<HeldCommand> =
        serde_json::from_slice(&serde_json::to_vec(&pool.held_commands()).unwrap()).unwrap();

    // The previous program releases the workload and ends; the provider's
    // root exits and its tool shell is reparented outside the harness.
    let grow = workload.pid("grow");
    let session = "released-paused-room";
    let owner = SessionProcessOwner::Agent("released-paused-agent".into());
    workload.domain.bind_key("released-previous");
    let _previous_program = control::pause_sampling_for_key("released-previous");
    workload.domain.release(session, owner.clone(), None);
    workload.child.kill().unwrap();
    workload.child.wait().unwrap();
    unsafe { libc::kill(workload.pid("big"), libc::SIGKILL) };
    settle(2);
    assert!(workload.waiting("grow"), "nothing watched, yet it moved");
    let handed = serde_json::to_vec(&handover::hand_over_released())
        .and_then(|body| serde_json::from_slice::<Vec<handover::ReleasedJobs>>(&body))
        .unwrap()
        .into_iter()
        .filter(|jobs| jobs.pids().contains(&(grow as u32)))
        .collect::<Vec<_>>();
    assert_eq!(handed.len(), 1, "the released workload is handed over");
    registry()
        .lock()
        .unwrap()
        .retain(|_, state| !state.ptr_eq(&Arc::downgrade(&workload.domain.state)));

    adopt_admission(held);
    handover::adopt_released(handed);
    settle(5);
    assert!(
        workload.alive("grow") && workload.alive("jobs"),
        "the job or its shell was ended where room had freed"
    );
    assert!(
        !workload.waiting("grow"),
        "the paused job of the released workload stayed stopped"
    );
    stop_released(Some(session), Some(&owner));
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

/// A slowly growing command takes a one-off 500 MiB burst after several
/// samples: the pause it earned does not ask
/// for room to keep growing at the spike's rate. It continues with the room
/// there is and is not ended.
#[cfg(target_os = "linux")]
#[test]
fn a_startup_spike_does_not_keep_a_slow_grower_paused() {
    let pool = pool(1536);
    let mut holder = Workload::gated(
        domain(&pool, "holder", false),
        &[("hold", "python3 \"$0/hold.py\" \"$0/room\"".into())],
    );
    std::fs::write(holder.dir.join("hold.py"), format!(
        "{DIE_WITH_PARENT}import os,time\na=bytearray(500*1048576); b=bytearray(300*1048576)\nwhile not os.path.exists(sys.argv[1]): time.sleep(0.05)\ndel a\ntime.sleep(60)"
    )).unwrap();
    holder.go(&["hold"]);
    settle(1);
    let mut grower = Workload::gated(
        domain(&pool, "grower", false),
        &[("grow", "python3 \"$0/grow.py\" \"$0/burst\"".into())],
    );
    std::fs::write(grower.dir.join("grow.py"), format!(
        "{DIE_WITH_PARENT}import os,time\nb=[bytearray(100*1048576)]\nwhile not os.path.exists(sys.argv[1]):\n b.append(bytearray(1048576)); time.sleep(0.25)\nb.append(bytearray(500*1048576))\nfor _ in range(120):\n b.append(bytearray(1048576)); time.sleep(0.25)"
    )).unwrap();
    grower.go(&["grow"]);
    settle(3);
    std::fs::write(grower.dir.join("burst"), "").unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !grower.waiting("grow") {
        assert!(
            std::time::Instant::now() < deadline,
            "the burst never caused a pause"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // Enough room for slow growth, but not for another burst every sample.
    std::fs::write(holder.dir.join("room"), "").unwrap();
    settle(3);
    assert!(
        holder.alive("hold") && grower.alive("grow"),
        "a command was ended"
    );
    assert!(
        !grower.waiting("grow"),
        "the pause kept the burst's rate and never continued"
    );
}

/// A command that grows fast all along keeps the rate it was paused at: it
/// needs room for that growth, stays stopped (a stopped process measures none
/// and is not paused again and again) and ends after 60 s naming what it needs.
#[cfg(target_os = "linux")]
#[test]
fn a_steady_fast_grower_stays_paused_once_and_ends_with_the_reason() {
    take_wait_events();
    const RATE: u64 = 200;
    let pool = pool(1536);
    let holder = Workload::gated(
        domain(&pool, "a", false),
        &[
            ("big", "python3 \"$0/holder.py\" \"$0/pressure\"".into()),
            ("idle", "python3 \"$0/alloc.py\" 0 0 0".into()),
        ],
    );
    std::fs::write(holder.dir.join("holder.py"), format!(
        "{DIE_WITH_PARENT}import os,time\nheld=[bytearray(100*1048576)]\nwhile not os.path.exists(sys.argv[1]): time.sleep(0.02)\nheld.append(bytearray(700*1048576))\ntime.sleep(60)"
    )).unwrap();
    holder.go(&["big", "idle"]);
    let holder_pid = u32::try_from(holder.pid("big")).expect("the holder PID should be positive");
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let ready = {
            let observed = holder.domain.state.lock().unwrap();
            observed.admission.contains_key(&holder_pid)
                && observed.commands.iter().any(|command| {
                    command.pid == holder_pid
                        && command.resident >= 100 * MIB
                        && command.measured_growth == 0
                })
        };
        if ready {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the initial holder was never sampled"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let _c = Workload::start(domain(&pool, "c", false), &[("small", 20, 0, 0)]);
    let mut b = Workload::start(
        domain(&pool, "b", false),
        &[("idle", 0, 0, 0), ("grow", 100, RATE, 1000)],
    );
    // Observe a real steady-rate window before another owner applies pressure.
    let grower_pid = u32::try_from(b.pid("grow")).expect("the grower PID should be positive");
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let ready = b
            .domain
            .state
            .lock()
            .unwrap()
            .commands
            .iter()
            .any(|command| {
                command.pid == grower_pid
                    && command.resident >= 300 * MIB
                    && (RATE * 3 * MIB / 4..=RATE * 5 * MIB / 4).contains(&command.steady_growth)
                    && command.measured_growth >= admission::GROWING
            });
        if ready && !b.waiting("grow") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the grower never established its steady rate"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // Start observing waits once startup has established the test's steady phase.
    take_wait_events();
    std::fs::write(holder.dir.join("pressure"), "").unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while !b.waiting("grow") {
        assert!(
            std::time::Instant::now() < deadline,
            "the added holder never caused pressure"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(b.waiting("grow"), "the fast grower was not paused");
    settle(3);
    assert!(
        b.waiting("grow"),
        "the paused grower continued without room"
    );
    let began = take_wait_events()
        .into_iter()
        .filter(|event| matches!(event.change, WaitChange::Began))
        .count();
    assert_eq!(began, 1, "the grower was paused {began} times");
    {
        let mut observation = b.domain.state.lock().unwrap();
        for entry in observation
            .admission
            .values_mut()
            .filter(|e| e.wait.is_some())
        {
            entry.since_ms = crate::session::unix_epoch_ms() - 61_000;
        }
    }
    settle(3);
    assert!(
        !b.alive("grow"),
        "the paused grower stayed stopped for ever"
    );
    b.assert_told("grow", "it still needs ");
}

/// The kernel's own pool acts on what the machine reports, not on its own
/// accounting of resident bytes: a pool full by that accounting while the
/// machine has memory pauses and stops nothing.
#[cfg(target_os = "linux")]
#[test]
fn a_pool_full_by_accounting_alone_pauses_nothing_while_the_machine_has_memory() {
    let pool =
        Arc::new(pool::Pool::new(1536 * MIB, true).with_machine_pressure(Box::new(|| false)));
    let (mut a, mut b, mut c, _pool) = pressure_in(pool, false, 300, 48, 20);
    assert!(
        !b.waiting("grow"),
        "a command was paused with no pressure on the machine"
    );
    assert!(
        a.alive("big") && a.alive("idle") && b.alive("grow") && c.alive("small"),
        "a command was stopped with no pressure on the machine"
    );
    assert!(a.root_alive() && b.root_alive() && c.root_alive());
}

/// Plenty of host memory does not hide the workloads' own finite allocation
/// boundary. The kernel must pause before that cgroup reaches memory.max.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires a writable cgroup-v2 delegated parent"]
fn a_full_budget_pauses_even_when_the_host_reports_no_pressure() {
    let pool =
        Arc::new(pool::Pool::new(1536 * MIB, true).with_machine_pressure(Box::new(|| false)));
    let (mut a, mut b, mut c, _pool) = pressure_in(pool, true, 300, 48, 10);
    assert!(
        b.waiting("grow"),
        "the finite cgroup's pressure was ignored"
    );
    assert!(a.alive("big") && b.alive("grow") && c.alive("small"));
    assert!(a.root_alive() && b.root_alive() && c.root_alive());
}
