//! Bounded saved-slice image depth.
//!
//! Docker's layer store refuses images deeper than 125 layers (overlay2), and
//! each capture of a slice that already runs on a saved image (a save or backup
//! after a restore or clone) commits one more layer. A capture whose parent is
//! already `MAX_CAPTURED_IMAGE_LAYERS` deep flattens the stopped or paused
//! container into a one-layer image with the same configuration instead
//! (`slice-linux-docker/captured-image-depth.mjs`). The managed and local DEV
//! brokers apply the same rule to the commits they run; the kernel flattens
//! only on a direct Docker engine and predicts the flatten for disk admission.

use crate::error::DaemonError;
use crate::runtime::bounded_process::BoundedCommandExt as _;

use super::broker::{self, docker_command};

/// Keep in sync with `MAX_CAPTURED_IMAGE_LAYERS` in captured-image-depth.mjs.
pub(super) const MAX_CAPTURED_IMAGE_LAYERS: usize = 100;
const FLATTEN_SCRIPT: &str = "captured-image-depth.mjs";
// Match the bulk commit budget in captured-image-depth.mjs. Capturing a full
// root filesystem is active work, not a short utility call or a child reap.
const CAPTURE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(600);

#[cfg(test)]
thread_local! {
    static TEST_CAPTURE_DEADLINE: std::cell::Cell<Option<std::time::Duration>> = const { std::cell::Cell::new(None) };
}

fn capture_deadline() -> std::time::Duration {
    #[cfg(test)]
    if let Some(limit) = TEST_CAPTURE_DEADLINE.with(std::cell::Cell::get) {
        return limit;
    }
    CAPTURE_DEADLINE
}

pub(super) fn capture_needs_flatten(parent_layers: usize) -> bool {
    parent_layers.saturating_add(1) > MAX_CAPTURED_IMAGE_LAYERS
}

/// Depth of the image the slice container runs on, when Docker reports it.
pub(super) fn container_parent_layers(container: &str) -> Option<usize> {
    let image = docker_stdout(&["inspect", "--format", "{{.Image}}", container])?;
    if !image.starts_with("sha256:") {
        return None;
    }
    docker_stdout(&[
        "image",
        "inspect",
        "--format",
        "{{len .RootFS.Layers}}",
        &image,
    ])?
    .parse()
    .ok()
}

/// Whether capturing this container flattens it. Unknown depth keeps the
/// ordinary commit; a broker still decides from its own inspection.
pub(super) fn capture_will_flatten(container: &str) -> bool {
    container_parent_layers(container).is_some_and(capture_needs_flatten)
}

