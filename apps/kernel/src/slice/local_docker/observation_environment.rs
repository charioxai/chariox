//! MP-08/MP-10/MP-11: prove a fresh desktop before clearing a missing-value fence.
use super::*;

pub(crate) fn fresh_local_docker_observation_environment(
    slice: &SliceRecord,
    options: &LocalDockerSliceOptions,
) -> bool {
    if slice.source_slice_ref.is_some()
        || slice.saved_state_ref.is_some()
        || options.saved_home_archive.is_some()
    {
        return false;
    }
    let container = local_docker_container_name(slice);
    let home = format!("{container}-home");
    // Successful inventories establish absence. Docker errors never establish it.
    let containers = docker_command()
        .args(["ps", "-a", "--format", "{{.Names}}"])
        .output()
        .ok()
        .filter(|output| output.status.success());
    let volumes = docker_command()
        .args(["volume", "ls", "--format", "{{.Name}}"])
        .output()
        .ok()
        .filter(|output| output.status.success());
    absent(
        containers.as_ref().map(|output| output.stdout.as_slice()),
        &container,
    ) && absent(
        volumes.as_ref().map(|output| output.stdout.as_slice()),
        &home,
    )
}

fn absent(inventory: Option<&[u8]>, name: &str) -> bool {
    inventory
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .is_some_and(|names| !names.lines().any(|candidate| candidate.trim() == name))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fresh_environment_requires_successful_absence_of_container_and_home() {
        assert!(!absent(None, "owned-home"));
        assert!(!absent(Some(b"owned-home\n"), "owned-home"));
        assert!(!absent(Some(&[255]), "owned-home"));
        assert!(absent(Some(b"unrelated-home\n"), "owned-home"));
        assert!(absent(Some(b""), "owned-home"));
    }
}
