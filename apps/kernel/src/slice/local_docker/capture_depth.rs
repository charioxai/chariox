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

use std::process::Command;

use crate::error::DaemonError;

use super::broker::{self, docker_command};

/// Keep in sync with `MAX_CAPTURED_IMAGE_LAYERS` in captured-image-depth.mjs.
pub(super) const MAX_CAPTURED_IMAGE_LAYERS: usize = 100;
const FLATTEN_SCRIPT: &str = "captured-image-depth.mjs";

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
    let output = docker_command().args(args).output().ok()?;
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
    let status = docker_command()
        .args(["commit", container, image_ref])
        .status()
        .map_err(|error| DaemonError::LocalTransport {
            operation,
            message: format!("failed to commit slice container `{container}`: {error}"),
        })?;
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
    let mut command = Command::new("node");
    command.arg(&script).args(["flatten", container, image_ref]);
    // MP-08/MP-11: the same explicit engine as the ordinary commit.
    if std::env::var_os("DOCKER_HOST").is_some_and(|host| !host.is_empty()) {
        command.env_remove("DOCKER_CONTEXT");
    }
    let status = command
        .status()
        .map_err(|error| DaemonError::LocalTransport {
            operation,
            message: format!("failed to flatten slice container `{container}`: {error}"),
        })?;
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
