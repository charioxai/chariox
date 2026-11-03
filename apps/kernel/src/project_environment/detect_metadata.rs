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

// Detect keeps a separate prompt path from export discovery.
pub fn project_environment_detect_prompt(
    input: &ProjectEnvironmentDiscoveryInput,
) -> Result<String, crate::error::DaemonError> {
    let (bounded, _) = bound_detect_metadata(input.clone());
    let encoded = serde_json::to_string(&bounded)
        .map_err(|_| super::environment_error("Detect metadata encoding failed"))?;
    if encoded.len() > 32 * 1024 {
        return Err(super::environment_error("Detect metadata exceeds bounds"));
    }
    Ok(format!(
        "MP-08 Project environment Detect. Classify only this bounded filename inventory and reference metadata. Folder identities are the keys in changed_paths. Tools are disabled; no file contents or values are supplied. Return only a JSON Project environment manifest with schema_version 1, the exact project_id and evidence_digest, entries copied from references (classification may change), private_files [], and toolchain_hints/package_hints/service_hints. Hints are review proposals, never readiness or authority to install anything. Never invent reference names, use sites or locators.\nMetadata:\n{encoded}"
    ))
}
