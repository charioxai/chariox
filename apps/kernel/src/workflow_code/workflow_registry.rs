use super::*;

impl WorkflowRegistry {
    pub fn new(workspace_root: Option<PathBuf>, user_root: Option<PathBuf>) -> Self {
        Self {
            workspace_root,
            user_root,
        }
    }

    pub fn workspace_root(workspace: impl AsRef<Path>) -> PathBuf {
        workspace.as_ref().join(".arroba").join("workflows")
    }

    pub fn user_root() -> Option<PathBuf> {
        arroba_home().map(|home| home.join("workflows"))
    }

    pub fn add(
        &self,
        name: &str,
        scope: WorkflowRegistrySourceScope,
        source: WorkflowRegistrySourceInput,
        node_path: &str,
        limits: &WorkflowCodeLimitsConfig,
    ) -> Result<WorkflowRegistryEntryMetadata, crate::DaemonError> {
        validate_registry_name(name, "workflow registry entry name")?;
        let root = self.write_root(scope.clone())?;
        let entry_dir = root.join(name);
        if entry_dir.exists() || root.join(format!("{name}.js")).exists() {
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_registry.add",
                message: format!("workflow registry entry `{name}` already exists"),
            });
        }
        let temp_dir = root.join(format!(
            ".{name}.tmp-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        if temp_dir.exists() {
            fs::remove_dir_all(&temp_dir).map_err(io_error("workflow_registry.add"))?;
        }
        fs::create_dir_all(&temp_dir).map_err(io_error("workflow_registry.add"))?;
        let result = self.write_entry_to_dir(name, scope, &temp_dir, source, node_path, limits);
        match result {
            Ok(metadata) => {
                if let Some(parent) = entry_dir.parent() {
                    fs::create_dir_all(parent).map_err(io_error("workflow_registry.add"))?;
                }
                fs::rename(&temp_dir, &entry_dir).map_err(io_error("workflow_registry.add"))?;
                Ok(metadata)
            }
            Err(error) => {
                fs::remove_dir_all(&temp_dir).ok();
                Err(error)
            }
        }
    }

    pub fn add_from_export(
        &self,
        name: &str,
        scope: WorkflowRegistrySourceScope,
        export: WorkflowCodeSourceExport,
        node_path: &str,
        limits: &WorkflowCodeLimitsConfig,
    ) -> Result<WorkflowRegistryEntryMetadata, crate::DaemonError> {
        let source = match export.format {
            WorkflowCodeSourceExportFormat::Inline => WorkflowRegistrySourceInput::SingleFile {
                source: export.source,
                source_path: Some(export.source_path),
            },
            WorkflowCodeSourceExportFormat::Directory => {
                WorkflowRegistrySourceInput::SourceDirectory {
                    files: export.files,
                }
            }
        };
        self.add(name, scope, source, node_path, limits)
    }

    pub fn list(&self) -> Result<Vec<WorkflowRegistryEntryMetadata>, crate::DaemonError> {
        let mut entries = BTreeMap::new();
        if let Some(root) = self.workspace_root.as_deref() {
            for entry in self.list_root(root, WorkflowRegistrySourceScope::Workspace)? {
                entries.entry(entry.name.clone()).or_insert(entry);
            }
        }
        if let Some(root) = self.user_root.as_deref() {
            for entry in self.list_root(root, WorkflowRegistrySourceScope::User)? {
                entries.entry(entry.name.clone()).or_insert(entry);
            }
        }
        for example in WORKFLOW_CODE_PATTERN_EXAMPLES {
            entries
                .entry(example.slug.to_string())
                .or_insert_with(|| builtin_workflow_registry_metadata(example));
        }
        Ok(entries.into_values().collect())
    }

    pub fn get(&self, name: &str) -> Result<WorkflowRegistryEntryMetadata, crate::DaemonError> {
        Ok(self.resolve(name)?.metadata)
    }

    pub fn resolve(&self, name: &str) -> Result<WorkflowRegistryResolvedEntry, crate::DaemonError> {
        validate_registry_name(name, "workflow registry entry name")?;
        if let Some(root) = self.workspace_root.as_deref() {
            if let Some(entry) =
                self.resolve_root(root, name, WorkflowRegistrySourceScope::Workspace)?
            {
                return Ok(entry);
            }
        }
        if let Some(root) = self.user_root.as_deref() {
            if let Some(entry) = self.resolve_root(root, name, WorkflowRegistrySourceScope::User)? {
                return Ok(entry);
            }
        }
        if let Some(example) = WORKFLOW_CODE_PATTERN_EXAMPLES
            .iter()
            .find(|example| example.slug == name)
        {
            return Ok(WorkflowRegistryResolvedEntry {
                metadata: builtin_workflow_registry_metadata(example),
                source: example.source.to_string(),
                node_path: example.path.to_string(),
                schema_import_root: None,
            });
        }
        Err(crate::DaemonError::LocalTransport {
            operation: "workflow_registry.resolve",
            message: format!("workflow registry entry `{name}` was not found"),
        })
    }

    pub fn delete(
        &self,
        name: &str,
        scope: Option<WorkflowRegistrySourceScope>,
    ) -> Result<PathBuf, crate::DaemonError> {
        validate_registry_name(name, "workflow registry entry name")?;
        let scopes = match scope {
            Some(WorkflowRegistrySourceScope::Builtin) => {
                return Err(crate::DaemonError::LocalTransport {
                    operation: "workflow_registry.delete",
                    message: format!("builtin workflow registry entry `{name}` cannot be deleted"),
                });
            }
            Some(scope) => vec![scope],
            None => vec![
                WorkflowRegistrySourceScope::Workspace,
                WorkflowRegistrySourceScope::User,
            ],
        };
        for candidate_scope in scopes {
            let Some(root) = self.root_for_scope(candidate_scope) else {
                continue;
            };
            let dir = root.join(name);
            if dir.exists() {
                fs::remove_dir_all(&dir).map_err(io_error("workflow_registry.delete"))?;
                return Ok(dir);
            }
            let file = root.join(format!("{name}.js"));
            if file.exists() {
                fs::remove_file(&file).map_err(io_error("workflow_registry.delete"))?;
                return Ok(file);
            }
        }
        if WORKFLOW_CODE_PATTERN_EXAMPLES
            .iter()
            .any(|example| example.slug == name)
        {
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_registry.delete",
                message: format!("builtin workflow registry entry `{name}` cannot be deleted"),
            });
        }
        Err(crate::DaemonError::LocalTransport {
            operation: "workflow_registry.delete",
            message: format!("workflow registry entry `{name}` was not found"),
        })
    }

    fn write_entry_to_dir(
        &self,
        name: &str,
        scope: WorkflowRegistrySourceScope,
        entry_dir: &Path,
        source: WorkflowRegistrySourceInput,
        node_path: &str,
        limits: &WorkflowCodeLimitsConfig,
    ) -> Result<WorkflowRegistryEntryMetadata, crate::DaemonError> {
        let (source_kind, source_path, files) = normalize_workflow_registry_input(source)?;
        for file in &files {
            if source_kind == WorkflowRegistrySourceKind::SourceDirectory
                && file.path == "manifest.json"
            {
                continue;
            }
            write_registry_file(entry_dir, &file.path, &file.contents)?;
        }
        let source_file = entry_dir.join(&source_path);
        let source = fs::read_to_string(&source_file).map_err(io_error("workflow_registry.add"))?;
        validate_workflow_registry_source_directory_manifest(entry_dir, &files, &source_path)?;
        let schema_import_root = match source_kind {
            WorkflowRegistrySourceKind::SingleFile => None,
            WorkflowRegistrySourceKind::SourceDirectory => Some(entry_dir),
        };
        let compile = compile_workflow_code_source_with_schema_import_root(
            node_path,
            &source,
            WorkflowCodeLanguage::JavaScript,
            limits,
            schema_import_root,
        )?;
        if !compile.validation.ok {
            let diagnostics = workflow_registry_validation_diagnostics(&compile.validation);
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_registry.add",
                message: format!(
                    "workflow registry entry `{name}` is invalid: {}",
                    diagnostics.join(", ")
                ),
            });
        }
        let now = crate::session::unix_epoch_ms();
        let source_sha256 = sha256_hex(source.as_bytes());
        let file_sha256 = files
            .iter()
            .filter(|file| {
                source_kind != WorkflowRegistrySourceKind::SourceDirectory
                    || file.path != "manifest.json"
            })
            .map(|file| (file.path.clone(), sha256_hex(file.contents.as_bytes())))
            .collect::<BTreeMap<_, _>>();
        let validation = WorkflowRegistryValidationSummary {
            ok: compile.validation.ok,
            diagnostics: workflow_registry_validation_diagnostics(&compile.validation),
        };
        let manifest = StoredWorkflowRegistryManifest {
            manifest_version: WORKFLOW_REGISTRY_MANIFEST_VERSION,
            name: name.to_string(),
            source_kind: source_kind.clone(),
            source_path: source_path.clone(),
            source_sha256: source_sha256.clone(),
            source_bytes: source.len() as u64,
            definition_sha256: Some(workflow_code_definition_sha256_hex(&compile.definition)),
            file_sha256,
            created_at_ms: now,
            updated_at_ms: now,
            validation: validation.clone(),
            summary: Some(WorkflowRegistryEntrySummary::from_definition(
                &compile.definition,
            )),
            parameters_schema: compile.definition.parameters_schema.clone(),
        };
        write_workflow_registry_manifest(&entry_dir.join("manifest.json"), &manifest)?;
        Ok(manifest.into_metadata(scope))
    }
}

