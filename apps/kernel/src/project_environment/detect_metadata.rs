//! MP-08 / MP-10 / MP-11: representative utility metadata; full deterministic evidence stays local.
use super::ProjectEnvironmentDiscoveryInput;

pub(super) fn bound_detect_metadata(
    mut input: ProjectEnvironmentDiscoveryInput,
) -> (ProjectEnvironmentDiscoveryInput, bool) {
    let mut limited = input.references.len() > 32;
    input.references.truncate(32);
    for reference in &mut input.references {
        limited |= reference.uses.len() > 1;
        reference.uses.truncate(1);
    }
    while encoded_size(&input.references) > 12 * 1024 {
        input.references.pop();
        limited = true;
    }
    for paths in input.changed_paths.values_mut() {
        limited |= paths.len() > 8;
        paths.truncate(8);
    }
    while encoded_size(&input) > 32 * 1024 {
        let paths = input
            .changed_paths
            .values_mut()
            .max_by_key(|paths| paths.len());
        let Some(paths) = paths.filter(|paths| !paths.is_empty()) else {
            break;
        };
        paths.pop();
        limited = true;
    }
    (input, limited)
}

fn encoded_size(value: &impl serde::Serialize) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len())
}
