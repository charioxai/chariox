use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionKind {
    Mcp,
    Skill,
    Script,
    Connector,
}

impl ExtensionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Mcp => "mcp",
            Self::Skill => "skill",
            Self::Script => "script",
            Self::Connector => "connector",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ExtensionGrant {
    pub kind: ExtensionKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_safety: Option<String>,
}

impl ExtensionGrant {
    pub fn new(kind: ExtensionKind, name: impl Into<String>) -> Self {
        Self {
            kind,
            name: name.into(),
            environment: None,
            credential: None,
            max_safety: None,
        }
    }

    pub fn script(name: impl Into<String>, environment: impl Into<String>) -> Self {
        Self {
            kind: ExtensionKind::Script,
            name: name.into(),
            environment: Some(environment.into()),
            credential: None,
            max_safety: None,
        }
    }

    pub fn connector(
        name: impl Into<String>,
        credential: Option<String>,
        max_safety: impl Into<String>,
    ) -> Self {
        Self {
            kind: ExtensionKind::Connector,
            name: name.into(),
            environment: None,
            credential,
            max_safety: Some(max_safety.into()),
        }
    }

    pub fn matches(&self, kind: &ExtensionKind, name: &str) -> bool {
        &self.kind == kind && self.name == name
    }
}
