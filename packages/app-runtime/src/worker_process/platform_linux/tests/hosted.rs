//! Real delegated cgroups and noexec mounts, disposable hosted Linux only.
use super::super::*;
use crate::worker_process::{
    record::LaunchRecord, spawn, PreparedWorker, ResourceDomain, WorkerLimits, WorkerProcess,
};
use std::{
    ffi::CString,
    fs::{self, File},
    io::{Read, Write},
    os::{fd::OwnedFd, unix::net::UnixStream},
    path::PathBuf,
    time::{Duration, Instant},
};

fn scratch() -> PathBuf {
    assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(), Ok("true"));
    assert_eq!(
        std::env::var("RUNNER_ENVIRONMENT").as_deref(),
        Ok("github-hosted")
    );
    assert_eq!(
        std::env::var("GITHUB_REPOSITORY").as_deref(),
        Ok("charioxai/chariox")
    );
    assert_ne!(unsafe { libc::geteuid() }, 0);
    let root = PathBuf::from(std::env::var_os("CHARIOX_LINUX_DOMAIN_SCRATCH").unwrap());
    assert_eq!(fs::canonicalize(&root).unwrap(), root);
    assert_eq!(
        root.parent(),
        Some(
            fs::canonicalize(std::env::var_os("RUNNER_TEMP").unwrap())
                .unwrap()
                .as_path()
        )
    );
    assert!(root
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("chariox-sandbox."));
    root
}

fn delegation() -> cgroup::Delegation {
    let path = PathBuf::from(std::env::var_os("CHARIOX_LINUX_DOMAIN_CGROUP").unwrap());
    assert!(path.starts_with("/sys/fs/cgroup/system.slice"));
    cgroup::Delegation::open(&path).unwrap()
}
fn binding(path: PathBuf) -> plan::Binding {
    plan::Binding {
        object: File::open(&path).unwrap(),
        path,
    }
}

struct ObservedDomain(domain::Domain);
impl ResourceDomain for ObservedDomain {
    fn setup_descriptors(&self) -> &[File] {
        self.0.setup_descriptors()
    }
    fn verify_before_continue(&mut self, pid: i32) -> Result<()> {
        assert!(
            !self.0._roots[1].path.join("constructor-ran").exists(),
            "runtime constructor ran before inspection/Continue"
        );
        self.0.verify_before_continue(pid)
    }
    fn check_running(&mut self, pid: i32, now: Instant) -> Result<()> {
        self.0.check_running(pid, now)
    }
    fn terminate(&mut self, pid: i32) {
        self.0.terminate(pid);
    }
    fn reap_domain_blocking(&mut self) {
        self.0.reap_domain_blocking();
    }
}

