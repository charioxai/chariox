mod coordinator;
mod filesystem;
mod text;
mod types;

pub use coordinator::{
    ArtifactEditCoordinator, ArtifactReadRequest, ArtifactWriteRequest, PreparedArtifactEdit,
};
pub use filesystem::{ManagedFileIo, ManagedFileReadRequest, ManagedFileWriteRequest};
pub use text::TextDocumentDomain;
pub use types::{
    AgentEditIntent, AgentEditOperation, ArtifactContent, ArtifactDomainKind, ArtifactEditError,
    ArtifactEditWarning, ArtifactId, ArtifactReadResult, ArtifactSnapshotId, ArtifactVersion,
    EditResult, TextRange, WorkspaceIdentity,
};
