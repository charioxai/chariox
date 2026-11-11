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
        self.child.try_wait().unwrap().is_none()
    }
    /// The stopped command was told why on its own error stream. Only Linux
    /// can reopen another process's stream; elsewhere the stop is silent.
    pub(super) fn assert_told(&self, name: &str, reason: &str) {
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
    assert!(a.alive("big") && a.alive("idle") && a.root_alive() && b.root_alive());
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
