use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArtifactId(String);

impl ArtifactId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ArtifactId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArtifactVersion(u64);

impl ArtifactVersion {
    pub fn initial() -> Self {
        Self(1)
    }

    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }

    pub fn value(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArtifactSnapshotId(String);

impl ArtifactSnapshotId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ArtifactSnapshotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceIdentity {
    pub vcs_provider: Option<String>,
    pub repo_id: Option<String>,
    pub repo_url: Option<String>,
    pub branch: Option<String>,
    pub head_commit: Option<String>,
    pub worktree_root_fingerprint: String,
}

impl WorkspaceIdentity {
    pub fn local(worktree_root_fingerprint: impl Into<String>) -> Self {
        Self {
            vcs_provider: None,
            repo_id: None,
            repo_url: None,
            branch: None,
            head_commit: None,
            worktree_root_fingerprint: worktree_root_fingerprint.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactReservationOwner {
    pub provider_run_id: String,
    pub agent_instance_id: Option<String>,
    pub tool_name: String,
}

impl ArtifactReservationOwner {
    pub fn new(
        provider_run_id: impl Into<String>,
        agent_instance_id: Option<String>,
        tool_name: impl Into<String>,
    ) -> Self {
        Self {
            provider_run_id: provider_run_id.into(),
            agent_instance_id,
            tool_name: tool_name.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactReservationSnapshot {
    pub artifact_id: ArtifactId,
    pub owner: ArtifactReservationOwner,
    pub ranges: Vec<TextRange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactDomainKind {
    TextDocument,
    StructuredDocument,
    OpaqueBlob,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactContent {
    Text(String),
    Bytes(Vec<u8>),
}

impl ArtifactContent {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Bytes(_) => None,
        }
    }

    pub fn into_text(self) -> Option<String> {
        match self {
            Self::Text(text) => Some(text),
            Self::Bytes(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextRange {
    pub start: usize,
    pub end: usize,
}

impl TextRange {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end
    }

    pub fn overlaps(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentEditOperation {
    ReplaceText {
        old_text: String,
        new_text: String,
    },
    ReplaceRange {
        range: TextRange,
        old_text: String,
        new_text: String,
    },
    WriteArtifact {
        content: ArtifactContent,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentEditIntent {
    pub path: PathBuf,
    pub snapshot_id: Option<ArtifactSnapshotId>,
    pub operation: AgentEditOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactReadResult {
    pub artifact_id: ArtifactId,
    pub path: PathBuf,
    pub domain: ArtifactDomainKind,
    pub version: ArtifactVersion,
    pub snapshot_id: ArtifactSnapshotId,
    pub content: ArtifactContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactEditWarning {
    RebasedOverNonOverlappingChange {
        base_version: ArtifactVersion,
        applied_version: ArtifactVersion,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactEditError {
    ArtifactNotTracked {
        path: PathBuf,
    },
    SnapshotNotFound {
        snapshot_id: ArtifactSnapshotId,
    },
    UnsupportedDomain {
        domain: ArtifactDomainKind,
    },
    InvalidOperation {
        message: String,
    },
    Filesystem {
        path: PathBuf,
        message: String,
    },
    ExternalChangeDuringApply {
        path: PathBuf,
    },
    ActiveReservationConflict {
        path: PathBuf,
        active_owner: ArtifactReservationOwner,
        requested_ranges: Vec<TextRange>,
        reserved_ranges: Vec<TextRange>,
        message: String,
    },
    Conflict {
        path: PathBuf,
        base_version: ArtifactVersion,
        current_version: ArtifactVersion,
        requested_ranges: Vec<TextRange>,
        changed_ranges: Vec<TextRange>,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditResult {
    Applied {
        new_version: ArtifactVersion,
    },
    AppliedWithWarning {
        new_version: ArtifactVersion,
        warning: ArtifactEditWarning,
    },
    Rejected {
        reason: ArtifactEditError,
    },
}
