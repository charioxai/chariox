use serde::{Deserialize, Serialize};

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListExtensionsArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestExtensionArgs {
    pub kind: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_body: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterMcpArgs {
    pub config: crate::mcp::ArrobaMcpServerConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_to_current_agent: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterSkillPathArgs {
    pub path: std::path::PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_to_current_agent: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterEnvironmentArgs {
    pub config: crate::script::ArrobaEnvironmentConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterScriptPathArgs {
    pub path: std::path::PathBuf,
    pub environment: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_to_current_agent: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterConnectorPathArgs {
    pub path: std::path::PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_to_current_agent: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterConnectorAdapterPathArgs {
    pub path: std::path::PathBuf,
}

pub fn extension_runtime_tool_specs() -> Vec<RuntimeToolSpec> {
    let canonical = vec![
        RuntimeToolSpec {
            name: LIST_EXTENSIONS_TOOL.to_string(),
            description: "List Arroba-managed extensions available in this workspace, including whether they are already granted to the current agent. Use this before requesting an extension.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["mcp", "skill", "script", "connector", "all"]
                    }
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: REQUEST_EXTENSION_TOOL.to_string(),
            description: "Request access to an Arroba-managed MCP, skill, script, or connector for the current agent. Script requests require an environment. Connector requests may include an allow safety level and credential handle.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["kind", "name"],
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["mcp", "skill", "script", "connector"]
                    },
                    "name": {"type": "string"},
                    "reason": {"type": "string"},
                    "return_body": {"type": "boolean"},
                    "environment": {"type": "string"},
                    "allow": {
                        "type": "string",
                        "enum": ["read", "write", "admin"]
                    },
                    "credential": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: REGISTER_MCP_TOOL.to_string(),
            description: "Register or update a global Arroba-managed MCP definition. Set grant_to_current_agent to also grant it to this agent in the same operation.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["config"],
                "properties": {
                    "config": {"type": "object"},
                    "grant_to_current_agent": {"type": "boolean"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: REGISTER_SKILL_PATH_TOOL.to_string(),
            description: "Register or update a global Arroba skill from a directory containing SKILL.md. Set grant_to_current_agent to also grant it to this agent in the same operation.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": {"type": "string"},
                    "grant_to_current_agent": {"type": "boolean"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: REGISTER_ENVIRONMENT_TOOL.to_string(),
            description: "Register or update a global Arroba script execution environment.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["config"],
                "properties": {
                    "config": {"type": "object"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: REGISTER_SCRIPT_PATH_TOOL.to_string(),
            description: "Register a global Arroba script extension from a Python or TypeScript file. Set grant_to_current_agent to also grant it to this agent in the same operation.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["path", "environment"],
                "properties": {
                    "path": {"type": "string"},
                    "environment": {"type": "string"},
                    "name": {"type": "string"},
                    "grant_to_current_agent": {"type": "boolean"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: REGISTER_CONNECTOR_PATH_TOOL.to_string(),
            description: "Register or update a global Arroba connector from connector YAML. Set grant_to_current_agent to also grant it to this agent in the same operation.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": {"type": "string"},
                    "grant_to_current_agent": {"type": "boolean"},
                    "credential": {"type": "string"},
                    "allow": {"type": "string", "enum": ["read", "write", "destructive"]}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: REGISTER_CONNECTOR_ADAPTER_PATH_TOOL.to_string(),
            description: "Register or update a global Arroba connector adapter from an adapter YAML/package.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
    ];
    canonical
}

pub fn canonical_extension_tool_name(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        LIST_EXTENSIONS_TOOL
        | "arroba_list_extensions"
        | "mcp__arroba__list_extensions"
        | "mcp__arroba__arroba_list_extensions" => Some(LIST_EXTENSIONS_TOOL),
        REQUEST_EXTENSION_TOOL
        | "arroba_request_extension"
        | "mcp__arroba__request_extension"
        | "mcp__arroba__arroba_request_extension" => Some(REQUEST_EXTENSION_TOOL),
        REGISTER_MCP_TOOL
        | "arroba_register_mcp"
        | "mcp__arroba__register_mcp"
        | "mcp__arroba__arroba_register_mcp" => Some(REGISTER_MCP_TOOL),
        REGISTER_SKILL_PATH_TOOL
        | "arroba_register_skill_path"
        | "mcp__arroba__register_skill_path"
        | "mcp__arroba__arroba_register_skill_path" => Some(REGISTER_SKILL_PATH_TOOL),
        REGISTER_ENVIRONMENT_TOOL
        | "arroba_register_environment"
        | "mcp__arroba__register_environment"
        | "mcp__arroba__arroba_register_environment" => Some(REGISTER_ENVIRONMENT_TOOL),
        REGISTER_SCRIPT_PATH_TOOL
        | "arroba_register_script_path"
        | "mcp__arroba__register_script_path"
        | "mcp__arroba__arroba_register_script_path" => Some(REGISTER_SCRIPT_PATH_TOOL),
        REGISTER_CONNECTOR_PATH_TOOL
        | "arroba_register_connector_path"
        | "mcp__arroba__register_connector_path"
        | "mcp__arroba__arroba_register_connector_path" => Some(REGISTER_CONNECTOR_PATH_TOOL),
        REGISTER_CONNECTOR_ADAPTER_PATH_TOOL
        | "arroba_register_connector_adapter_path"
        | "mcp__arroba__register_connector_adapter_path"
        | "mcp__arroba__arroba_register_connector_adapter_path" => {
            Some(REGISTER_CONNECTOR_ADAPTER_PATH_TOOL)
        }
        _ => None,
    }
}
