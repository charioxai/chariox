use super::*;

impl ArrobaConnectorRegistry {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn user_root() -> Option<PathBuf> {
        arroba_home().map(|home| home.join("connectors").join("definitions"))
    }

    pub fn user() -> Result<Self, DaemonError> {
        let root = Self::user_root().ok_or_else(|| DaemonError::InvalidConfig {
            field: "connector registry root",
            message: "HOME must be set to resolve ~/.arroba/connectors/definitions",
        })?;
        Ok(Self::new(root))
    }

    pub fn install_from_file(
        &self,
        source: &Path,
        adapters: &ArrobaConnectorAdapterRegistry,
    ) -> Result<(ArrobaConnectorDefinition, PathBuf), DaemonError> {
        if !source.is_file() {
            return Err(DaemonError::InvalidConfig {
                field: "connector file",
                message: "connector registration requires a YAML file",
            });
        }
        let definition = Self::read_yaml(source)?;
        definition.validate()?;
        let adapter = adapters.get(&definition.adapter)?.ok_or_else(|| {
            connector_error(
                "connector.register",
                format!(
                    "connector adapter `{}` is not registered",
                    definition.adapter
                ),
            )
        })?;
        validate_connector_with_adapter(&definition, &adapter)?;
        ensure_private_dir(&self.root, "connector.register")?;
        let path = self.path_for(&definition.name)?;
        let payload =
            serde_yaml::to_string(&definition).map_err(|error| DaemonError::LocalTransport {
                operation: "connector.register",
                message: format!(
                    "failed to serialize connector `{}`: {error}",
                    definition.name
                ),
            })?;
        atomic_write_private(&path, payload.as_bytes(), "connector.register")?;
        Ok((definition, path))
    }

    pub fn upsert_definition(
        &self,
        definition: &ArrobaConnectorDefinition,
        adapters: &ArrobaConnectorAdapterRegistry,
    ) -> Result<(ArrobaConnectorDefinition, PathBuf), DaemonError> {
        definition.validate()?;
        let adapter = adapters.get(&definition.adapter)?.ok_or_else(|| {
            connector_error(
                "connector.upsert",
                format!(
                    "connector adapter `{}` is not registered",
                    definition.adapter
                ),
            )
        })?;
        validate_connector_with_adapter(definition, &adapter)?;
        ensure_private_dir(&self.root, "connector.upsert")?;
        let path = self.path_for(&definition.name)?;
        let payload =
            serde_yaml::to_string(definition).map_err(|error| DaemonError::LocalTransport {
                operation: "connector.upsert",
                message: format!(
                    "failed to serialize connector `{}`: {error}",
                    definition.name
                ),
            })?;
        atomic_write_private(&path, payload.as_bytes(), "connector.upsert")?;
        Ok((definition.clone(), path))
    }

