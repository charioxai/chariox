use std::path::PathBuf;

pub(super) struct WorkspaceLiveSyncWorkspaceContext {
    pub(super) root: PathBuf,
    pub(super) identity: crate::io::WorkspaceIdentity,
    pub(super) generation: u64,
    pub(super) identity_changed: bool,
    pub(super) valid: bool,
}
