//! Workspace live sync patch operation validation and final-state planning.

use super::*;

pub(in crate::runtime::state) enum ManagedPatchPlanOutcome {
    Planned(ManagedPatchPlan),
    Rejected(crate::transport::runtime_tools::RuntimeToolResult),
}

pub(in crate::runtime::state) struct ManagedPatchPlan {
    pub(in crate::runtime::state) before_states: BTreeMap<PathBuf, Option<String>>,
    pub(in crate::runtime::state) final_states: BTreeMap<PathBuf, Option<String>>,
    pub(in crate::runtime::state) reservation_ranges: BTreeMap<PathBuf, Vec<crate::io::TextRange>>,
}

pub(in crate::runtime::state) fn plan_managed_patch_operations(
    workspace_root: &PathBuf,
    operations: Vec<ManagedPatchOperation>,
) -> Result<ManagedPatchPlanOutcome, DaemonError> {
    let mut before_states = BTreeMap::new();
    let mut final_states = BTreeMap::new();
    let mut reservation_ranges = BTreeMap::new();

    for operation in operations {
        match operation {
            ManagedPatchOperation::Add { path, content } => {
                workspace_live_sync_validate_patch_path(workspace_root, &path)?;
                let current = managed_patch_state(
                    workspace_root,
                    &path,
                    &mut before_states,
                    &mut final_states,
                )?;
                if current.is_some() {
                    return Ok(ManagedPatchPlanOutcome::Rejected(managed_patch_rejected(
                        path,
                        "add file target already exists; reread and retry with an update",
                    )));
                }
                reserve_full_artifact(&mut reservation_ranges, &path);
                final_states.insert(path, Some(content));
            }
            ManagedPatchOperation::Update {
                path,
                old_text,
                new_text,
            } => {
                workspace_live_sync_validate_patch_path(workspace_root, &path)?;
                let current = managed_patch_state(
                    workspace_root,
                    &path,
                    &mut before_states,
                    &mut final_states,
                )?;
                let Some(current) = current else {
                    return Ok(ManagedPatchPlanOutcome::Rejected(managed_patch_rejected(
                        path,
                        "update file target does not exist",
                    )));
                };
                let Some((range, updated)) = replace_unique_text(&current, &old_text, &new_text)
                else {
                    return Ok(ManagedPatchPlanOutcome::Rejected(managed_patch_rejected(
                        path,
                        "patch old text was not found exactly once in the current artifact",
                    )));
                };
                reservation_ranges
                    .entry(path.clone())
                    .or_default()
                    .push(range);
                final_states.insert(path, Some(updated));
            }
            ManagedPatchOperation::Delete { path } => {
                workspace_live_sync_validate_patch_path(workspace_root, &path)?;
                let current = managed_patch_state(
                    workspace_root,
                    &path,
                    &mut before_states,
                    &mut final_states,
                )?;
                if current.is_none() {
                    return Ok(ManagedPatchPlanOutcome::Rejected(managed_patch_rejected(
                        path,
                        "delete file target does not exist",
                    )));
                }
                reserve_full_artifact(&mut reservation_ranges, &path);
                final_states.insert(path, None);
            }
            ManagedPatchOperation::Move {
                from_path,
                to_path,
                old_text,
                new_text,
            } => {
                workspace_live_sync_validate_patch_path(workspace_root, &from_path)?;
                workspace_live_sync_validate_patch_path(workspace_root, &to_path)?;
                if from_path == to_path {
                    return Ok(ManagedPatchPlanOutcome::Rejected(managed_patch_rejected(
                        from_path,
                        "move source and target are identical",
                    )));
                }
                let source = managed_patch_state(
                    workspace_root,
                    &from_path,
                    &mut before_states,
                    &mut final_states,
                )?;
                let Some(mut source) = source else {
                    return Ok(ManagedPatchPlanOutcome::Rejected(managed_patch_rejected(
                        from_path,
                        "move source does not exist",
                    )));
                };
                let target = managed_patch_state(
                    workspace_root,
                    &to_path,
                    &mut before_states,
                    &mut final_states,
                )?;
                if target.is_some() {
                    return Ok(ManagedPatchPlanOutcome::Rejected(managed_patch_rejected(
                        to_path,
                        "move target already exists",
                    )));
                }
                if let (Some(old_text), Some(new_text)) = (old_text, new_text) {
                    let Some((_range, updated)) =
                        replace_unique_text(&source, &old_text, &new_text)
                    else {
                        return Ok(ManagedPatchPlanOutcome::Rejected(managed_patch_rejected(
                            from_path,
                            "move patch old text was not found exactly once in the current artifact",
                        )));
                    };
                    source = updated;
                }
                reserve_full_artifact(&mut reservation_ranges, &from_path);
                reserve_full_artifact(&mut reservation_ranges, &to_path);
                final_states.insert(from_path, None);
                final_states.insert(to_path, Some(source));
            }
        }
    }

    Ok(ManagedPatchPlanOutcome::Planned(ManagedPatchPlan {
        before_states,
        final_states,
        reservation_ranges,
    }))
}

fn reserve_full_artifact(
    reservation_ranges: &mut BTreeMap<PathBuf, Vec<crate::io::TextRange>>,
    path: &PathBuf,
) {
    reservation_ranges
        .entry(path.clone())
        .or_default()
        .push(crate::io::TextRange::new(0, usize::MAX));
}