impl WorkflowRegistry {
    fn list_root(
        &self,
        root: &Path,
        scope: WorkflowRegistrySourceScope,
    ) -> Result<Vec<WorkflowRegistryEntryMetadata>, crate::DaemonError> {
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut entries = Vec::new();
        for entry in fs::read_dir(root).map_err(io_error("workflow_registry.list"))? {
            let path = entry.map_err(io_error("workflow_registry.list"))?.path();
            if path.is_dir() {
                let manifest_path = path.join("manifest.json");
                if manifest_path.exists() {
                    let manifest = read_workflow_registry_manifest(&manifest_path)?;
                    entries.push(manifest.into_metadata(scope.clone()));
                }
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("js") {
                entries.push(single_file_workflow_registry_metadata(
                    &path,
                    scope.clone(),
                )?);
            }
        }
        Ok(entries)
    }

    fn resolve_root(
        &self,
        root: &Path,
        name: &str,
        scope: WorkflowRegistrySourceScope,
    ) -> Result<Option<WorkflowRegistryResolvedEntry>, crate::DaemonError> {
        let entry_dir = root.join(name);
        if entry_dir.is_dir() {
            let manifest_path = entry_dir.join("manifest.json");
            let manifest = read_workflow_registry_manifest(&manifest_path)?;
            validate_workflow_registry_manifest_hashes(&entry_dir, &manifest)?;
            let source_path = entry_dir.join(&manifest.source_path);
            let source =
                fs::read_to_string(&source_path).map_err(io_error("workflow_registry.get"))?;
            let schema_import_root = match manifest.source_kind {
                WorkflowRegistrySourceKind::SingleFile => None,
                WorkflowRegistrySourceKind::SourceDirectory => Some(entry_dir.clone()),
            };
            return Ok(Some(WorkflowRegistryResolvedEntry {
                metadata: manifest.into_metadata(scope),
                source,
                node_path: source_path.display().to_string(),
                schema_import_root,
            }));
        }
        let source_path = root.join(format!("{name}.js"));
        if source_path.is_file() {
            let source =
                fs::read_to_string(&source_path).map_err(io_error("workflow_registry.get"))?;
            let metadata = single_file_workflow_registry_metadata(&source_path, scope)?;
            return Ok(Some(WorkflowRegistryResolvedEntry {
                metadata,
                source,
                node_path: source_path.display().to_string(),
                schema_import_root: None,
            }));
        }
        Ok(None)
    }

    fn write_root(
        &self,
        scope: WorkflowRegistrySourceScope,
    ) -> Result<PathBuf, crate::DaemonError> {
        match scope {
            WorkflowRegistrySourceScope::Workspace => {
                self.workspace_root
                    .clone()
                    .ok_or(crate::DaemonError::LocalTransport {
                        operation: "workflow_registry.add",
                        message: "workspace workflow registry is unavailable for this session"
                            .to_string(),
                    })
            }
            WorkflowRegistrySourceScope::User => {
                self.user_root
                    .clone()
                    .ok_or(crate::DaemonError::LocalTransport {
                    operation: "workflow_registry.add",
                    message:
                        "user workflow registry is unavailable because ARROBA_HOME/HOME is not set"
                            .to_string(),
                })
            }
            WorkflowRegistrySourceScope::Builtin => Err(crate::DaemonError::LocalTransport {
                operation: "workflow_registry.add",
                message: "builtin workflow registry entries cannot be modified".to_string(),
            }),
        }
    }

    fn root_for_scope(&self, scope: WorkflowRegistrySourceScope) -> Option<&PathBuf> {
        match scope {
            WorkflowRegistrySourceScope::Workspace => self.workspace_root.as_ref(),
            WorkflowRegistrySourceScope::User => self.user_root.as_ref(),
            WorkflowRegistrySourceScope::Builtin => None,
        }
    }
}

impl StoredWorkflowRegistryManifest {
    fn into_metadata(
        self,
        source_scope: WorkflowRegistrySourceScope,
    ) -> WorkflowRegistryEntryMetadata {
        WorkflowRegistryEntryMetadata {
            name: self.name,
            source_scope,
            source_kind: self.source_kind,
            source_path: self.source_path,
            source_sha256: self.source_sha256,
            source_bytes: self.source_bytes,
            definition_sha256: self.definition_sha256,
            created_at_ms: self.created_at_ms,
            updated_at_ms: self.updated_at_ms,
            validation: self.validation,
            summary: self.summary,
            parameters_schema: self.parameters_schema,
        }
    }
}

fn normalize_workflow_registry_input(
    source: WorkflowRegistrySourceInput,
) -> Result<
    (
        WorkflowRegistrySourceKind,
        String,
        Vec<WorkflowCodeSourceExportFile>,
    ),
    crate::DaemonError,
> {
    match source {
        WorkflowRegistrySourceInput::SingleFile {
            source,
            source_path: _,
        } => {
            if source.trim().is_empty() {
                return Err(crate::DaemonError::LocalTransport {
                    operation: "workflow_registry.add",
                    message: "workflow registry source file must not be empty".to_string(),
                });
            }
            let path = "workflow.js".to_string();
            Ok((
                WorkflowRegistrySourceKind::SingleFile,
                path.clone(),
                vec![WorkflowCodeSourceExportFile {
                    sha256: sha256_hex(source.as_bytes()),
                    path,
                    contents: source,
                }],
            ))
        }
        WorkflowRegistrySourceInput::SourceDirectory { files } => {
            if files.is_empty() {
                return Err(crate::DaemonError::LocalTransport {
                    operation: "workflow_registry.add",
                    message: "workflow registry source directory must include files".to_string(),
                });
            }
            let mut normalized = Vec::new();
            let mut source_path = None;
            for file in files {
                let path = normalize_registry_relative_path(&file.path)?;
                let actual_sha = sha256_hex(file.contents.as_bytes());
                if !file.sha256.is_empty() && file.sha256 != actual_sha {
                    return Err(crate::DaemonError::LocalTransport {
                        operation: "workflow_registry.add",
                        message: format!("workflow registry source file `{path}` sha256 mismatch"),
                    });
                }
                if path == "manifest.json" {
                    if let Ok(manifest) =
                        serde_json::from_str::<WorkflowCodeSourceExportManifest>(&file.contents)
                    {
                        source_path =
                            Some(normalize_registry_relative_path(&manifest.source_path)?);
                    }
                }
                normalized.push(WorkflowCodeSourceExportFile {
                    path,
                    contents: file.contents,
                    sha256: actual_sha,
                });
            }
            let source_path = source_path.unwrap_or_else(|| "workflow.js".to_string());
            if !normalized.iter().any(|file| file.path == source_path) {
                return Err(crate::DaemonError::LocalTransport {
                    operation: "workflow_registry.add",
                    message: format!(
                        "workflow registry source directory is missing `{source_path}`"
                    ),
                });
            }
            Ok((
                WorkflowRegistrySourceKind::SourceDirectory,
                source_path,
                normalized,
            ))
        }
    }
}

fn normalize_registry_relative_path(path: &str) -> Result<String, crate::DaemonError> {
    let value = path.trim();
    if value.is_empty() {
        return Err(crate::DaemonError::LocalTransport {
            operation: "workflow_registry.path",
            message: "workflow registry file path must not be empty".to_string(),
        });
    }
    let path = Path::new(value);
    if path.is_absolute() {
        return Err(crate::DaemonError::LocalTransport {
            operation: "workflow_registry.path",
            message: "workflow registry file path must be relative".to_string(),
        });
    }
    if path
        .components()
        .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(crate::DaemonError::LocalTransport {
            operation: "workflow_registry.path",
            message: format!("workflow registry file path `{value}` must stay inside the entry"),
        });
    }
    Ok(path.to_string_lossy().replace('\\', "/"))
}

