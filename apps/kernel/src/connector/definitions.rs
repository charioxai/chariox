use super::*;

impl CharioxConnectorDefinition {
    pub fn validate(&self) -> Result<(), DaemonError> {
        if self.kind != "connector" {
            return Err(DaemonError::InvalidConfig {
                field: "kind",
                message: "connector YAML kind must be `connector`",
            });
        }
        validate_registry_name(&self.name, "connector name")?;
        validate_registry_name(&self.adapter, "connector adapter")?;
        if self.description.trim().is_empty() {
            return Err(DaemonError::InvalidConfig {
                field: "description",
                message: "connector description must not be empty",
            });
        }
        if self.timeout_ms == 0 {
            return Err(DaemonError::InvalidConfig {
                field: "timeout_ms",
                message: "connector timeout_ms must be greater than zero",
            });
        }
        if self.max_response_bytes == 0 {
            return Err(DaemonError::InvalidConfig {
                field: "max_response_bytes",
                message: "connector max_response_bytes must be greater than zero",
            });
        }
        if self.operations.is_empty() {
            return Err(DaemonError::InvalidConfig {
                field: "operations",
                message: "connector must define at least one operation",
            });
        }
        let mut seen = BTreeSet::new();
        for operation in &self.operations {
            validate_registry_name(&operation.name, "operation name")?;
            if !seen.insert(operation.name.as_str()) {
                return Err(DaemonError::InvalidConfig {
                    field: "operations.name",
                    message: "operation names must be unique",
                });
            }
            if operation.description.trim().is_empty() {
                return Err(DaemonError::InvalidConfig {
                    field: "operations.description",
                    message: "operation description must not be empty",
                });
            }
            JSONSchema::compile(&operation.input_schema).map_err(|error| {
                connector_error(
                    "connector.validate",
                    format!("invalid JSON schema: {error}"),
                )
            })?;
        }
        Ok(())
    }

    pub fn operation(&self, name: &str) -> Result<&ConnectorOperation, DaemonError> {
        self.operations
            .iter()
            .find(|operation| operation.name == name)
            .ok_or_else(|| {
                connector_error(
                    "connector.operation",
                    format!("connector `{}` has no operation `{name}`", self.name),
                )
            })
    }

    pub fn operation_tool_name(&self, operation: &str) -> String {
        connector_tool_name(&self.name, operation)
    }

    pub fn allowed_operation_tool_names(&self, max_safety: ConnectorSafety) -> Vec<String> {
        self.operations
            .iter()
            .filter(|operation| operation.safety <= max_safety)
            .map(|operation| self.operation_tool_name(&operation.name))
            .collect()
    }

    pub fn definition_hash(&self) -> Result<String, DaemonError> {
        let bytes = serde_json::to_vec(self).map_err(|error| DaemonError::LocalTransport {
            operation: "connector.definition_hash",
            message: format!("failed to serialize connector `{}`: {error}", self.name),
        })?;
        let digest = Sha256::digest(&bytes);
        Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
    }
}

impl CharioxConnectorAdapterDefinition {
    pub fn validate(&self) -> Result<(), DaemonError> {
        if self.kind != "connector_adapter" {
            return Err(DaemonError::InvalidConfig {
                field: "kind",
                message: "connector adapter YAML kind must be `connector_adapter`",
            });
        }
        validate_registry_name(&self.name, "connector adapter name")?;
        if self.adapter_protocol != CONNECTOR_ADAPTER_PROTOCOL_VERSION {
            return Err(DaemonError::InvalidConfig {
                field: "adapter_protocol",
                message: "unsupported connector adapter protocol",
            });
        }
        if self.command.as_os_str().is_empty() {
            return Err(DaemonError::InvalidConfig {
                field: "command",
                message: "connector adapter command must not be empty",
            });
        }
        Ok(())
    }

    pub fn resolved_command(&self) -> Result<PathBuf, DaemonError> {
        if self.command.is_absolute() {
            return Ok(self.command.clone());
        }
        if self.command.components().count() == 1 {
            return Ok(self.command.clone());
        }
        let manifest = self.manifest_path.as_ref().ok_or_else(|| {
            connector_error(
                "connector.adapter",
                format!("connector adapter `{}` has no manifest path", self.name),
            )
        })?;
        let parent = manifest.parent().ok_or_else(|| {
            connector_error(
                "connector.adapter",
                format!("connector adapter `{}` manifest has no parent", self.name),
            )
        })?;
        Ok(parent.join(&self.command))
    }
}

impl ConnectorSafety {
    pub fn parse(value: Option<&str>) -> Result<Self, DaemonError> {
        match value.unwrap_or("read") {
            "read" => Ok(Self::Read),
            "write" => Ok(Self::Write),
            "destructive" => Ok(Self::Destructive),
            other => Err(connector_error(
                "connector.safety",
                format!("unknown connector safety `{other}`"),
            )),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Destructive => "destructive",
        }
    }
}
