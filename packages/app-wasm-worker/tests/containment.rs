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
    let executable = std::env::current_exe().unwrap();
    let mut failures = 0;
    for scenario in ["authority", "memory", "cpu", "file", "aot"] {
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
        let engine = chariox_app_wasm_worker::engine::engine().unwrap();
        Some(
            engine
                .precompile_module(
                    &wat::parse_str(
                        "(module (func (export \"answer\") (result i32) i32.const 42))",
                    )
                    .unwrap(),
                )
                .unwrap(),
        )
    } else {
        None
    };
    let green = mode == "green";
    if green {
        chariox_app_wasm_worker::containment::enter().unwrap();
    }
    match scenario {
        "authority" => {
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
            let spawned = Command::new("/bin/true").status();
            assert_eq!(spawned.is_err(), green, "native process spawn");
            if green {
                assert_eq!(spawned.unwrap_err().raw_os_error(), Some(libc::EPERM));
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
        "file" => {
            std::io::stdout().write_all(&[b'x'; 8192]).unwrap();
        }
        "aot" => {
            let engine = chariox_app_wasm_worker::engine::engine().unwrap();
            // These bytes were precompiled by this exact process/engine above,
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
