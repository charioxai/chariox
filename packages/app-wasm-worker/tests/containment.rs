//! Native hostile code probes the same mandatory boundary as serve. These
//! probes deliberately bypass WASM; a component cannot import libc on the base.
use std::{
    fs,
    io::{Read, Write},
    os::unix::{net::UnixStream, process::CommandExt},
    process::{Command, Stdio},
};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "--spawn-probe") {
        return;
    }
    if args.get(1).is_some_and(|arg| arg == "--child") {
        child(&args[2], &args[3]);
        return;
    }
    let root = std::env::var_os("CHARIOX_WORKER_DRILL_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("native-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let outside = root.join("outside");
    fs::write(&outside, b"not an App file").unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    // Offline compilation may start platform runtime threads. The contained
    // child must only read these trusted bytes before entering its boundary,
    // just as a supervisor prepares the production AOT input.
    let aot = root.join("trusted.aot");
    let compiler = chariox_app_wasm_worker::worker::WorkerEngine::new().unwrap();
    fs::write(
        &aot,
        compiler
            .engine()
            .precompile_module(
                &wat::parse_str("(module (func (export \"answer\") (result i32) i32.const 42))")
                    .unwrap(),
            )
            .unwrap(),
    )
    .unwrap();
    drop(compiler);
    let executable = std::env::current_exe().unwrap();
    let mut failures = 0;
    for scenario in [
        "authority",
        "memory",
        "cpu",
        "file",
        "aot",
        "without-landlock",
        "cpu-budget",
        "icache",
        "unsigned-code",
        "installation-files",
        "runtime",
    ] {
        if matches!(scenario, "memory" | "without-landlock" | "icache") && cfg!(target_os = "macos")
        {
            println!("NATIVE {scenario}: NOT RUN (Linux-only native probe)");
            continue;
        }
        if scenario == "unsigned-code" && !cfg!(target_os = "macos") {
            println!("NATIVE unsigned-code: NOT RUN (macOS code-signing probe)");
            continue;
        }
        for mode in ["red", "green"] {
            let red = std::env::var_os("CHARIOX_UNCONTAINED_PROBE");
            let binary = if mode == "red" {
                red.as_deref()
                    .map(std::path::Path::new)
                    .unwrap_or(&executable)
            } else {
                &executable
            };
            let mut command = Command::new(binary);
            command
                .args(["--child", scenario, mode])
                .env("PROBE_OUTSIDE", &outside)
                .env("PROBE_AOT", &aot)
                .env("PROBE_LISTENER", listener.local_addr().unwrap().to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            // Lower inherited ceilings make resource stops fast. Production can
            // only tighten them and the probe verifies attempts to raise them.
            unsafe {
                command.pre_exec(move || {
                    if mode == "red" {
                        return Ok(());
                    }
                    #[cfg(target_os = "linux")]
                    if scenario == "without-landlock" {
                        without_landlock()?;
                    }
                    let (resource, value) = match scenario {
                        "cpu" => (libc::RLIMIT_CPU, 1),
                        "file" => (libc::RLIMIT_FSIZE, 4096),
                        _ => return Ok(()),
                    };
                    let limit = libc::rlimit {
                        rlim_cur: value,
                        rlim_max: value + if scenario == "cpu" { 1 } else { 0 },
                    };
                    if libc::setrlimit(resource, &limit) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            if scenario == "file" {
                command.stdout(fs::File::create(root.join(format!("{mode}.output"))).unwrap());
            }
            let output = command.output().unwrap();
            use std::os::unix::process::ExitStatusExt;
            let expected_signal = match (scenario, mode) {
                ("cpu", "green") => Some(libc::SIGXCPU),
                ("file", "green") => Some(libc::SIGXFSZ),
                ("unsigned-code", "green") if cfg!(target_os = "macos") => Some(libc::SIGKILL),
                _ => None,
            };
            let passed = if let Some(signal) = expected_signal {
                output.status.signal() == Some(signal)
            } else {
                output.status.success()
            };
            failures += usize::from(!passed);
            println!(
                "NATIVE {scenario}/{mode}: {} status={} {} {}",
                if passed { "PASS" } else { "FAIL" },
                output.status,
                String::from_utf8_lossy(&output.stdout).trim(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
    }
    fs::remove_dir_all(root).unwrap();
    assert_eq!(failures, 0, "native containment scenarios failed");
}

fn child(scenario: &str, mode: &str) {
    let outside = std::env::var("PROBE_OUTSIDE").unwrap();
    let executable = std::env::current_exe().unwrap();
    let original_cpu_budget = cpu_budget();
    let (reader, writer) = UnixStream::pair().unwrap();
    // Cargo may leave jobserver descriptors in the child. Place broker IPC at
    // its real ABI positions rather than assuming socketpair returns FD3/4.
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    let relocate = |fd| {
        let copy = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 10) };
        assert!(copy >= 10);
        unsafe { OwnedFd::from_raw_fd(copy) }
    };
    let left = relocate(reader.as_raw_fd());
    let right = relocate(writer.as_raw_fd());
    drop((reader, writer));
    assert_eq!(unsafe { libc::dup2(left.as_raw_fd(), 3) }, 3);
    assert_eq!(unsafe { libc::dup2(right.as_raw_fd(), 4) }, 4);
    drop((left, right));
    let mut reader = unsafe { UnixStream::from_raw_fd(3) };
    let mut writer = unsafe { UnixStream::from_raw_fd(4) };
    // The worker must discard even an accidentally inherited outside-file FD.
    // A path-only policy would leave this descriptor usable.
    use std::os::fd::IntoRawFd;
    let outside_fd = fs::File::open(&outside).unwrap().into_raw_fd();
    assert_eq!(unsafe { libc::dup2(outside_fd, 80) }, 80);
    unsafe {
        libc::close(outside_fd);
    }
    let aot = if scenario == "aot" {
        Some(fs::read(std::env::var_os("PROBE_AOT").unwrap()).unwrap())
    } else {
        None
    };
    let green = mode == "green";
    // An installed App's data and tmp directories, created before the boundary
    // as the kernel does; they must be the only native file authority left.
    let data = std::path::Path::new(&outside).with_file_name(format!("data-{mode}"));
    let tmp = data.with_file_name(format!("tmp-{mode}"));
    let files = if scenario == "installation-files" {
        fs::create_dir(&data).unwrap();
        fs::create_dir(&tmp).unwrap();
        vec![data.clone(), tmp.clone()]
    } else {
        vec![]
    };
    if green && scenario == "runtime" {
        chariox_app_wasm_worker::cli::contain(&files).unwrap();
    } else if green {
        chariox_app_wasm_worker::containment::enter(&files).unwrap();
    }
    match scenario {
        "authority" | "without-landlock" => {
            assert_eq!(
                unsafe { libc::fcntl(80, libc::F_GETFD) } < 0,
                green,
                "inherited outside-file FD"
            );
            let read = fs::read(&outside);
            assert_eq!(read.is_err(), green, "native file escape");
            if green {
                let errno = read.unwrap_err().raw_os_error();
                assert!(matches!(errno, Some(libc::EPERM) | Some(libc::EACCES)));
            }
            let socket = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
            // Seatbelt mediates network use, not allocation of an unconnected
            // socket. Linux seccomp also refuses the socket syscall itself.
            if cfg!(target_os = "linux") || !green {
                assert_eq!(socket < 0, green, "native socket allocation");
            }
            if green && cfg!(target_os = "linux") {
                assert_eq!(
                    std::io::Error::last_os_error().raw_os_error(),
                    Some(libc::EPERM)
                );
            }
            if socket >= 0 {
                unsafe {
                    libc::close(socket);
                }
            }
            let connected = std::net::TcpStream::connect(std::env::var("PROBE_LISTENER").unwrap());
            assert_eq!(
                connected.is_err(),
                green,
                "native network connection: {connected:?}"
            );
            let spawned = Command::new(executable).arg("--spawn-probe").status();
            assert_eq!(spawned.is_err(), green, "native process spawn: {spawned:?}");
            if green {
                assert_eq!(spawned.unwrap_err().raw_os_error(), Some(libc::EPERM));
            } else {
                assert!(spawned.unwrap().success());
            }
            #[cfg(target_os = "linux")]
            if green {
                let trace = unsafe { libc::ptrace(libc::PTRACE_ATTACH, libc::getppid(), 0, 0) };
                assert_eq!(trace, -1);
                assert_eq!(
                    std::io::Error::last_os_error().raw_os_error(),
                    Some(libc::EPERM)
                );
            }
            let rwx = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    4096,
                    libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
                    libc::MAP_PRIVATE | libc::MAP_ANON,
                    -1,
                    0,
                )
            };
            if green || cfg!(target_os = "linux") {
                assert_eq!(rwx == libc::MAP_FAILED, green, "native RWX map");
            }
            if rwx != libc::MAP_FAILED {
                unsafe {
                    libc::munmap(rwx, 4096);
                }
            }
            let rw = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    4096,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_PRIVATE | libc::MAP_ANON,
                    -1,
                    0,
                )
            };
            assert_ne!(rw, libc::MAP_FAILED);
            let protect = unsafe {
                libc::mprotect(
                    rw,
                    4096,
                    libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
                )
            };
            if green || cfg!(target_os = "linux") {
                assert_eq!(protect < 0, green, "native RWX transition");
            }
            let rx = unsafe { libc::mprotect(rw, 4096, libc::PROT_READ | libc::PROT_EXEC) };
            // Darwin validates code lazily when it executes, rather than
            // refusing this transition. The unsigned-code child probes that
            // enforcement; the AOT child separately proves admitted JIT code.
            assert_eq!(rx, 0);
            unsafe {
                libc::munmap(rw, 4096);
            }
            if green {
                let unlimited = libc::rlimit {
                    rlim_cur: libc::RLIM_INFINITY,
                    rlim_max: libc::RLIM_INFINITY,
                };
                #[cfg(target_os = "linux")]
                let resource = libc::RLIMIT_AS;
                #[cfg(target_os = "macos")]
                let resource = libc::RLIMIT_NOFILE;
                assert_eq!(unsafe { libc::setrlimit(resource, &unlimited) }, -1);
            }
            let thread = std::thread::spawn(move || {
                if green {
                    assert!(
                        fs::read(outside).is_err(),
                        "pthread inherited filesystem policy"
                    );
                    assert!(
                        std::net::TcpListener::bind("127.0.0.1:0").is_err(),
                        "pthread inherited socket policy"
                    );
                }
                7
            });
            assert_eq!(thread.join().unwrap(), 7);
            writer.write_all(b"broker").unwrap();
            let mut data = [0; 6];
            reader.read_exact(&mut data).unwrap();
            assert_eq!(&data, b"broker");
            println!("file/socket/spawn/ptrace/W^X/pthread/inherited-IPC checked");
        }
        "memory" => {
            let limited = green && cfg!(target_os = "linux");
            let length = 2usize << 30;
            let memory = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    length,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_PRIVATE | libc::MAP_ANON,
                    -1,
                    0,
                )
            };
            assert_eq!(
                memory == libc::MAP_FAILED,
                limited,
                "native address-space ceiling"
            );
            if limited {
                assert_eq!(
                    std::io::Error::last_os_error().raw_os_error(),
                    Some(libc::ENOMEM)
                );
            }
            if memory != libc::MAP_FAILED {
                unsafe {
                    libc::munmap(memory, length);
                }
            }
            println!(
                "APP_NATIVE_MEMORY_LIMIT: mmap {}",
                if limited {
                    "ENOMEM"
                } else if green {
                    "allowed (no native address-space cap on macOS)"
                } else {
                    "allowed"
                }
            );
        }
        "cpu" => {
            let start = std::time::Instant::now();
            while start.elapsed().as_secs() < 2 {
                std::hint::spin_loop();
            }
        }
        "cpu-budget" => {
            assert_eq!(
                cpu_budget(),
                original_cpu_budget,
                "introduced a lifetime CPU budget"
            );
            println!("healthy generation retains its inherited CPU lifetime budget");
        }
        #[cfg(target_os = "linux")]
        "icache" => {
            for command in [64u64, 32] {
                let result = unsafe { libc::syscall(libc::SYS_membarrier, command, 0u64) };
                if green && cfg!(target_arch = "x86_64") {
                    assert_eq!(result, -1);
                    assert_eq!(
                        std::io::Error::last_os_error().raw_os_error(),
                        Some(libc::EPERM)
                    );
                } else {
                    assert_eq!(
                        result,
                        0,
                        "private icache membarrier: {}",
                        std::io::Error::last_os_error()
                    );
                }
            }
            if green {
                for (command, flags) in [
                    (0u64, 0u64),
                    (1, 0),
                    (32, 1),
                    (64, 1),
                    (32 | (1 << 32), 0),
                    (64, 1 << 32),
                ] {
                    assert_eq!(
                        unsafe { libc::syscall(libc::SYS_membarrier, command, flags) },
                        -1
                    );
                    assert_eq!(
                        std::io::Error::last_os_error().raw_os_error(),
                        Some(libc::EPERM)
                    );
                }
            }
            println!("private icache commands and rejected command/flag variants checked");
        }
        "file" => {
            std::io::stdout().write_all(&[b'x'; 8192]).unwrap();
        }
        #[cfg(target_os = "macos")]
        "unsigned-code" => {
            #[cfg(target_arch = "aarch64")]
            let instructions: &[u8] = &[0x40, 0x05, 0x80, 0x52, 0xc0, 0x03, 0x5f, 0xd6];
            #[cfg(target_arch = "x86_64")]
            let instructions: &[u8] = &[0xb8, 42, 0, 0, 0, 0xc3];
            let memory = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    4096,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_PRIVATE | libc::MAP_ANON,
                    -1,
                    0,
                )
            };
            assert_ne!(memory, libc::MAP_FAILED);
            unsafe {
                std::ptr::copy_nonoverlapping(
                    instructions.as_ptr(),
                    memory.cast::<u8>(),
                    instructions.len(),
                );
                assert_eq!(
                    libc::mprotect(memory, 4096, libc::PROT_READ | libc::PROT_EXEC),
                    0
                );
                unsafe extern "C" {
                    fn sys_icache_invalidate(start: *mut libc::c_void, length: usize);
                }
                sys_icache_invalidate(memory, instructions.len());
                // A hardened process must be killed here: this is anonymous
                // unsigned code, deliberately outside any MAP_JIT allocation.
                let call: unsafe extern "C" fn() -> u32 = std::mem::transmute(memory);
                assert_eq!(call(), 42);
                libc::munmap(memory, 4096);
            }
        }
        "aot" => {
            let engine = chariox_app_wasm_worker::worker::WorkerEngine::new().unwrap();
            // These bytes were precompiled by this exact executable/config,
            // never read from a package or an unverified native cache.
            let module = unsafe { engine.deserialize_module(aot.as_ref().unwrap()) }.unwrap();
            let mut store = wasmtime::Store::new(engine.engine(), ());
            store.set_epoch_deadline(u64::MAX);
            let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
            assert_eq!(
                instance
                    .get_typed_func::<(), i32>(&mut store, "answer")
                    .unwrap()
                    .call(&mut store, ())
                    .unwrap(),
                42
            );
            println!("trusted precompiled code loads and executes after containment");
        }
        "runtime" => {
            // The worker builds its runtime after containment; a socketpair for
            // tokio's signal driver would be refused there.
            let runtime = chariox_app_wasm_worker::cli::runtime().unwrap();
            assert_eq!(
                runtime.block_on(async { tokio::spawn(async { 42 }).await.unwrap() }),
                42
            );
            // The seccomp filter's socket rules exist only on Linux.
            #[cfg(target_os = "linux")]
            {
                // Only the filter answers EPERM; the kernel would answer these with
                // EAFNOSUPPORT or EPROTONOSUPPORT, or succeed.
                let pair = |domain, kind, protocol| {
                    let mut fds = [0; 2];
                    let result =
                        unsafe { libc::socketpair(domain, kind, protocol, fds.as_mut_ptr()) };
                    if result == 0 {
                        unsafe {
                            libc::close(fds[0]);
                            libc::close(fds[1]);
                        }
                    }
                    (result == 0, std::io::Error::last_os_error().raw_os_error())
                };
                let refused =
                    |(opened, errno): (bool, Option<i32>)| !opened && errno == Some(libc::EPERM);
                let stream = libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK;
                assert!(pair(libc::AF_UNIX, stream, 0).0, "in-process stream pair");
                for (name, attempt) in [
                    ("datagram", pair(libc::AF_UNIX, libc::SOCK_DGRAM, 0)),
                    ("seqpacket", pair(libc::AF_UNIX, libc::SOCK_SEQPACKET, 0)),
                    ("raw", pair(libc::AF_UNIX, libc::SOCK_RAW, 0)),
                    ("inet", pair(libc::AF_INET, libc::SOCK_STREAM, 0)),
                    ("netlink", pair(libc::AF_NETLINK, libc::SOCK_RAW, 0)),
                    ("protocol", pair(libc::AF_UNIX, libc::SOCK_STREAM, 1)),
                ] {
                    assert_eq!(refused(attempt), green, "{name} socketpair");
                }
            }
            println!("async runtime starts and runs tasks after containment; only in-process stream pairs open");
        }
        "installation-files" => {
            fs::write(data.join("note"), b"App file").unwrap();
            fs::create_dir(data.join("nested")).unwrap();
            fs::rename(data.join("note"), data.join("nested/note")).unwrap();
            assert_eq!(fs::read(data.join("nested/note")).unwrap(), b"App file");
            fs::remove_file(data.join("nested/note")).unwrap();
            fs::write(tmp.join("scratch"), b"App tmp").unwrap();
            assert_eq!(fs::read(tmp.join("scratch")).unwrap(), b"App tmp");
            fs::remove_file(tmp.join("scratch")).unwrap();
            let read = fs::read(&outside);
            assert_eq!(read.is_err(), green, "file outside the installation");
            let written = fs::write(data.with_file_name("escape"), b"x");
            assert_eq!(written.is_err(), green, "write outside the installation");
            // A link the App makes inside its own files cannot reach outside.
            std::os::unix::fs::symlink(&outside, data.join("link")).unwrap();
            let linked = fs::read(data.join("link"));
            assert_eq!(linked.is_err(), green, "symlink out of the installation");
            if green {
                // Landlock answers EACCES; Seatbelt answers EPERM.
                for (operation, result) in [
                    ("read", read),
                    ("write", written.map(|()| Vec::new())),
                    ("symlink", linked),
                ] {
                    let errno = result.unwrap_err().raw_os_error();
                    let refused = if cfg!(target_os = "linux") {
                        errno == Some(libc::EACCES)
                    } else {
                        matches!(errno, Some(libc::EPERM) | Some(libc::EACCES))
                    };
                    assert!(refused, "{operation} outside the installation: {errno:?}");
                }
                // Seatbelt mediates use of a socket, not its allocation.
                if cfg!(target_os = "linux") {
                    let socket = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
                    assert!(socket < 0, "file grants must not admit sockets");
                }
                let connected =
                    std::net::TcpStream::connect(std::env::var("PROBE_LISTENER").unwrap());
                assert!(connected.is_err(), "file grants must not admit network");
                let spawned = Command::new(&executable).arg("--spawn-probe").status();
                assert!(spawned.is_err(), "file grants must not admit exec");
            }
        }
        _ => panic!("unknown native probe"),
    }
}

fn cpu_budget() -> (libc::rlim_t, libc::rlim_t) {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_CPU, &mut limit) }, 0);
    (limit.rlim_cur, limit.rlim_max)
}

#[cfg(target_os = "linux")]
fn without_landlock() -> std::io::Result<()> {
    // Emulate a host without Landlock through a real inherited seccomp filter.
    // The worker must still establish its full policy and deny every escape.
    let instruction = |code, k, jt, jf| libc::sock_filter { code, k, jt, jf };
    let mut filter = [
        instruction(0x20, 0, 0, 0),
        instruction(0x15, libc::SYS_landlock_create_ruleset as u32, 0, 1),
        instruction(0x06, 0x00050000 | libc::ENOSYS as u32, 0, 0),
        instruction(0x06, 0x7fff0000, 0, 0),
    ];
    let program = libc::sock_fprog {
        len: filter.len() as u16,
        filter: filter.as_mut_ptr(),
    };
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0
        || unsafe { libc::syscall(libc::SYS_seccomp, 1, 0, &program) } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}
