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

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionAuthority {
    Home,
    Worker,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionDefinitionOrigin {
    Home,
    Worker,
    ProjectedSnapshot,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionExecutionLocation {
    Home,
    Worker,
    None,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteExtensionManifest {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<RemoteExtensionTool>,
}

impl RemoteExtensionManifest {
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub fn validate_unique_tool_names(
        &self,
        operation: &'static str,
    ) -> Result<(), crate::error::DaemonError> {
        let mut seen = std::collections::BTreeMap::<&str, &RemoteExtensionTool>::new();
        for tool in &self.tools {
            if let Some(existing) = seen.insert(tool.tool_name.as_str(), tool) {
                return Err(crate::error::DaemonError::LocalTransport {
                    operation,
                    message: format!(
                        "home-proxy extension tool name `{}` is duplicated by `{}:{}` and `{}:{}`",
                        tool.tool_name,
                        existing.kind.as_str(),
                        existing.name,
                        tool.kind.as_str(),
                        tool.name
                    ),
                });
            }
        }
        Ok(())
    }

    pub fn home_proxy_runtime_tool_specs(
        &self,
    ) -> impl Iterator<Item = crate::transport::runtime_tools::RuntimeToolSpec> + '_ {
        self.tools
            .iter()
            .filter(|tool| tool.execution_location == ExtensionExecutionLocation::Home)
            .filter(|tool| matches!(tool.kind, ExtensionKind::Script | ExtensionKind::Connector))
            .map(|tool| crate::transport::runtime_tools::RuntimeToolSpec {
                name: tool.tool_name.clone(),
                description: tool.description.clone(),
                input_schema: tool.input_schema.clone(),
            })
    }

    pub fn home_proxy_mcp_server_names(&self) -> impl Iterator<Item = &str> {
        self.tools
            .iter()
            .filter(|tool| tool.execution_location == ExtensionExecutionLocation::Home)
            .filter(|tool| tool.kind == ExtensionKind::Mcp)
            .map(|tool| tool.tool_name.as_str())
    }

    pub fn home_proxy_tool(&self, tool_name: &str) -> Option<&RemoteExtensionTool> {
        self.tools.iter().find(|tool| {
            tool.execution_location == ExtensionExecutionLocation::Home
                && tool.tool_name == tool_name
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteExtensionTool {
    pub kind: ExtensionKind,
    pub name: String,
    pub tool_name: String,
    pub description: String,
    #[serde(default)]
    pub input_schema: serde_json::Value,
    pub authority: ExtensionAuthority,
    pub definition_origin: ExtensionDefinitionOrigin,
    pub execution_location: ExtensionExecutionLocation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_sec: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_hash: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(kind: ExtensionKind, name: &str) -> RemoteExtensionTool {
        RemoteExtensionTool {
            kind,
            name: name.to_string(),
            tool_name: name.to_string(),
            description: format!("{name} description"),
            input_schema: serde_json::json!({"type": "object"}),
            authority: ExtensionAuthority::Home,
            definition_origin: ExtensionDefinitionOrigin::Home,
            execution_location: ExtensionExecutionLocation::Home,
            safety: None,
            timeout_sec: None,
            version_hash: None,
        }
    }

    #[test]
    fn remote_manifest_projects_runtime_tools_but_not_mcp_servers() {
        let manifest = RemoteExtensionManifest {
            tools: vec![
                tool(ExtensionKind::Script, "home_script"),
                tool(ExtensionKind::Connector, "home_connector_lookup"),
                tool(ExtensionKind::Mcp, "home_browser"),
            ],
        };

        let specs = manifest
            .home_proxy_runtime_tool_specs()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();

        assert_eq!(specs, vec!["home_script", "home_connector_lookup"]);
        assert_eq!(
            manifest.home_proxy_mcp_server_names().collect::<Vec<_>>(),
            vec!["home_browser"]
        );
        assert!(manifest.home_proxy_tool("home_script").is_some());
        assert!(manifest.home_proxy_tool("missing").is_none());
    }

    #[test]
    fn remote_manifest_rejects_duplicate_home_proxy_tool_names() {
        let manifest = RemoteExtensionManifest {
            tools: vec![
                tool(ExtensionKind::Script, "shared_name"),
                tool(ExtensionKind::Connector, "shared_name"),
            ],
        };

        let error = manifest
            .validate_unique_tool_names("test manifest")
            .expect_err("duplicate tool names should be rejected");

        assert!(error.to_string().contains("duplicated"));
    }
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
