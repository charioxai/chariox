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
    // Offline compilation may start platform runtime threads. The contained
    // child must only read these trusted bytes before entering its boundary,
    // just as a supervisor prepares the production AOT input.
    let aot = root.join("trusted.aot");
    let engine = chariox_app_wasm_worker::engine::engine().unwrap();
    fs::write(
        &aot,
        engine
            .precompile_module(
                &wat::parse_str("(module (func (export \"answer\") (result i32) i32.const 42))")
                    .unwrap(),
            )
            .unwrap(),
    )
    .unwrap();
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
    ] {
        if matches!(scenario, "memory" | "without-landlock" | "icache") && cfg!(target_os = "macos")
        {
            println!("NATIVE {scenario}: NOT RUN (Linux-only native probe)");
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
    let (mut reader, mut writer) = UnixStream::pair().unwrap();
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
    if green {
        chariox_app_wasm_worker::containment::enter().unwrap();
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
            assert_eq!(socket < 0, green, "native socket authority");
            if green {
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
            // Linux enforces W^X. Hardened macOS permits only signed/JIT code,
            // and the separate AOT probe must prove Wasmtime compatibility.
            if cfg!(target_os = "macos") && green {
                assert_eq!(rx, -1);
            } else {
                assert_eq!(rx, 0);
            }
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
        "aot" => {
            let engine = chariox_app_wasm_worker::engine::engine().unwrap();
            // These bytes were precompiled by this exact executable/config,
            // never read from a package or an unverified native cache.
            let module =
                unsafe { wasmtime::Module::deserialize(&engine, aot.as_ref().unwrap()) }.unwrap();
            let mut store = wasmtime::Store::new(&engine, ());
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
