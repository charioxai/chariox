mod coordinator;
mod text;
mod types;

pub use coordinator::{ArtifactEditCoordinator, ArtifactReadRequest, ArtifactWriteRequest};
pub use text::TextDocumentDomain;
pub use types::{
    AgentEditIntent, AgentEditOperation, ArtifactContent, ArtifactDomainKind, ArtifactEditError,
    ArtifactEditWarning, ArtifactId, ArtifactReadResult, ArtifactSnapshotId, ArtifactVersion,
    EditResult, TextRange, WorkspaceIdentity,
};
