mod coordinator;
mod external_change_monitor;
mod filesystem;
mod text;
mod types;

pub use coordinator::{
    ArtifactEditCoordinator, ArtifactReadRequest, ArtifactReservationToken, ArtifactWriteRequest,
    PreparedArtifactEdit,
};
pub(crate) use external_change_monitor::{
    ArtifactExternalChangeHealthSnapshot, ArtifactExternalChangeMonitor,
    ArtifactExternalChangeNotice,
};
pub use filesystem::{
    WorkspaceLiveSyncFileIo, WorkspaceLiveSyncFileReadRequest, WorkspaceLiveSyncFileWriteRequest,
};
pub use text::TextDocumentDomain;
pub use types::{
    AgentEditIntent, AgentEditOperation, ArtifactContent, ArtifactDomainKind, ArtifactEditError,
    ArtifactEditWarning, ArtifactId, ArtifactReadResult, ArtifactReservationOwner,
    ArtifactReservationSnapshot, ArtifactSnapshotId, ArtifactVersion, EditResult, TextRange,
    WorkspaceIdentity,
};
