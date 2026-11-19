//! A workflow node's instruction file. Its agent rereads it after losing
//! context, so it lives in the kernel home beside, not inside, the kernel's
//! hidden state.
use crate::config::DaemonConfig;

/// Writes the node's instructions (or a placeholder the first time) and
/// returns the file's path, or `None` when it cannot be written.
pub(crate) fn node_instruction_file(
    config: &DaemonConfig,
    session_id: &str,
    workflow_run_id: &str,
    node_id: &str,
    instructions: Option<&str>,
) -> Option<String> {
    let root = config
        .workflow_instruction_root()
        .join(session_id)
        .join(workflow_run_id);
    let path = root.join(format!("node-{node_id}.md"));
    if !path.exists() || instructions.is_some() {
        if let Err(error) = std::fs::create_dir_all(&root) {
            tracing::debug!(
                ?error,
                "Failed to create workflow instruction directory at {:?}",
                root
            );
            return None;
        }
        let content = instructions.map(str::to_string).unwrap_or_else(|| {
            format!(
                "# Workflow Node Instructions\n\nThis file is daemon-managed. Update node instructions through workflow configuration tooling.\n\nNode: {node_id}\n"
            )
        });
        if let Err(error) = std::fs::write(&path, content) {
            tracing::debug!(
                ?error,
                "Failed to write workflow instruction file at {:?}",
                path
            );
            return None;
        }
    }
    Some(path.to_string_lossy().to_string())
}