#[test]
#[ignore = "dedicated disposable hosted Linux resource/mount fixture only"]
fn hosted_native_worker_uses_production_cgroup_and_observer() {
    let scratch = scratch();
    let roots = ["package", "data", "tmp", "runtime"]
        .map(|name| binding(scratch.join("domain-mounts").join(name)));
    let libc = fs::canonicalize("/lib/x86_64-linux-gnu/libc.so.6").unwrap();
    let loader = fs::canonicalize("/lib64/ld-linux-x86-64.so.2").unwrap();
    let libraries = vec![
        ("/lib/x86_64-linux-gnu/libc.so.6".into(), binding(libc)),
        ("/lib64/ld-linux-x86-64.so.2".into(), binding(loader)),
    ];
    let arguments = plan::arguments(&roots, &libraries).unwrap();
    let leaf = delegation().create().unwrap();
    let cgroup_path = leaf.path.clone();
    let setup = [
        leaf.processes.try_clone().unwrap(),
        File::open(scratch.join("bin/chariox-bwrap")).unwrap(),
    ];
    let executable = File::open(roots[3].path.join("chariox-app-worker")).unwrap();
    let domain = domain::Domain {
        leaf,
        observer: inspection::Observer::new(executable).unwrap(),
        setup,
        _roots: roots,
        _libraries: libraries,
        storage: None,
        _runtime: None,
        _release: None,
    };
    let prepared = PreparedWorker {
        program: CString::new(
            scratch
                .join("bin/chariox-app-domain-entry")
                .to_str()
                .unwrap(),
        )
        .unwrap(),
        arguments,
        record: LaunchRecord {
            generation: "7".into(),
            installation: "linux_fixture".into(),
            release_digest: format!("sha256:{}", "a".repeat(64)),
            roots: plan::ROOTS.map(str::to_owned),
            bootstrap: "probe-v1".into(),
            nofile: 64,
            cpu_seconds: 10,
            heap_mib: 64,
            v8_threads: 1,
            max_file_bytes: 1048576,
        },
        _objects: vec![],
        domain: Box::new(ObservedDomain(domain)),
    };
    let mut worker = WorkerProcess::spawn_blocking(prepared, WorkerLimits::default()).unwrap();
    // The native libc probe deliberately speaks only four raw bytes on the
    // inherited SDK socket. This test does not claim Node/SDK protocol evidence.
    let mut channel = worker.sdk.take().unwrap();
    channel.set_nonblocking(false).unwrap();
    channel
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    channel
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    channel.write_all(b"PING").unwrap();
    let mut response = [0; 4];
    channel.read_exact(&mut response).unwrap();
    assert_eq!(&response, b"PONG");
    let result = worker.wait_blocking().unwrap();
    let output = String::from_utf8(result.stdout_tail).unwrap();
    println!("{output}");
    assert_eq!(
        result.code,
        Some(0),
        "native stdout={output} stderr={:?}",
        result.stderr_tail
    );
    assert_eq!(result.failure, None);
    assert!(
        !cgroup_path.exists(),
        "resource lease must remove its empty leaf after reap"
    );
    println!("production_linux_cgroup_namespace_observer_native_probe_passed");
}

struct Child {
    pid: i32,
    leaf: cgroup::Lease,
}
impl Drop for Child {
    fn drop(&mut self) {
        self.leaf.terminate();
        let mut status = 0;
        loop {
            if unsafe { libc::waitpid(self.pid, &mut status, 0) } >= 0 {
                break;
            }
            if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                break;
            }
        }
        self.leaf.reap_blocking();
    }
}

#[test]
#[ignore = "dedicated disposable hosted Linux delegated cgroup only"]
fn hosted_entry_constrains_immediate_fork_before_parent_can_observe_it() {
    let scratch = scratch();
    let leaf = delegation().create().unwrap();
    let path = leaf.path.clone();
    let target = File::open(scratch.join("bin/domain-immediate-fork")).unwrap();
    // The entry must exec the selected descriptor even if its former path is
    // replaced. Immutable artifact enrollment remains a separate production gate.
    fs::rename(
        scratch.join("bin/domain-immediate-fork"),
        scratch.join("bin/held-fork-fixture"),
    )
    .unwrap();
    fs::write(
        scratch.join("bin/domain-immediate-fork"),
        b"not the selected executable",
    )
    .unwrap();
    let null = File::options()
        .read(true)
        .write(true)
        .open("/dev/null")
        .unwrap();
    let (mut sdk, child_sdk) = UnixStream::pair().unwrap();
    sdk.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let child_sdk = File::from(OwnedFd::from(child_sdk));
    let pid = spawn::launch(
        &CString::new(
            scratch
                .join("bin/chariox-app-domain-entry")
                .to_str()
                .unwrap(),
        )
        .unwrap(),
        &[CString::new("domain-immediate-fork").unwrap()],
        &[
            &null,
            &null,
            &null,
            &child_sdk,
            &null,
            &leaf.processes,
            &target,
        ],
    )
    .unwrap();
    let owned = Child { pid, leaf };
    drop(child_sdk);
    let mut bytes = [0; 8];
    sdk.read_exact(&mut bytes).unwrap();
    let reported_parent = i32::from_ne_bytes(bytes[..4].try_into().unwrap());
    let reported_child = i32::from_ne_bytes(bytes[4..].try_into().unwrap());
    assert_eq!(reported_parent, pid);
    let mut members = owned.leaf.members().unwrap();
    members.sort_unstable();
    let mut expected = [pid, reported_child];
    expected.sort_unstable();
    assert_eq!(members, expected);
    owned.leaf.verify_limits().unwrap();
    drop(owned);
    assert!(!path.exists());
    println!("pre_exec_cgroup_membership_inherited_by_immediate_fork_and_reaped");
}
