//! Text-diff helpers for workspace live sync collision reporting.
//!
//! The public surface is intentionally small: callers ask for rendered diff text while internal
//! LCS/diff operations remain local to this module.

use super::*;

pub(super) fn artifact_content_byte_count(content: &crate::io::ArtifactContent) -> usize {
    match content {
        crate::io::ArtifactContent::Text(text) => text.len(),
        crate::io::ArtifactContent::Bytes(bytes) => bytes.len(),
    }
}

pub(super) struct WorkspaceLiveSyncDiff {
    pub(super) text: String,
    pub(super) truncated: bool,
}

pub(super) const WORKSPACE_LIVE_SYNC_MAX_DIFF_BYTES: usize = 80_000;

pub(super) fn workspace_live_sync_unified_diff(
    path: &PathBuf,
    before: &WorkspaceLiveSyncTextSnapshot,
    after: &WorkspaceLiveSyncTextSnapshot,
) -> WorkspaceLiveSyncDiff {
    let normalized_path = path.to_string_lossy();
    let mut lines = Vec::new();
    lines.push(format!(
        "diff --git a/{normalized_path} b/{normalized_path}"
    ));
    if !before.existed {
        lines.push("new file mode 100644".to_string());
        lines.push("--- /dev/null".to_string());
    } else {
        if !after.existed {
            lines.push("deleted file mode 100644".to_string());
        }
        lines.push(format!("--- a/{normalized_path}"));
    }
    if after.existed {
        lines.push(format!("+++ b/{normalized_path}"));
    } else {
        lines.push("+++ /dev/null".to_string());
    }
    let before_lines = diff_lines(&before.text);
    let after_lines = diff_lines(&after.text);
    lines.extend(workspace_live_sync_diff_hunks(&before_lines, &after_lines));
    let mut text = lines.join("\n");
    let mut truncated = false;
    if text.len() > WORKSPACE_LIVE_SYNC_MAX_DIFF_BYTES {
        text.truncate(WORKSPACE_LIVE_SYNC_MAX_DIFF_BYTES);
        text.push_str("\n... diff truncated ...");
        truncated = true;
    }
    WorkspaceLiveSyncDiff { text, truncated }
}

fn diff_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split_inclusive('\n')
        .map(|line| line.strip_suffix('\n').unwrap_or(line))
        .collect()
}

#[derive(Clone, Copy)]
enum WorkspaceLiveSyncDiffOp<'a> {
    Context(&'a str),
    Remove(&'a str),
    Add(&'a str),
}

fn workspace_live_sync_diff_ops<'a>(
    before: &'a [&'a str],
    after: &'a [&'a str],
) -> Vec<WorkspaceLiveSyncDiffOp<'a>> {
    let lcs = workspace_live_sync_lcs_table(before, after);
    let mut ops = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < before.len() && j < after.len() {
        if before[i] == after[j] {
            ops.push(WorkspaceLiveSyncDiffOp::Context(before[i]));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            ops.push(WorkspaceLiveSyncDiffOp::Remove(before[i]));
            i += 1;
        } else {
            ops.push(WorkspaceLiveSyncDiffOp::Add(after[j]));
            j += 1;
        }
    }
    while i < before.len() {
        ops.push(WorkspaceLiveSyncDiffOp::Remove(before[i]));
        i += 1;
    }
    while j < after.len() {
        ops.push(WorkspaceLiveSyncDiffOp::Add(after[j]));
        j += 1;
    }
    ops
}

fn workspace_live_sync_diff_hunks(before: &[&str], after: &[&str]) -> Vec<String> {
    const CONTEXT: usize = 3;
    let ops = workspace_live_sync_diff_ops(before, after);
    if !ops
        .iter()
        .any(|op| matches!(op, WorkspaceLiveSyncDiffOp::Remove(_) | WorkspaceLiveSyncDiffOp::Add(_)))
    {
        return vec![format!("@@ -1,{} +1,{} @@", before.len(), after.len())];
    }

    let mut old_positions = Vec::with_capacity(ops.len());
    let mut new_positions = Vec::with_capacity(ops.len());
    let (mut old_line, mut new_line) = (1usize, 1usize);
    for op in &ops {
        old_positions.push(old_line);
        new_positions.push(new_line);
        match op {
            WorkspaceLiveSyncDiffOp::Context(_) => {
                old_line += 1;
                new_line += 1;
            }
            WorkspaceLiveSyncDiffOp::Remove(_) => old_line += 1,
            WorkspaceLiveSyncDiffOp::Add(_) => new_line += 1,
        }
    }

    let changed_indices = ops
        .iter()
        .enumerate()
        .filter_map(|(idx, op)| {
            matches!(op, WorkspaceLiveSyncDiffOp::Remove(_) | WorkspaceLiveSyncDiffOp::Add(_)).then_some(idx)
        })
        .collect::<Vec<_>>();
    let mut groups = Vec::new();
    for idx in changed_indices {
        let start = idx.saturating_sub(CONTEXT);
        let end = (idx + CONTEXT + 1).min(ops.len());
        if let Some((_, current_end)) = groups.last_mut() {
            if start <= *current_end {
                *current_end = (*current_end).max(end);
                continue;
            }
        }
        groups.push((start, end));
    }

    let mut lines = Vec::new();
    for (start, end) in groups {
        let hunk_ops = &ops[start..end];
        let old_start = old_positions[start];
        let new_start = new_positions[start];
        let old_count = hunk_ops
            .iter()
            .filter(|op| !matches!(op, WorkspaceLiveSyncDiffOp::Add(_)))
            .count();
        let new_count = hunk_ops
            .iter()
            .filter(|op| !matches!(op, WorkspaceLiveSyncDiffOp::Remove(_)))
            .count();
        lines.push(format!(
            "@@ -{},{} +{},{} @@",
            old_start, old_count, new_start, new_count
        ));
        lines.extend(hunk_ops.iter().map(|op| match op {
            WorkspaceLiveSyncDiffOp::Context(line) => format!(" {line}"),
            WorkspaceLiveSyncDiffOp::Remove(line) => format!("-{line}"),
            WorkspaceLiveSyncDiffOp::Add(line) => format!("+{line}"),
        }));
    }
    lines
}

fn workspace_live_sync_lcs_table(before: &[&str], after: &[&str]) -> Vec<Vec<usize>> {
    let mut table = vec![vec![0; after.len() + 1]; before.len() + 1];
    for i in (0..before.len()).rev() {
        for j in (0..after.len()).rev() {
            table[i][j] = if before[i] == after[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    table
}

pub(in crate::runtime::state) fn workspace_live_sync_text_for_diff(
    workspace_root: &PathBuf,
    path: &PathBuf,
    allow_missing: bool,
) -> Option<WorkspaceLiveSyncTextSnapshot> {
    let full_path = workspace_live_sync_diff_workspace_path(workspace_root, path)?;
    match std::fs::read_to_string(full_path) {
        Ok(text) => Some(WorkspaceLiveSyncTextSnapshot {
            existed: true,
            text,
        }),
        Err(error) if allow_missing && error.kind() == std::io::ErrorKind::NotFound => {
            Some(WorkspaceLiveSyncTextSnapshot {
                existed: false,
                text: String::new(),
            })
        }
        Err(_) => None,
    }
}

pub(super) fn workspace_live_sync_diff_workspace_path(
    workspace_root: &PathBuf,
    path: &PathBuf,
) -> Option<PathBuf> {
    if path.is_absolute() {
        return None;
    }
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => relative.push(part),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => return None,
        }
    }
    Some(workspace_root.join(relative))
}
