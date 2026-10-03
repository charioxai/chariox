use super::*;
use std::io::{Read, Write};
use std::os::unix::fs::{symlink, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use wait_timeout::ChildExt;

struct Fixture {
    root: PathBuf,
    previous: Vec<(&'static str, Option<std::ffi::OsString>)>,
}
impl Fixture {
    fn new(mode: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-ha-{}-{}",
            std::process::id(),
            rand::random::<u32>()
        ));
        std::fs::create_dir(&root).unwrap();
        let bin = root.join("bin");
        std::fs::create_dir(&bin).unwrap();
        let docker = bin.join("docker");
        std::fs::write(
            &docker,
            r#"#!/bin/sh
printf '%s\n' "$*" >> "$FAKE_ARCHIVE_LOG"
case "$1" in
  start) test "$FAKE_ARCHIVE_MODE" != "spawnfailed" || rm "$0" ;;
  exec)
    case "$7" in
      *"/tmp/home.tar.zst"*) printf 'layer archive' > "$FAKE_ARCHIVE_LAYER" ;;
      *"tar --zstd -cf - ."*) test -f /dev/fd/1 || test -p /dev/fd/1 || exit 23 ;;
      -C) test "$5" = tar && test "$6" = --zstd && test "$8" = /home-src && test "$9" = -cf && test "${10}" = - ;;
      *) exit 25 ;;
    esac
    case "$FAKE_ARCHIVE_MODE" in
      failed) printf 'partial synthetic home'; exit 17 ;;
      empty) exit 0 ;;
      large) head -c 2097152 /dev/zero ;;
      stderr) head -c 4194304 /dev/zero >&2; printf 'synthetic home' ;;
      *) printf 'synthetic home' ;;
    esac
    ;;
  cp) for arg in "$@"; do destination=$arg; done
      printf 'synthetic home' > "$destination"; chmod 644 "$destination" ;;
