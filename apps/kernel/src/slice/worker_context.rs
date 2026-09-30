//! Identify local slice workers and their recorded placements without relay aliases.
use super::SliceRecord;
use crate::config::DaemonConfig;

pub(crate) fn slice_worker_id(
    daemon_id: &str,
    machine_id: &str,
    local_slice_id: Option<&str>,
) -> Option<String> {
    let local_slice_id = local_slice_id
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(private_id) = machine_id
        .strip_prefix("slice:")
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return local_slice_id
            .is_none_or(|local_id| local_id == private_id)
            .then(|| private_id.to_string());
    }
    if super::machine_scoped_slice_worker_ref(daemon_id, machine_id) {
        return local_slice_id.map(str::to_string);
    }
    None
}

pub(crate) fn current_slice_worker_id() -> Option<String> {
    let machine = std::env::var("CHARIOX_MACHINE_ID")
        .or_else(|_| std::env::var("CHARIOX_SLICE_MACHINE_ID"))
        .ok()?;
    let daemon = std::env::var("CHARIOX_DAEMON_ID")
        .or_else(|_| std::env::var("CHARIOX_SLICE_DAEMON_ID"))
        .unwrap_or_default();
    let local_slice = std::env::var("CHARIOX_SLICE_ID").ok();
    slice_worker_id(&daemon, &machine, local_slice.as_deref())
}

pub(crate) fn slice_worker_id_for_config(config: &DaemonConfig) -> Option<String> {
    let local_slice = config
        .room_environment_worker_binding
        .as_ref()
        .map(|binding| binding.slice_id.clone())
        .or_else(|| std::env::var("CHARIOX_SLICE_ID").ok());
    slice_worker_id(
        &config.daemon_id,
        &config.host_machine_id,
        local_slice.as_deref(),
    )
}

pub(crate) fn recorded_slice_for_worker<'a>(
    slices: &'a [SliceRecord],
    worker_kernel_id: &str,
    worker_machine_id: &str,
) -> Option<&'a SliceRecord> {
    let mut matches = slices.iter().filter(|slice| {
        let canonical =
            super::worker_identity::qualified_worker_ref_parts(&slice.worker_kernel_ref).is_some();
        if canonical
            && !super::machine_scoped_slice_worker_ref(
                &slice.worker_kernel_ref,
                &slice.owner_machine_id,
            )
        {
            return false;
        }
        if canonical
            && slice
                .worker_kernel_id
                .as_deref()
                .is_some_and(|observed| observed != slice.worker_kernel_ref)
        {
            return false;
        }
        let expected_kernel = slice
            .worker_kernel_id
            .as_deref()
            .unwrap_or(&slice.worker_kernel_ref);
        if expected_kernel != worker_kernel_id {
            return false;
        }
        let private_machine = format!("slice:{}", slice.id);
        if let Some(recorded_machine) = slice.worker_machine_id.as_deref() {
            if recorded_machine != worker_machine_id {
                return false;
            }
        } else if worker_machine_id != slice.owner_machine_id
            && worker_machine_id != private_machine
        {
            // A custom SSH worker needs its actual Machine recorded first.
            return false;
        }
        !canonical
            || worker_machine_id == slice.owner_machine_id
            || worker_machine_id == private_machine
    });
    let found = matches.next()?;
    matches.next().is_none().then_some(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slice::hosted_worker_test_support::*;

    #[test]
    fn hosted_slice_context_identity_requires_actual_parent_and_local_id() {
        let canonical = canonical_worker(0);
        assert_eq!(
            slice_worker_id(&canonical, MACHINE, Some(SLICE)),
            Some(SLICE.into())
        );
        assert_eq!(slice_worker_id(&canonical, MACHINE, None), None);
        assert_eq!(
            slice_worker_id(&canonical, "foreign-machine", Some(SLICE)),
            None
        );
        assert_eq!(
            slice_worker_id("slice:friendly", MACHINE, Some(SLICE)),
            None
        );
        assert_eq!(slice_worker_id("ordinary", MACHINE, Some(SLICE)), None);
        assert_eq!(
            slice_worker_id("ordinary", "slice:private", None),
            Some("private".into())
        );
        assert_eq!(
            slice_worker_id("ordinary", "slice:private", Some("foreign")),
            None
        );
    }
}
