//! MP-08/MP-11: the credential-free export schema, separate from file inspection.
//! Exhaustive typed construction/matches force new runtime fields to be reviewed.
use super::{refused, validate_bytes, MAX_FILE};
use crate::error::DaemonError;
use crate::managed_context::kernel::*;
use crate::mcp::{CharioxMcpServerConfig, CharioxMcpTransportConfig};
use base64::Engine;
use serde_json::Value;

pub(crate) fn exportable_mcp(config: &CharioxMcpServerConfig) -> CharioxMcpServerConfig {
    let CharioxMcpServerConfig {
        name,
        transport,
        enabled,
        required,
        startup_timeout_sec,
        tool_timeout_sec,
        enabled_tools,
        disabled_tools,
        tools,
    } = config;
    // Literal env/header values and ambient or credential bindings are omitted
    // by schema role, including innocuous names. No credential registry is read.
    let transport = match transport {
        CharioxMcpTransportConfig::Stdio {
            command,
            args,
            env: _,
            credential_env: _,
            env_vars: _,
            cwd,
        } => CharioxMcpTransportConfig::Stdio {
            command: command.clone(),
            args: args.clone(),
            cwd: cwd.clone(),
            env: Default::default(),
            credential_env: Default::default(),
            env_vars: vec![],
        },
        CharioxMcpTransportConfig::StreamableHttp {
            url,
            bearer_token_env_var: _,
            bearer_token_credential: _,
            http_headers: _,
            credential_http_headers: _,
            env_http_headers: _,
        } => CharioxMcpTransportConfig::StreamableHttp {
            url: url.clone(),
            bearer_token_env_var: None,
            bearer_token_credential: None,
            http_headers: Default::default(),
            credential_http_headers: Default::default(),
            env_http_headers: Default::default(),
        },
    };
    CharioxMcpServerConfig {
        name: name.clone(),
        transport,
        enabled: *enabled,
        required: *required,
        startup_timeout_sec: *startup_timeout_sec,
        tool_timeout_sec: *tool_timeout_sec,
        enabled_tools: enabled_tools.clone(),
        disabled_tools: disabled_tools.clone(),
        tools: tools
            .iter()
            .map(|(name, policy)| {
                let crate::mcp::CharioxMcpToolConfig { approval_mode } = policy;
                (
                    name.clone(),
                    crate::mcp::CharioxMcpToolConfig {
                        approval_mode: approval_mode.clone(),
                    },
                )
            })
            .collect(),
    }
}

fn string(text: &str) -> Result<(), DaemonError> {
    validate_bytes("context-text", text.as_bytes())
}

// Only values in already allowlisted typed metadata are free text. Object keys
// are identifiers (tool/operation names), never guessed to be secret slots.
fn strings(value: &Value) -> Result<(), DaemonError> {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                string(key)?;
                strings(value)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                strings(value)?;
            }
        }
        Value::String(text) => string(text)?,
        _ => {}
    }
    Ok(())
}

// MP-11: arbitrary settings/defaults/examples are not exportable metadata.
// Property/definition names can describe credential INPUTS without values.
fn schema(value: &Value) -> Result<(), DaemonError> {
    match value {
        Value::Bool(_) => Ok(()),
        Value::Object(fields) => {
            for (key, value) in fields {
                match key.as_str() {
                    "properties" | "$defs" | "definitions" | "patternProperties" => {
                        for (name, child) in value.as_object().ok_or_else(refused)? {
                            string(name)?;
                            schema(child)?;
                        }
                    }
                    "items" | "additionalProperties" | "not" | "contains" | "propertyNames" => {
                        schema(value)?
                    }
                    "allOf" | "anyOf" | "oneOf" | "prefixItems" => {
                        for child in value.as_array().ok_or_else(refused)? {
                            schema(child)?;
                        }
                    }
                    "type" if value.is_string() => string(value.as_str().ok_or_else(refused)?)?,
                    "type" | "required" => {
                        for name in value.as_array().ok_or_else(refused)? {
                            string(name.as_str().ok_or_else(refused)?)?;
                        }
                    }
                    "$schema" | "$id" | "$ref" | "title" | "description" | "format" | "pattern" => {
                        string(value.as_str().ok_or_else(refused)?)?;
                    }
                    "minimum" | "maximum" | "exclusiveMinimum" | "exclusiveMaximum"
                    | "multipleOf" | "minLength" | "maxLength" | "minItems" | "maxItems"
                    | "minProperties" | "maxProperties" => {
                        if !value.is_number() {
                            return Err(refused());
                        }
                    }
                    "uniqueItems" | "readOnly" | "writeOnly" | "deprecated" => {
                        if !value.is_boolean() {
                            return Err(refused());
                        }
                    }
                    _ => return Err(refused()),
                }
            }
            Ok(())
        }
        _ => Err(refused()),
    }
}

fn connector_config(value: &Value) -> Result<(), DaemonError> {
    if value.is_null() {
        return Ok(());
    }
    for (key, value) in value.as_object().ok_or_else(refused)? {
        match key.as_str() {
            "url" | "method" | "path" => string(value.as_str().ok_or_else(refused)?)?,
            "timeout_ms" | "max_response_bytes" => {
                if value.as_u64().is_none() {
                    return Err(refused());
                }
            }
            _ => return Err(refused()),
        }
    }
    Ok(())
}

