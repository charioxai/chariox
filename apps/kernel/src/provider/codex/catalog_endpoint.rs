use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::OnceLock;
#[cfg(test)]
use std::time::Duration;

use super::super::codex_client::{codex_endpoint_is_healthy, CODEX_ENDPOINT_STARTUP_TIMEOUT};
use super::ports::resolve_codex_catalog_port;
use crate::error::DaemonError;
use crate::provider::account_endpoints::{AccountEndpoints, ManagedAccountEndpoint};

static CODEX_ACCOUNT_ENDPOINTS: OnceLock<AccountEndpoints> = OnceLock::new();

pub fn codex_catalog_endpoint() -> Result<String, DaemonError> {
    let port = resolve_codex_catalog_port()?;
    Ok(format!("ws://127.0.0.1:{port}"))
}

pub(crate) fn ensure_codex_account_endpoint(
    owner_user_id: &str,
    account_profile: &str,
    environment: BTreeMap<String, String>,
) -> Result<String, DaemonError> {
    let key = format!("{owner_user_id}\0{account_profile}");
    let owner_user_id = owner_user_id.to_string();
    let account_profile = account_profile.to_string();
    CODEX_ACCOUNT_ENDPOINTS
        .get_or_init(|| AccountEndpoints::new("codex.catalog"))
        .ensure(
            key,
            codex_endpoint_is_healthy,
            CODEX_ENDPOINT_STARTUP_TIMEOUT,
            move || {
                let request = crate::provider::LaunchProviderRequest::new(
                    "provider-account",
                    "codex",
                    "codex",
                    &account_profile,
                    "default",
                )
                .with_owner_user_id(&owner_user_id)
                .with_provider_account_env(environment);
                let launch = crate::provider::apply_managed_provider_isolation(
                    super::plan_codex_launch(Some(&request))?,
                    &request,
                )?;
                let endpoint = launch.structured_endpoint.clone().ok_or_else(|| {
                    DaemonError::LocalTransport {
                        operation: "ensure_codex_account_endpoint",
                        message: "Codex account launch did not expose an endpoint".to_string(),
                    }
                })?;
                let mut command = crate::provider::command_from_provider_launch(launch)?;
                command
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::piped());
                for name in crate::account_profile::provider_auth_env_vars("codex") {
                    command.env_remove(name);
                }
                #[cfg(unix)]
                unsafe {
                    use std::os::unix::process::CommandExt;
                    command.pre_exec(|| {
                        if libc::setsid() < 0 {
                            Err(std::io::Error::last_os_error())
                        } else {
                            Ok(())
                        }
                    });
                }
                let child = crate::process_spawn::spawn_command(command).map_err(|error| {
                    DaemonError::LocalTransport {
                        operation: "ensure_codex_account_endpoint",
                        message: format!(
                            "failed to start profile-specific Codex app-server: {error}"
                        ),
                    }
                })?;
                Ok(ManagedAccountEndpoint::new(endpoint, child))
            },
        )
}

pub(crate) fn invalidate_codex_account_endpoint(owner_user_id: &str, account_profile: &str) {
    if let Some(endpoints) = CODEX_ACCOUNT_ENDPOINTS.get() {
        endpoints.invalidate(format!("{owner_user_id}\0{account_profile}"));
    }
}

pub(crate) fn shutdown_codex_account_endpoints() {
    if let Some(endpoints) = CODEX_ACCOUNT_ENDPOINTS.get() {
        endpoints.shutdown();
    }
}
#[cfg(all(test, target_os = "linux"))]
mod stall_tests {
    use super::*;
    use std::io::BufRead;