fn write_registry_file(
    entry_dir: &Path,
    relative_path: &str,
    contents: &str,
) -> Result<(), crate::DaemonError> {
    let relative_path = normalize_registry_relative_path(relative_path)?;
    let path = entry_dir.join(relative_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_error("workflow_registry.write"))?;
    }
    fs::write(path, contents).map_err(io_error("workflow_registry.write"))
}

fn read_workflow_registry_manifest(
    path: &Path,
) -> Result<StoredWorkflowRegistryManifest, crate::DaemonError> {
    let contents = fs::read_to_string(path).map_err(io_error("workflow_registry.read"))?;
    let manifest =
        serde_json::from_str::<StoredWorkflowRegistryManifest>(&contents).map_err(|error| {
            crate::DaemonError::LocalTransport {
                operation: "workflow_registry.read",
                message: format!(
                    "failed to parse workflow registry manifest `{}`: {error}",
                    path.display()
                ),
            }
        })?;
    if manifest.manifest_version != WORKFLOW_REGISTRY_MANIFEST_VERSION {
        return Err(crate::DaemonError::LocalTransport {
            operation: "workflow_registry.read",
            message: format!(
                "unsupported workflow registry manifest version {}; expected {}",
                manifest.manifest_version, WORKFLOW_REGISTRY_MANIFEST_VERSION
            ),
        });
    }
    Ok(manifest)
}