fn encoded(path: &str, content: &str) -> Result<(), DaemonError> {
    if !crate::managed_context::portable_path::is_portable_relative_path(path) {
        return Err(refused());
    }
    if content.len() > (MAX_FILE as usize).div_ceil(3) * 4 {
        return Err(refused());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(content)
        .map_err(|_| refused())?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(refused());
    }
    validate_bytes(path, &bytes)
}

fn files(files: &[KernelPackageFile]) -> Result<(), DaemonError> {
    for KernelPackageFile {
        path,
        content_base64,
        sha256: _,
        size_bytes: _,
        executable: _,
    } in files
    {
        encoded(path, content_base64)?;
    }
    Ok(())
}

pub(super) fn validate(payload: &KernelContextPayload) -> Result<(), DaemonError> {
    let KernelContextPayload {
        schema_version: _,
        context_id: _,
        source_kernel_id: _,
        source_key_thumbprint: _,
        target_kernel_id: _,
        target_key_thumbprint: _,
        compatibility: _,
        extensions,
        dependencies,
        vault,
    } = payload;
    if vault.is_some() {
        return Err(refused());
    }
    for KernelExtensionSnapshot {
        kind: _,
        scope: _,
        name,
        definition_sha256: _,
        definition,
    } in extensions
    {
        string(name)?;
        match definition {
            KernelExtensionDefinition::Mcp { config, runtime } => {
                if **config != exportable_mcp(config) {
                    return Err(refused());
                }
                strings(&serde_json::to_value(config).map_err(|_| refused())?)?;
                if let CharioxMcpTransportConfig::Stdio { command, args, .. } = &config.transport {
                    // Keep executable/argument context for command-specific flags.
                    validate_bytes(
                        "mcp-command.sh",
                        format!("{} {}", command, args.join(" ")).as_bytes(),
                    )?;
                }
                if let Some(KernelMcpStdioRuntimeSnapshot {
                    command_path,
                    cwd_path,
                    package_sha256: _,
                    files: packaged,
                }) = runtime
                {
                    validate_bytes(command_path, b"")?;
                    if let Some(path) = cwd_path {
                        validate_bytes(path, b"")?;
                    }
                    files(packaged)?;
                }
            }
            KernelExtensionDefinition::Skill {
                package,
                executable_paths,
            } => {
                let crate::skill::CharioxSkillPackage {
                    metadata,
                    version_hash: _,
                    files,
                } = package;
                let crate::skill::CharioxSkillMetadata {
                    name,
                    description,
                    short_description,
                    path,
                } = metadata;
                string(name)?;
                string(description)?;
                if let Some(text) = short_description {
                    string(text)?;
                }
                validate_bytes(path.to_str().ok_or_else(refused)?, b"")?;
                for path in executable_paths {
                    validate_bytes(path, b"")?;
                }
                for crate::skill::CharioxSkillPackageFile {
                    path,
                    content_base64,
                    sha256: _,
                } in files
                {
                    encoded(path, content_base64)?;
                }
            }
            KernelExtensionDefinition::Script { script } => {
                let KernelScriptSnapshot {
                    runtime,
                    description,
                    input_schema,
                    timeout_sec: _,
                    source_sha256: _,
                    source_base64,
                } = script;
                string(description)?;
                schema(input_schema)?;
                let path = match runtime {
                    crate::script::CharioxScriptRuntime::Python => "script.py",
                    crate::script::CharioxScriptRuntime::TypeScript => "script.ts",
                };
                encoded(path, source_base64)?;
            }
            KernelExtensionDefinition::Connector { definition } => {
                let crate::connector::CharioxConnectorDefinition {
                    kind,
                    name,
                    description,
                    adapter,
                    credential,
                    timeout_ms: _,
                    max_response_bytes: _,
                    operations,
                } = definition;
                if let Some(crate::connector::ConnectorCredentialPolicy { required: _ }) =
                    credential
                {}
                for text in [kind, name, description, adapter] {
                    string(text)?;
                }
                for crate::connector::ConnectorOperation {
                    name,
                    description,
                    safety: _,
                    input_schema,
                    config,
                } in operations
                {
                    string(name)?;
                    string(description)?;
                    schema(input_schema)?;
                    connector_config(config)?;
                }
            }
        }
    }
    for dependency in dependencies {
        match dependency {
            KernelExtensionDependency::Credential { .. } => return Err(refused()),
            KernelExtensionDependency::UserRules { body } => string(body)?,
            KernelExtensionDependency::Environment { name, runtime } => {
                string(name)?;
                match runtime {
                    PortableEnvironmentRuntime::Python {
                        version,
                        files: packaged,
                    }
                    | PortableEnvironmentRuntime::Node {
                        version,
                        files: packaged,
                    } => {
                        string(version)?;
                        files(packaged)?;
                    }
                }
            }
            KernelExtensionDependency::UserConnectorAdapter {
                name,
                definition_sha256: _,
                files: packaged,
            } => {
                string(name)?;
                files(packaged)?;
            }
            KernelExtensionDependency::BundledConnectorAdapter {
                name,
                version,
                adapter_protocol,
                artifact_sha256: _,
            } => {
                string(name)?;
                string(adapter_protocol)?;
                if let Some(version) = version {
                    string(version)?;
                }
            }
        }
    }
    Ok(())
}
