//! The id of the Project a Room lives under.
use serde::{Deserialize, Deserializer, Serialize};

const HIDDEN_SCOPE_PREFIX: &str = "hidden-scope-";

/// A Project id is never empty: it is checked once where it enters the kernel, and every Room is built with one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ProjectId(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptyProjectId;

impl std::fmt::Display for EmptyProjectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("project_id must not be empty: name the Project the Room lives under")
    }
}

impl ProjectId {
    pub fn new(id: impl Into<String>) -> Result<Self, EmptyProjectId> {
        let id = id.into();
        if id.is_empty() {
            return Err(EmptyProjectId);
        }
        Ok(Self(id))
    }

    /// A hidden backing Room lives under no registered Project until a lease binds one, so it is built with an id of its own.
    pub(crate) fn hidden_scope(session_id: &str) -> Self {
        Self(format!("{HIDDEN_SCOPE_PREFIX}{session_id}"))
    }

    pub fn is_hidden_scope(&self) -> bool {
        self.0.starts_with(HIDDEN_SCOPE_PREFIX)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[cfg(test)]
    pub(crate) fn test() -> Self {
        Self("project-test".into())
    }
}

impl<'de> Deserialize<'de> for ProjectId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl std::fmt::Display for ProjectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for ProjectId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
