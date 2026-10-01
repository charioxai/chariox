#!/usr/bin/env python3
"""Compile the actual capture entrypoints with metadata-only Docker doubles.

No Docker daemon, private files, provider profiles or kernel build are used.
--state-source permits the unchanged public baseline to demonstrate the red.
"""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile

repo = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser()
parser.add_argument("--state-source", type=Path)
args = parser.parse_args()
source = (args.state_source or repo / "apps/kernel/src/slice/local_docker/state.rs").read_text()


def function(name):
    start = source.index(f"fn {name}(")
    end = source.index("\nfn ", start + 1)
    return source[start:end]


guard = repo / "apps/kernel/src/slice/local_docker/capture_preflight.rs"
header = r'''
#![allow(dead_code)]
use std::path::{Path, PathBuf};
use std::process::{Stdio, ExitStatus};
use std::os::unix::process::ExitStatusExt;
use std::sync::Mutex;
mod error {
    #[derive(Debug)]
    pub enum DaemonError { LocalTransport { operation: &'static str, message: String } }
    impl std::fmt::Display for DaemonError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            let Self::LocalTransport { operation, message } = self;
            write!(f, "{operation}: {message}")
        }
    }
}
use error::DaemonError;
static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());
mod rand { pub fn random<T: Default>() -> T { T::default() } }
struct CaptureMetadata { home_paths: Vec<&'static str>, config_env: Vec<&'static str>,
    claimed_mounts: Vec<&'static str>, immutable_base_verified: bool }
struct SliceRecord { name: String, metadata: CaptureMetadata }
struct LocalDockerSliceOptions;
fn local_docker_container_name(record: &SliceRecord) -> String { record.name.clone() }
struct Docker(Vec<String>);
fn docker_command() -> Docker { Docker(Vec::new()) }
impl Docker {
    fn args<I, S>(mut self, args: I) -> Self where I: IntoIterator<Item=S>, S: AsRef<std::ffi::OsStr> {
        self.0.extend(args.into_iter().map(|v| v.as_ref().to_string_lossy().into_owned())); self
    }
    fn stdout(self, _: Stdio) -> Self { self }
    fn stderr(self, _: Stdio) -> Self { self }
    fn status(self) -> std::io::Result<ExitStatus> {
        EVENTS.lock().unwrap().push(self.0.join(" ")); Ok(ExitStatus::from_raw(0))
    }
}
mod local_docker {
    use super::*;
    mod snapshot_pause {
        use super::*;
        pub fn create_helper(_: &SliceRecord, _: &LocalDockerSliceOptions, _: &str) -> Result<(), DaemonError> {
            EVENTS.lock().unwrap().push("create-helper".into()); Ok(())
        }
    }
    mod capture_preflight { include!("GUARD_PATH"); }
    mod state {
        use super::*;
'''.replace('mod capture_preflight { include!("GUARD_PATH"); }',
            '#[path = ' + repr(str(guard)).replace("'", '"') + '] mod capture_preflight;')
footer = r'''
        fn archive_local_docker_home_volume_with_helper(_: &str, _: &str, path: &Path, _: &str, _: &str, _: &'static str)
            -> Result<(PathBuf, u64, String), DaemonError> {
            EVENTS.lock().unwrap().push("tar-and-copy".into()); Ok((path.to_path_buf(), 1, "synthetic-digest".into()))
        }
        // Each case represents unsafe metadata; no private payload is present.
        fn refuses(metadata_category: &str, operation: &'static str) {
            EVENTS.lock().unwrap().clear();
            let metadata = CaptureMetadata {
                home_paths: if metadata_category.contains("identity") { vec![".chariox/kernels/synthetic/identity.json"] } else { vec![] },
                config_env: if metadata_category.contains("env") { vec!["CHARIOX_RELAY_TOKEN=synthetic"] } else { vec![] },
                claimed_mounts: if metadata_category.contains("mount") { vec!["/opt/chariox-slice/private"] } else { vec![] },
                immutable_base_verified: false,
            };
            let record = SliceRecord { name: metadata_category.into(), metadata };
            let error = docker_commit_container(&record, "synthetic-image", operation).unwrap_err();
            assert!(error.to_string().contains("unavailable for this capture layout"));
            assert!(!error.to_string().contains(metadata_category));
            assert!(archive_local_docker_home_volume(&record, &LocalDockerSliceOptions,
                Path::new("synthetic-home.tar.zst"), "backup", "synthetic", operation).is_err());
            assert!(EVENTS.lock().unwrap().is_empty(), "commit/helper/tar/copy must not execute");
        }
        #[test] fn mixed_home_save_is_refused() { refuses("synthetic-private-identity-filename", "slice.state.save"); }
        #[test] fn token_in_environment_backup_is_refused() { refuses("synthetic-env-token", "slice.backup.create"); }
        #[test] fn unproven_base_rollback_is_refused() { refuses("synthetic-private-layer", "slice.backup.restore"); }
        #[test] fn claimed_protected_mounts_do_not_enable_capture() { refuses("synthetic-mount-claim", "slice.state.save"); }
    }
}
'''
with tempfile.TemporaryDirectory(prefix="chariox-capture-preflight-", dir=os.environ.get("TMPDIR", "/var/tmp")) as scratch:
    root = Path(scratch)
    harness = root / "test.rs"
    harness.write_text(header + function("docker_commit_container") + "\n" +
                       function("archive_local_docker_home_volume") + footer)
    rustc = os.environ.get("CHARIOX_TEST_RUSTC", "rustc")
    subprocess.run([rustc, "--edition=2021", "--test", str(harness), "-o", str(root / "test")], check=True)
    subprocess.run([str(root / "test"), "--test-threads=1"], check=True)