fn write_workflow_registry_manifest(
    path: &Path,
    manifest: &StoredWorkflowRegistryManifest,
) -> Result<(), crate::DaemonError> {
    let payload = serde_json::to_string_pretty(manifest).map_err(|error| {
        crate::DaemonError::LocalTransport {
            operation: "workflow_registry.write",
            message: format!("failed to serialize workflow registry manifest: {error}"),
        }
    })?;
    fs::write(path, format!("{payload}\n")).map_err(io_error("workflow_registry.write"))
}

fn validate_workflow_registry_source_directory_manifest(
    entry_dir: &Path,
    files: &[WorkflowCodeSourceExportFile],
    source_path: &str,
) -> Result<(), crate::DaemonError> {
    let Some(manifest_file) = files.iter().find(|file| file.path == "manifest.json") else {
        return Ok(());
    };
    let manifest = serde_json::from_str::<WorkflowCodeSourceExportManifest>(
        &manifest_file.contents,
    )
    .map_err(|error| crate::DaemonError::LocalTransport {
        operation: "workflow_registry.add",
        message: format!("workflow registry source manifest is invalid: {error}"),
    })?;
    if manifest.manifest_version != WORKFLOW_CODE_SOURCE_EXPORT_MANIFEST_VERSION {
        return Err(crate::DaemonError::LocalTransport {
            operation: "workflow_registry.add",
            message: format!(
                "unsupported workflow-code source manifest version {}; expected {}",
                manifest.manifest_version, WORKFLOW_CODE_SOURCE_EXPORT_MANIFEST_VERSION
            ),
        });
    }
    if normalize_registry_relative_path(&manifest.source_path)? != source_path {
        return Err(crate::DaemonError::LocalTransport {
            operation: "workflow_registry.add",
            message: "workflow registry source manifest source_path mismatch".to_string(),
        });
    }
    let source =
        fs::read(entry_dir.join(source_path)).map_err(io_error("workflow_registry.add"))?;
    if sha256_hex(&source) != manifest.source_sha256 {
        return Err(crate::DaemonError::LocalTransport {
            operation: "workflow_registry.add",
            message: "workflow registry source manifest source_sha256 mismatch".to_string(),
        });
    }
    for path in manifest.schema_paths.values() {
        let path = normalize_registry_relative_path(path)?;
        if !entry_dir.join(&path).is_file() {
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_registry.add",
                message: format!(
                    "workflow registry source manifest references missing schema `{path}`"
                ),
            });
        }
    }
    Ok(())
}