fn docker_stdout(args: &[&str]) -> Option<String> {
    let output = docker_command().args(args).bounded_output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Commits `container` to `image_ref`, flattening it first when its parent is
/// too deep and this kernel drives Docker directly.
pub(super) fn commit_container_bounded(
    container: &str,
    image_ref: &str,
    operation: &'static str,
) -> Result<(), DaemonError> {
    if !broker::configured() && capture_will_flatten(container) {
        return flatten_container_into_image(container, image_ref, operation);
    }
    let output = docker_command()
        .args(["commit", container, image_ref])
        .output_with_deadline(capture_deadline())
        .map_err(|error| DaemonError::LocalTransport {
            operation,
            message: format!("failed to commit slice container `{container}`: {error}"),
        })?;
    let status = output.status;
    if status.success() {
        Ok(())
    } else {
        Err(DaemonError::LocalTransport {
            operation,
            message: format!(
                "docker commit `{container}` to `{image_ref}` failed with status {status}"
            ),
        })
    }
}

fn flatten_container_into_image(
    container: &str,
    image_ref: &str,
    operation: &'static str,
) -> Result<(), DaemonError> {
    let provisioner = super::linux_docker_slice_script()?;
    let script = provisioner
        .parent()
        .map(|directory| directory.join(FLATTEN_SCRIPT))
        .filter(|script| script.is_file())
        .ok_or_else(|| DaemonError::LocalTransport {
            operation,
            message: format!(
                "slice image flattening helper {FLATTEN_SCRIPT} is missing beside {}",
                provisioner.display()
            ),
        })?;
    let mut command = crate::process_spawn::command("node");
    command.arg(&script).args(["flatten", container, image_ref]);
    // MP-08/MP-11: the same explicit engine as the ordinary commit.
    if std::env::var_os("DOCKER_HOST").is_some_and(|host| !host.is_empty()) {
        command.env_remove("DOCKER_CONTEXT");
    }
    // Capture owns the helper's output pipes and inherited writers, including
    // Docker export/import. A deadline or I/O error settles that pipeline too.
    let output = command
        .output_with_deadline(capture_deadline())
        .map_err(|error| DaemonError::LocalTransport {
            operation,
            message: format!("failed to flatten slice container `{container}`: {error}"),
        })?;
    let status = output.status;
    if status.success() {
        Ok(())
    } else {
        Err(DaemonError::LocalTransport {
            operation,
            message: format!(
                "flattening slice container `{container}` to `{image_ref}` failed with status {status}"
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn slow_capture(flatten: bool) {
        crate::test_support::isolated_env_test!();
        use std::os::unix::fs::PermissionsExt;
        let root = crate::test_support::TestWorktree::new("direct-capture-lifetime");
        let bin = root.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        for name in ["docker", "node"] {
            let script = format!(
                "#!/usr/bin/env python3\nimport sys,time\nif sys.argv[1] == 'inspect': print('sha256:' + '7'*64)\nelif sys.argv[1:3] == ['image', 'inspect']: print({})\nelse: time.sleep(61)\n",
                if flatten { 100 } else { 99 }
            );
            let path = bin.join(name);
            std::fs::write(&path, script).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mut paths = vec![bin];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
        std::env::remove_var("CHARIOX_SLICE_DOCKER_BROKER_REQUIRED");
        let started = std::time::Instant::now();
        commit_container_bounded(
            "chariox-slice-dev",
            "chariox-slice-state:lifetime",
            "slice.state.save",
        )
        .expect("bulk snapshot capture must survive the generic command deadline");
        assert!(started.elapsed() >= std::time::Duration::from_secs(61));
    }

    #[cfg(unix)]
    #[test]
    fn direct_snapshot_commit_survives_the_generic_command_deadline() {
        slow_capture(false);
    }

    #[cfg(unix)]
    #[test]
    fn direct_snapshot_flatten_survives_the_generic_command_deadline() {
        slow_capture(true);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn direct_snapshot_flatten_deadline_settles_export_and_import() {
        crate::test_support::isolated_env_test!();
        use std::os::unix::fs::PermissionsExt;
        let root = crate::test_support::TestWorktree::new("direct-flatten-cancel");
        let bin = root.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let docker = bin.join("docker");
        std::fs::write(
            &docker,
            r#"#!/usr/bin/env python3
import json,os,pathlib,signal,sys,time
args=sys.argv[1:]
if args[0] == 'inspect': print('sha256:' + '7'*64)
elif args[:2] == ['image','inspect']: print(100)
elif args[:2] == ['container','inspect']:
    print(json.dumps([{'Id':'fixture','State':{'Running':False},'Config':{}}]))
else:
    signal.signal(signal.SIGTERM,signal.SIG_IGN)
    pathlib.Path(os.environ['CAPTURE_TEST_ROOT'], args[0] + '.pid').write_text(str(os.getpid()))
    time.sleep(120)
"#,
        )
        .unwrap();
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut paths = vec![bin];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
        std::env::set_var("CAPTURE_TEST_ROOT", root.path());
        TEST_CAPTURE_DEADLINE.with(|limit| limit.set(Some(std::time::Duration::from_secs(2))));
        let mut witness = crate::process_spawn::command("/bin/sleep")
            .arg("120")
            .spawn()
            .unwrap();
        let result = commit_container_bounded(
            "chariox-slice-dev",
            "chariox-slice-state:cancel",
            "slice.state.save",
        );
        let witness_alive = witness.try_wait().unwrap().is_none();
        crate::runtime::child_shutdown::stop(&mut witness, "capture_test_witness").unwrap();
        assert!(result.is_err(), "the capture must report its deadline");
        for side in ["export", "import"] {
            let pid: u32 = std::fs::read_to_string(root.path().join(format!("{side}.pid")))
                .expect("the real flatten helper must start both pipeline children")
                .parse()
                .unwrap();
            assert!(
                !crate::runtime::process_health::process_running(pid),
                "{side} survived capture cancellation"
            );
        }
        assert!(
            witness_alive,
            "cancellation must stop only this capture's processes"
        );
    }

    #[test]
    fn captures_flatten_only_once_the_parent_reaches_the_bound() {
        assert!(!capture_needs_flatten(1));
        assert!(!capture_needs_flatten(MAX_CAPTURED_IMAGE_LAYERS - 1));
        assert!(capture_needs_flatten(MAX_CAPTURED_IMAGE_LAYERS));
        assert!(capture_needs_flatten(124));
        assert!(capture_needs_flatten(usize::MAX));
        // Docker's overlay2 store refuses images deeper than 125 layers.
        const _: () = assert!(MAX_CAPTURED_IMAGE_LAYERS < 125);
    }
}