esac
exit 0
"#,
        )
        .unwrap();
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o700)).unwrap();
        let names = [
            "PATH",
            "FAKE_ARCHIVE_MODE",
            "FAKE_ARCHIVE_LOG",
            "FAKE_ARCHIVE_LAYER",
        ];
        let previous = names
            .into_iter()
            .map(|n| (n, std::env::var_os(n)))
            .collect();
        let mut paths = vec![bin];
        if let Some(path) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&path));
        }
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
        std::env::set_var("FAKE_ARCHIVE_MODE", mode);
        std::env::set_var("FAKE_ARCHIVE_LOG", root.join("docker.log"));
        std::env::set_var("FAKE_ARCHIVE_LAYER", root.join("layer.tar"));
        Self { root, previous }
    }
    fn capture(&self) -> Result<(PathBuf, u64, String), DaemonError> {
        archive_local_docker_home_volume_with_helper(
            "chariox-slice-test-home-archive-1",
            "chariox-slice-test-home",
            &self.root.join("home.tar.zst"),
            "state",
            "test",
            "slice.test.archive",
        )
    }
    fn calls(&self) -> String {
        std::fs::read_to_string(self.root.join("docker.log")).unwrap()
    }
    fn no_layer_archive(&self) {
        assert!(
            !self.root.join("layer.tar").exists(),
            "home archive must not enter helper writable layer"
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for (name, value) in &self.previous {
            match value {
                Some(v) => std::env::set_var(name, v),
                None => std::env::remove_var(name),
            }
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
#[test]
fn home_archive_stream_is_private_direct_output_and_digest_bound() {
    let _lock = crate::env_lock::lock();
    let f = Fixture::new("success");
    let (path, size, hash) = f.capture().unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(size, 14);
    assert_eq!(std::fs::read(&path).unwrap(), b"synthetic home");
    assert_eq!((size, hash), file_sha256(&path, "test").unwrap());
    f.no_layer_archive();
    let calls = f.calls();
    assert!(calls.lines().next().unwrap().starts_with("start "));
    assert!(!calls.lines().any(|s| s.starts_with("cp ")));
}
#[test]
fn home_archive_stream_refuses_existing_generation() {
    let _lock = crate::env_lock::lock();
    let f = Fixture::new("success");
    let path = f.root.join("home.tar.zst");
    std::fs::write(&path, b"prior generation").unwrap();
    assert!(f.capture().is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"prior generation");
    f.no_layer_archive();
}
#[test]
fn home_archive_stream_refuses_symlink_without_touching_target() {
    let _lock = crate::env_lock::lock();
    let f = Fixture::new("success");
    let target = f.root.join("prior.tar");
    std::fs::write(&target, b"prior generation").unwrap();
    symlink(&target, f.root.join("home.tar.zst")).unwrap();
    assert!(f.capture().is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"prior generation");
    f.no_layer_archive();
}
#[test]
fn home_archive_stream_failed_tar_removes_partial_and_preserves_prior() {
    let _lock = crate::env_lock::lock();
    let f = Fixture::new("failed");
    std::fs::write(f.root.join("prior.tar"), b"prior generation").unwrap();
    assert!(f.capture().is_err());
    assert!(!f.root.join("home.tar.zst").exists());
    assert_eq!(
        std::fs::read(f.root.join("prior.tar")).unwrap(),
        b"prior generation"
    );
    f.no_layer_archive();
}
#[test]
fn home_archive_stream_empty_tar_is_rejected_and_removed() {
    let _lock = crate::env_lock::lock();
    let f = Fixture::new("empty");
    assert!(f.capture().is_err());
    assert!(!f.root.join("home.tar.zst").exists());
    f.no_layer_archive();
}
#[test]
fn home_archive_stream_spawn_failure_removes_created_file() {
    let _lock = crate::env_lock::lock();
    let f = Fixture::new("spawnfailed");
    assert!(f.capture().is_err());
    assert!(!f.root.join("home.tar.zst").exists());
    f.no_layer_archive();
}
#[test]
fn home_archive_stream_large_output_is_file_backed() {
    let _lock = crate::env_lock::lock();
    let f = Fixture::new("large");
    let (path, size, hash) = f.capture().unwrap();
    assert_eq!(size, 2 * 1024 * 1024);
    assert_eq!((size, hash), file_sha256(&path, "test").unwrap());
    f.no_layer_archive();
}
#[test]
fn home_archive_stream_stderr_volume_does_not_change_capture() {
    let _lock = crate::env_lock::lock();
    let f = Fixture::new("stderr");
    let (path, size, _) = f.capture().unwrap();
    assert_eq!(size, 14);
    assert_eq!(std::fs::read(&path).unwrap(), b"synthetic home");
    f.no_layer_archive();
}
#[test]
fn home_archive_stream_outer_failure_removes_owned_helper() {
    let _lock = crate::env_lock::lock();
    let f = Fixture::new("failed");
    let record = super::super::tests::test_record();
    let mut options = super::super::tests::test_options();
    options.root = f.root.clone();
    super::super::snapshot_pause::begin(&record, &options, false).unwrap();
    assert!(
        super::super::capture_preflight::with_test_verified_layout(|| {
            archive_local_docker_home_volume(
                &record,
                &options,
                &f.root.join("home.tar.zst"),
                "state",
                "test",
                "test",
            )
        })
        .is_err()
    );
    assert!(f
        .calls()
        .lines()
        .any(|s| s.starts_with("rm -f chariox-slice-dev-home-archive-")));
    f.no_layer_archive();
}
#[test]
fn home_archive_stream_managed_delegates_before_any_tar() {
    use base64::Engine;
    let root = std::env::temp_dir().join(format!(
        "chariox-hb-{}-{}",
        std::process::id(),
        rand::random::<u32>()
    ));
    std::fs::create_dir(&root).unwrap();
    let socket = root.join("broker.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut stream = loop {
            match listener.accept() {
                Ok((s, _)) => break s,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(e) => panic!("broker fixture failed to accept: {e}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut requests = Vec::new();
        for _ in 0..2 {
            let mut header = [0; 4];
            stream.read_exact(&mut header).unwrap();
            let len = u32::from_be_bytes(header) as usize;
            assert!(len < 4096);
            let mut bytes = vec![0; len];
            stream.read_exact(&mut bytes).unwrap();
            let request: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            let capture = request["kind"] == "home_archive_capture" && request["scope"] == "state";
            let start = request["kind"] == "docker" && request["args"][0] == "start";
            let stdout = if capture {
                serde_json::to_vec(&serde_json::json!({"path":"/fixture/private/home.tar.zst","sizeBytes":14,"sha256":"a".repeat(64)})).unwrap()
            } else {
                Vec::new()
            };
            let response = serde_json::to_vec(&serde_json::json!({
                "status": if start || capture {0} else {17},
                "stdoutBase64":base64::engine::general_purpose::STANDARD.encode(stdout),
                "stderrBase64":""
            }))
            .unwrap();
            stream
                .write_all(&(response.len() as u32).to_be_bytes())
                .unwrap();
            stream.write_all(&response).unwrap();
            requests.push(request);
        }
        requests
    });
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "managed_archive_stream_child"])
        .env("CHARIOX_SLICE_DOCKER_BROKER_SOCKET", &socket)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let status = child.wait_timeout(Duration::from_secs(15)).unwrap();
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let requests = server.join().unwrap();
    let _ = std::fs::remove_dir_all(root);
    assert!(
        status.is_some_and(|s| s.success()),
        "managed child must capture successfully"
    );
    assert_eq!(
        requests[0]["args"],
        serde_json::json!(["start", "chariox-slice-test-home-archive-1"])
    );
    assert_eq!(requests[1]["kind"], "home_archive_capture");
    assert_eq!(
        requests[1]["container"],
        "chariox-slice-test-home-archive-1"
    );
    assert_eq!(requests[1]["scope"], "state");
    assert_eq!(requests[1]["id"], "test");
}
#[test]
#[ignore = "isolated broker subprocess fixture"]
fn managed_archive_stream_child() {
    super::super::broker::initialize();
    let captured = archive_local_docker_home_volume_with_helper(
        "chariox-slice-test-home-archive-1",
        "chariox-slice-test-home",
        Path::new("/must-not-create/home.tar.zst"),
        "state",
        "test",
        "test",
    )
    .unwrap();
    assert_eq!(
        captured,
        (
            PathBuf::from("/fixture/private/home.tar.zst"),
            14,
            "a".repeat(64)
        )
    );
}

#[test]
fn home_archive_stream_uses_direct_tar_without_login_shell() {
    let _lock = crate::env_lock::lock();
    let f = Fixture::new("success");
    f.capture().unwrap();
    assert!(f.calls().lines().any(|line| line
        == "exec -u root chariox-slice-test-home-archive-1 tar --zstd -C /home-src -cf - ."));
}

#[test]
fn home_archive_stream_shared_reserve_refuses_before_file_or_producer() {
    let _lock = crate::env_lock::lock();
    let f = Fixture::new("success");
    assert_eq!(
        super::super::home_archive_capture::minimum_free_bytes(),
        2 * 1024 * 1024 * 1024
    );
    let result = super::super::home_archive_capture::capture_with_available_space(
        "chariox-slice-test-home-archive-1",
        &f.root.join("home.tar.zst"),
        "test",
        || Ok(0),
    );
    assert!(result.is_err());
    assert!(!f.root.join("home.tar.zst").exists());
    assert!(!f.root.join("docker.log").exists());
}

#[test]
fn home_archive_stream_pressure_removes_partial_but_retains_previous_generation() {
    let _lock = crate::env_lock::lock();
    let f = Fixture::new("success");
    let previous = f.root.join("prior.tar");
    std::fs::write(&previous, b"prior generation").unwrap();
    let reserve = super::super::home_archive_capture::minimum_free_bytes();
    let mut calls = 0;
    let result = super::super::home_archive_capture::capture_with_available_space(
        "chariox-slice-test-home-archive-1",
        &f.root.join("home.tar.zst"),
        "test",
        || {
            calls += 1;
            Ok(if calls < 3 {
                reserve + 1024
            } else {
                reserve - 1
            })
        },
    );
    assert!(result.is_err());
    assert!(!f.root.join("home.tar.zst").exists());
    assert_eq!(std::fs::read(previous).unwrap(), b"prior generation");
    assert!(calls >= 3);
}