fn validate_workflow_registry_manifest_hashes(
    entry_dir: &Path,
    manifest: &StoredWorkflowRegistryManifest,
) -> Result<(), crate::DaemonError> {
    let source_path = normalize_registry_relative_path(&manifest.source_path)?;
    let source =
        fs::read(entry_dir.join(&source_path)).map_err(io_error("workflow_registry.get"))?;
    let source_len = source.len() as u64;
    if source_len != manifest.source_bytes {
        return Err(crate::DaemonError::LocalTransport {
            operation: "workflow_registry.get",
            message: format!(
                "workflow registry entry `{}` source byte count mismatch",
                manifest.name
            ),
        });
    }
    if sha256_hex(&source) != manifest.source_sha256 {
        return Err(crate::DaemonError::LocalTransport {
            operation: "workflow_registry.get",
            message: format!(
                "workflow registry entry `{}` source sha256 mismatch",
                manifest.name
            ),
        });
    }
    for (relative_path, expected_sha) in &manifest.file_sha256 {
        let relative_path = normalize_registry_relative_path(relative_path)?;
        let bytes =
            fs::read(entry_dir.join(&relative_path)).map_err(io_error("workflow_registry.get"))?;
        if sha256_hex(&bytes) != *expected_sha {
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_registry.get",
                message: format!(
                    "workflow registry entry `{}` file `{relative_path}` sha256 mismatch",
                    manifest.name
                ),
            });
        }
    }
    Ok(())
}

