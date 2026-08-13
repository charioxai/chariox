use super::*;

impl CharioxConnectorAdapterRegistry {
    pub fn user_root() -> Option<PathBuf> {
        chariox_home().map(|home| home.join("connectors").join("adapters"))
    }

    pub fn user() -> Result<Self, DaemonError> {
        let user_root = Self::user_root().ok_or_else(|| DaemonError::InvalidConfig {
            field: "connector adapter registry root",
            message: "HOME must be set to resolve ~/.chariox/connectors/adapters",
        })?;
        Ok(Self::new(user_root, bundled_adapter_roots()))
    }

    pub fn new(user_root: PathBuf, bundled_roots: Vec<PathBuf>) -> Self {
        Self {
            user_root,
            bundled_roots,
        }
    }

    pub fn install_from_file(
        &self,
        source: &Path,
    ) -> Result<(CharioxConnectorAdapterDefinition, PathBuf), DaemonError> {
        if !source.is_file() {
            return Err(DaemonError::InvalidConfig {
                field: "connector adapter file",
                message: "connector adapter registration requires a YAML file",
            });
        }
        let mut adapter = Self::read_yaml(source, ConnectorAdapterSource::User)?;
        adapter.validate()?;
        ensure_private_dir(&self.user_root, "connector.adapter.register")?;
        let destination = self.user_root.join(&adapter.name);
        if destination.exists() {
            fs::remove_dir_all(&destination).map_err(io_error("connector.adapter.register"))?;
        }
        fs::create_dir_all(&destination).map_err(io_error("connector.adapter.register"))?;
        copy_adapter_package(source, &destination)?;
        set_private_dir_permissions(&destination, "connector.adapter.register")?;
        let manifest = destination.join("adapter.yaml");
        if source.file_name().and_then(|name| name.to_str()) != Some("adapter.yaml") {
            fs::copy(source, &manifest).map_err(io_error("connector.adapter.register"))?;
            set_private_file_permissions(&manifest, "connector.adapter.register")?;
        }
        adapter.manifest_path = Some(manifest.clone());
        adapter.source = Some(ConnectorAdapterSource::User);
        Ok((adapter, manifest))
    }

    pub fn remove(
        &self,
        name: &str,
    ) -> Result<(CharioxConnectorAdapterDefinition, PathBuf), DaemonError> {
        validate_registry_name(name, "connector adapter name")?;
        let path = self.user_root.join(name).join("adapter.yaml");
        if !path.exists() {
            return Err(connector_error(
                "connector.adapter.remove",
                format!("user connector adapter `{name}` is not registered"),
            ));
        }
        let adapter = Self::read_yaml(&path, ConnectorAdapterSource::User)?;
        fs::remove_dir_all(self.user_root.join(name))
            .map_err(io_error("connector.adapter.remove"))?;
        Ok((adapter, path))
    }

    pub fn list(&self) -> Result<Vec<CharioxConnectorAdapterDefinition>, DaemonError> {
        let mut entries = BTreeMap::new();
        for root in &self.bundled_roots {
            read_adapter_root(root, ConnectorAdapterSource::Bundled, &mut entries)?;
        }
        read_adapter_root(&self.user_root, ConnectorAdapterSource::User, &mut entries)?;
        Ok(entries.into_values().collect())
    }

    pub fn get(
        &self,
        name: &str,
    ) -> Result<Option<CharioxConnectorAdapterDefinition>, DaemonError> {
        validate_registry_name(name, "connector adapter name")?;
        let user = self.user_root.join(name).join("adapter.yaml");
        if user.exists() {
            return Self::read_yaml(&user, ConnectorAdapterSource::User).map(Some);
        }
        for root in &self.bundled_roots {
            let path = root.join(name).join("adapter.yaml");
            if path.exists() {
                return Self::read_yaml(&path, ConnectorAdapterSource::Bundled).map(Some);
            }
        }
        Ok(None)
    }

    pub(super) fn read_yaml(
        path: &Path,
        source: ConnectorAdapterSource,
    ) -> Result<CharioxConnectorAdapterDefinition, DaemonError> {
        let contents = fs::read_to_string(path).map_err(io_error("connector.adapter.read"))?;
        let mut adapter = serde_yaml::from_str::<CharioxConnectorAdapterDefinition>(&contents)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "connector.adapter.read",
                message: format!(
                    "failed to parse connector adapter `{}`: {error}",
                    path.display()
                ),
            })?;
        adapter.source = Some(source);
        adapter.manifest_path = Some(path.to_path_buf());
        adapter.validate()?;
        Ok(adapter)
    }
}