    #[test]
    fn invalidate_reaps_unresponsive_child_when_process_snapshot_is_unavailable() {
        crate::test_support::isolated_env_test!();
        let store = crate::provider::ProviderProcessServiceStore::new(
            crate::provider::ProviderProcessService::new(),
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while crate::runtime::process_resources::snapshot_row_count_for_test() == 0 {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(20));
        }
        for ignore_term in [false, true] {
            let descendants = crate::test_support::TestWorktree::new("account-child");
            let child_pid_file = descendants.path().join("child.pid");
            let grandchild = format!("import os,signal,time,pathlib; signal.signal(signal.SIGTERM,signal.SIG_IGN); pathlib.Path({:?}).write_text(str(os.getpid())); time.sleep(600)", child_pid_file.to_string_lossy());
            let script = format!("import signal,time,subprocess,sys,pathlib; {}; subprocess.Popen([sys.executable,'-c',{:?}]); p=pathlib.Path({:?});\nwhile not p.exists(): time.sleep(0.01)\nprint('ready',flush=True); time.sleep(600)",
                if ignore_term { "signal.signal(signal.SIGTERM,signal.SIG_IGN)" } else { "pass" }, grandchild, child_pid_file.to_string_lossy());
            let mut child = crate::process_spawn::command("python3")
                .args(["-c", &script])
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let mut ready = String::new();
            std::io::BufReader::new(child.stdout.take().unwrap())
                .read_line(&mut ready)
                .unwrap();
            assert_eq!(ready.trim(), "ready");
            let pid = child.id();
            let descendant_pid: u32 = std::fs::read_to_string(&child_pid_file)
                .unwrap()
                .parse()
                .unwrap();
            let mut cleanup =
                crate::runtime::owned_process_signals::OwnedProcessSignals::for_child(&child)
                    .unwrap();
            let key = "stall-owner\0stall-profile".to_string();
            let endpoints =
                CODEX_ACCOUNT_ENDPOINTS.get_or_init(|| AccountEndpoints::new("codex.catalog"));
            endpoints.install(key, ManagedAccountEndpoint::new("unused".into(), child));
            let (start, started) = std::sync::mpsc::channel();
            let (done, finished) = std::sync::mpsc::channel();
            let worker = std::thread::spawn(move || {
                started.recv().unwrap();
                invalidate_codex_account_endpoint("stall-owner", "stall-profile");
                done.send(()).unwrap();
            });
            let mut original = std::mem::MaybeUninit::<libc::rlimit>::uninit();
            assert_eq!(
                unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, original.as_mut_ptr()) },
                0
            );
            let original = unsafe { original.assume_init() };
            let limited = libc::rlimit {
                rlim_cur: 0,
                rlim_max: original.rlim_max,
            };
            assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limited) }, 0);
            start.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(100));
            let progress_started = std::time::Instant::now();
            assert!(store
                .get_runs_by_runtime_mcp_auth_token("unrelated-fixture")
                .is_empty());
            let request = crate::provider::LaunchProviderRequest::new(
                "stall-progress",
                "dev-stub",
                "stub",
                "default",
                "test",
            );
            let launched = store.start_run_provider_only(request).is_ok();
            let sampled = crate::runtime::process_resources::snapshot_row_count_for_test() > 0;
            let other_account_can_progress =
                launched && sampled && progress_started.elapsed() < Duration::from_millis(200);
            let returned = finished.recv_timeout(Duration::from_secs(2)).is_ok();
            assert_eq!(
                unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &original) },
                0
            );
            // A failing base run must still release the exact helper it created.
            if !returned {
                let _ = cleanup.kill_owned_processes();
            }
            if !returned && pid > 1 && pid <= i32::MAX as u32 {
                unsafe {
                    libc::kill(pid as i32, libc::SIGKILL);
                }
            }
            worker.join().unwrap();
            assert!(
                !crate::runtime::process_health::process_running(pid),
                "helper must be reaped"
            );
            let exit_deadline = std::time::Instant::now() + Duration::from_secs(1);
            while crate::runtime::process_health::process_running(descendant_pid)
                && std::time::Instant::now() < exit_deadline
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(
                !crate::runtime::process_health::process_running(descendant_pid),
                "native helper descendant survived invalidation"
            );
            assert!(
                other_account_can_progress,
                "launch, MCP lookup and sampler snapshot must progress during child shutdown"
            );
            assert!(
                returned,
                "invalidation exceeded its bound when the process snapshot failed"
            );
        }
    }
}