fn workflow_registry_validation_diagnostics(
    validation: &WorkflowCodeValidationReport,
) -> Vec<String> {
    validation
        .diagnostics
        .iter()
        .map(|diagnostic| {
            diagnostic
                .handle
                .as_deref()
                .map(|handle| format!("{}:{handle}", diagnostic.code))
                .unwrap_or_else(|| diagnostic.code.clone())
        })
        .collect()
}

fn single_file_workflow_registry_metadata(
    path: &Path,
    source_scope: WorkflowRegistrySourceScope,
) -> Result<WorkflowRegistryEntryMetadata, crate::DaemonError> {
    let source = fs::read(path).map_err(io_error("workflow_registry.list"))?;
    let name = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("workflow")
        .to_string();
    Ok(WorkflowRegistryEntryMetadata {
        name,
        source_scope,
        source_kind: WorkflowRegistrySourceKind::SingleFile,
        source_path: path.display().to_string(),
        source_sha256: sha256_hex(&source),
        source_bytes: source.len() as u64,
        definition_sha256: None,
        created_at_ms: 0,
        updated_at_ms: 0,
        validation: WorkflowRegistryValidationSummary {
            ok: true,
            diagnostics: Vec::new(),
        },
        summary: None,
        parameters_schema: None,
    })
}