    pub fn remove(&self, name: &str) -> Result<(ArrobaConnectorDefinition, PathBuf), DaemonError> {
        let path = self
            .find_path(name)?
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "connector.remove",
                message: format!("connector `{name}` is not registered"),
            })?;
        let definition = Self::read_yaml(&path)?;
        fs::remove_file(&path).map_err(io_error("connector.remove"))?;
        Ok((definition, path))
    }

    pub fn list(&self) -> Result<Vec<ArrobaConnectorDefinition>, DaemonError> {
        let mut entries = BTreeMap::new();
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        for entry in fs::read_dir(&self.root).map_err(io_error("connector.list"))? {
            let path = entry.map_err(io_error("connector.list"))?.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("yaml") {
                continue;
            }
            let definition = Self::read_yaml(&path)?;
            entries.entry(definition.name.clone()).or_insert(definition);
        }
        Ok(entries.into_values().collect())
    }

    pub fn get(&self, name: &str) -> Result<Option<ArrobaConnectorDefinition>, DaemonError> {
        let Some(path) = self.find_path(name)? else {
            return Ok(None);
        };
        Self::read_yaml(&path).map(Some)
    }

    pub fn prepare_call(
        &self,
        adapters: &ArrobaConnectorAdapterRegistry,
        connector_name: &str,
        operation_name: &str,
        credential_id: Option<&str>,
        max_safety: ConnectorSafety,
        arguments: Value,
        vault_config: crate::config::UserCredentialVaultConfig,
    ) -> Result<PreparedConnectorCall, DaemonError> {
        let definition = self.get(connector_name)?.ok_or_else(|| {
            connector_error(
                "connector.execute",
                format!("connector `{connector_name}` is not registered"),
            )
        })?;
        let operation = definition.operation(operation_name)?.clone();
        if operation.safety > max_safety {
            return Err(connector_error(
                "connector.execute",
                format!(
                    "operation `{connector_name}.{operation_name}` requires {:?} safety but grant allows {:?}",
                    operation.safety, max_safety
                ),
            ));
        }
        validate_arguments(&operation.input_schema, &arguments)?;
        let adapter = adapters.get(&definition.adapter)?.ok_or_else(|| {
            connector_error(
                "connector.execute",
                format!(
                    "connector adapter `{}` is not registered",
                    definition.adapter
                ),
            )
        })?;
        let credential_metadata = connector_credential_metadata(
            credential_id,
            definition
                .credential
                .as_ref()
                .map(|credential| credential.required)
                .unwrap_or(false),
        )?;
        let prepare_request = ConnectorAdapterRequest {
            id: "prepare-1".to_string(),
            request_type: ConnectorAdapterRequestType::Prepare,
            connector: definition.name.clone(),
            operation: Some(operation.name.clone()),
            arguments: Some(arguments),
            config: Some(operation.config.clone()),
            operations: Vec::new(),
            credential: None,
            timeout_ms: definition.timeout_ms,
            max_response_bytes: definition.max_response_bytes,
        };
        let prepare_response = run_adapter_request_once(&adapter, &prepare_request)?;
        let prepared = adapter_response_to_prepare_result(prepare_response)?;
        let credential = resolve_connector_credential(
            credential_metadata.as_ref(),
            &prepared.credential_targets,
            &vault_config,
        )?;
        let request = ConnectorAdapterRequest {
            id: "call-1".to_string(),
            request_type: ConnectorAdapterRequestType::Call,
            connector: definition.name.clone(),
            operation: Some(operation.name.clone()),
            arguments: None,
            config: Some(prepared.prepared_config),
            operations: Vec::new(),
            credential,
            timeout_ms: definition.timeout_ms,
            max_response_bytes: definition.max_response_bytes,
        };
        Ok(PreparedConnectorCall {
            connector: definition.name,
            operation: operation.name,
            safety: operation.safety,
            request,
            adapter,
        })
    }

    pub fn execute_once(
        &self,
        adapters: &ArrobaConnectorAdapterRegistry,
        connector_name: &str,
        operation_name: &str,
        credential_id: Option<&str>,
        max_safety: ConnectorSafety,
        arguments: Value,
        vault_config: crate::config::UserCredentialVaultConfig,
    ) -> Result<ConnectorExecution, DaemonError> {
        let prepared = self.prepare_call(
            adapters,
            connector_name,
            operation_name,
            credential_id,
            max_safety,
            arguments,
            vault_config,
        )?;
        let response = run_adapter_request_once(&prepared.adapter, &prepared.request)?;
        adapter_response_to_execution(prepared, response)
    }

    pub fn path_for(&self, name: &str) -> Result<PathBuf, DaemonError> {
        validate_registry_name(name, "connector name")?;
        Ok(self.root.join(format!("{name}.yaml")))
    }

    fn find_path(&self, name: &str) -> Result<Option<PathBuf>, DaemonError> {
        let path = self.path_for(name)?;
        Ok(path.exists().then_some(path))
    }

    fn read_yaml(path: &Path) -> Result<ArrobaConnectorDefinition, DaemonError> {
        let contents = fs::read_to_string(path).map_err(io_error("connector.read"))?;
        let definition =
            serde_yaml::from_str::<ArrobaConnectorDefinition>(&contents).map_err(|error| {
                DaemonError::LocalTransport {
                    operation: "connector.read",
                    message: format!("failed to parse connector `{}`: {error}", path.display()),
                }
            })?;
        definition.validate()?;
        Ok(definition)
    }
}