pub(super) fn builtin_workflow_registry_metadata(
    example: &WorkflowCodePatternExample,
) -> WorkflowRegistryEntryMetadata {
    WorkflowRegistryEntryMetadata {
        name: example.slug.to_string(),
        source_scope: WorkflowRegistrySourceScope::Builtin,
        source_kind: WorkflowRegistrySourceKind::SingleFile,
        source_path: example.path.to_string(),
        source_sha256: sha256_hex(example.source.as_bytes()),
        source_bytes: example.source.len() as u64,
        definition_sha256: None,
        created_at_ms: 0,
        updated_at_ms: 0,
        validation: WorkflowRegistryValidationSummary {
            ok: true,
            diagnostics: Vec::new(),
        },
        summary: None,
        parameters_schema: None,
    }
}

pub fn enrich_workflow_registry_entry_summary(
    resolved: WorkflowRegistryResolvedEntry,
    node_path: impl AsRef<Path>,
    limits: &WorkflowCodeLimitsConfig,
) -> WorkflowRegistryEntryMetadata {
    let mut metadata = resolved.metadata;
    if metadata.summary.is_some() {
        return metadata;
    }
    let cache_key = workflow_registry_summary_cache_key(&metadata);
    if let Some(cached) = workflow_registry_summary_cache()
        .lock()
        .expect("workflow registry summary cache mutex poisoned")
        .get(&cache_key)
        .cloned()
    {
        metadata.validation = cached.validation;
        metadata.definition_sha256 = metadata.definition_sha256.or(cached.definition_sha256);
        metadata.summary = cached.summary;
        return metadata;
    }

    let cached = match compile_workflow_code_source_with_schema_import_root(
        node_path,
        &resolved.source,
        WorkflowCodeLanguage::JavaScript,
        limits,
        resolved.schema_import_root.as_deref(),
    ) {
        Ok(compile) => {
            let validation = WorkflowRegistryValidationSummary {
                ok: compile.validation.ok,
                diagnostics: workflow_registry_validation_diagnostics(&compile.validation),
            };
            let definition_sha256 = Some(workflow_code_definition_sha256_hex(&compile.definition));
            let summary = compile
                .validation
                .ok
                .then(|| WorkflowRegistryEntrySummary::from_definition(&compile.definition));
            WorkflowRegistrySummaryCacheEntry {
                validation,
                definition_sha256,
                summary,
                parameters_schema: compile.definition.parameters_schema.clone(),
            }
        }
        Err(error) => {
            let mut diagnostics = metadata.validation.diagnostics.clone();
            diagnostics.push(format!("summary_unavailable: {error}"));
            WorkflowRegistrySummaryCacheEntry {
                validation: WorkflowRegistryValidationSummary {
                    ok: false,
                    diagnostics,
                },
                definition_sha256: metadata.definition_sha256.clone(),
                summary: None,
                parameters_schema: metadata.parameters_schema.clone(),
            }
        }
    };
    let mut cache = workflow_registry_summary_cache()
        .lock()
        .expect("workflow registry summary cache mutex poisoned");
    cache.insert(cache_key, cached.clone());
    if let Some(definition_sha256) = cached.definition_sha256.as_deref() {
        cache.insert(definition_sha256.to_string(), cached.clone());
    }
    drop(cache);

    metadata.validation = cached.validation;
    metadata.definition_sha256 = metadata.definition_sha256.or(cached.definition_sha256);
    metadata.summary = cached.summary;
    metadata.parameters_schema = cached.parameters_schema;
    metadata
}

pub fn workflow_registry_metadata_with_summary_failure(
    mut metadata: WorkflowRegistryEntryMetadata,
    error: impl std::fmt::Display,
) -> WorkflowRegistryEntryMetadata {
    metadata.validation.ok = false;
    metadata
        .validation
        .diagnostics
        .push(format!("summary_unavailable: {error}"));
    metadata
}

fn workflow_registry_summary_cache_key(metadata: &WorkflowRegistryEntryMetadata) -> String {
    metadata
        .definition_sha256
        .clone()
        .unwrap_or_else(|| metadata.source_sha256.clone())
}

fn workflow_registry_summary_cache(
) -> &'static Mutex<BTreeMap<String, WorkflowRegistrySummaryCacheEntry>> {
    static CACHE: OnceLock<Mutex<BTreeMap<String, WorkflowRegistrySummaryCacheEntry>>> =
        OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}
