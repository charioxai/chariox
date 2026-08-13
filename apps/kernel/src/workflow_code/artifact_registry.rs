use super::*;

impl WorkflowCodeArtifactRegistry {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }

    pub fn project_root(workspace: impl AsRef<Path>) -> PathBuf {
        workspace.as_ref().join(".chariox").join("workflow-code")
    }

    pub fn user_root() -> Option<PathBuf> {
        chariox_home().map(|home| home.join("workflow-code"))
    }

    pub fn save(
        &self,
        name: &str,
        language: WorkflowCodeLanguage,
        source: impl Into<String>,
        definition: WorkflowCodeDefinition,
        validation: WorkflowCodeValidationReport,
        actor: WorkflowCodeArtifactActor,
        action: WorkflowCodeArtifactHistoryAction,
    ) -> Result<WorkflowCodeArtifact, crate::DaemonError> {
        validate_registry_name(name, "workflow-code artifact name")?;
        if self.find_path(name)?.is_some() {
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_code.save",
                message: format!("workflow-code artifact `{name}` is already saved"),
            });
        }
        let source = source.into();
        let now = crate::session::unix_epoch_ms();
        let source_sha256 = sha256_hex(source.as_bytes());
        let provenance = WorkflowCodeArtifactProvenance {
            created_by: actor.clone(),
            updated_by: actor.clone(),
        };
        let history = workflow_code_artifact_history(vec![workflow_code_artifact_history_entry(
            action,
            now,
            actor,
            source_sha256.clone(),
            Some(validation.ok),
            None,
            Vec::new(),
        )]);
        let stored = StoredWorkflowCodeArtifact {
            name: name.to_string(),
            language,
            source,
            source_sha256,
            definition,
            validation,
            provenance,
            history,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let path = self.artifact_path(name)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_error("workflow_code.save"))?;
        }
        write_stored_artifact(&path, &stored)?;
        Ok(stored.into_artifact(path))
    }

    pub fn update(
        &self,
        name: &str,
        language: WorkflowCodeLanguage,
        source: impl Into<String>,
        definition: WorkflowCodeDefinition,
        validation: WorkflowCodeValidationReport,
        actor: WorkflowCodeArtifactActor,
        action: WorkflowCodeArtifactHistoryAction,
    ) -> Result<WorkflowCodeArtifact, crate::DaemonError> {
        validate_registry_name(name, "workflow-code artifact name")?;
        let path = self
            .find_path(name)?
            .ok_or_else(|| crate::DaemonError::LocalTransport {
                operation: "workflow_code.update",
                message: format!("workflow-code artifact `{name}` is not saved"),
            })?;
        let previous = read_stored_artifact(&path)?;
        let source = source.into();
        let now = crate::session::unix_epoch_ms();
        let source_sha256 = sha256_hex(source.as_bytes());
        let mut provenance = previous.provenance.clone();
        provenance.updated_by = actor.clone();
        let mut history = previous.history.clone();
        history.push(workflow_code_artifact_history_entry(
            action,
            now,
            actor,
            source_sha256.clone(),
            Some(validation.ok),
            None,
            Vec::new(),
        ));
        let stored = StoredWorkflowCodeArtifact {
            name: name.to_string(),
            language,
            source_sha256,
            source,
            definition,
            validation,
            provenance,
            history: workflow_code_artifact_history(history),
            created_at_ms: previous.created_at_ms,
            updated_at_ms: now,
        };
        write_stored_artifact(&path, &stored)?;
        Ok(stored.into_artifact(path))
    }

    pub fn get(&self, name: &str) -> Result<Option<WorkflowCodeArtifact>, crate::DaemonError> {
        validate_registry_name(name, "workflow-code artifact name")?;
        let Some(path) = self.find_path(name)? else {
            return Ok(None);
        };
        read_stored_artifact(&path).map(|stored| Some(stored.into_artifact(path)))
    }

    pub fn list(&self) -> Result<Vec<WorkflowCodeArtifactMetadata>, crate::DaemonError> {
        let mut entries = BTreeMap::new();
        for root in &self.roots {
            if !root.exists() {
                continue;
            }
            for entry in fs::read_dir(root).map_err(io_error("workflow_code.list"))? {
                let path = entry.map_err(io_error("workflow_code.list"))?.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                    continue;
                }
                let artifact = read_stored_artifact(&path)?.into_artifact(path);
                entries
                    .entry(artifact.metadata.name.clone())
                    .or_insert(artifact.metadata);
            }
        }
        Ok(entries.into_values().collect())
    }

    pub fn delete(&self, name: &str) -> Result<PathBuf, crate::DaemonError> {
        validate_registry_name(name, "workflow-code artifact name")?;
        let path = self
            .find_path(name)?
            .ok_or_else(|| crate::DaemonError::LocalTransport {
                operation: "workflow_code.delete",
                message: format!("workflow-code artifact `{name}` is not saved"),
            })?;
        fs::remove_file(&path).map_err(io_error("workflow_code.delete"))?;
        Ok(path)
    }

    pub fn export_package(
        &self,
        name: &str,
    ) -> Result<WorkflowCodeArtifactPackage, crate::DaemonError> {
        let artifact = self
            .get(name)?
            .ok_or_else(|| crate::DaemonError::LocalTransport {
                operation: "workflow_code.export",
                message: format!("workflow-code artifact `{name}` is not saved"),
            })?;
        Ok(artifact.into_package())
    }

    pub fn export_source(
        &self,
        name: &str,
        format: WorkflowCodeSourceExportFormat,
    ) -> Result<WorkflowCodeSourceExport, crate::DaemonError> {
        let artifact = self
            .get(name)?
            .ok_or_else(|| crate::DaemonError::LocalTransport {
                operation: "workflow_code.source_export",
                message: format!("workflow-code artifact `{name}` is not saved"),
            })?;
        artifact.export_source(format)
    }

    pub fn import_package(
        &self,
        name_override: Option<&str>,
        package: WorkflowCodeArtifactPackage,
        definition: WorkflowCodeDefinition,
        validation: WorkflowCodeValidationReport,
        actor: WorkflowCodeArtifactActor,
        overwrite: bool,
    ) -> Result<WorkflowCodeArtifact, crate::DaemonError> {
        package.validate_integrity()?;
        let name = name_override.unwrap_or(package.name.as_str());
        validate_registry_name(name, "workflow-code artifact name")?;
        let existing = self.get(name)?.is_some();
        if overwrite && existing {
            self.update(
                name,
                package.language,
                package.source,
                definition,
                validation,
                actor,
                WorkflowCodeArtifactHistoryAction::Imported,
            )
        } else {
            self.save(
                name,
                package.language,
                package.source,
                definition,
                validation,
                actor,
                WorkflowCodeArtifactHistoryAction::Imported,
            )
        }
    }

    pub fn record_apply_history(
        &self,
        name: &str,
        actor: WorkflowCodeArtifactActor,
        action: WorkflowCodeArtifactHistoryAction,
        report: &WorkflowCodeApplyReport,
    ) -> Result<WorkflowCodeArtifact, crate::DaemonError> {
        validate_registry_name(name, "workflow-code artifact name")?;
        let path = self
            .find_path(name)?
            .ok_or_else(|| crate::DaemonError::LocalTransport {
                operation: "workflow_code.history",
                message: format!("workflow-code artifact `{name}` is not saved"),
            })?;
        let mut stored = read_stored_artifact(&path)?;
        let now = crate::session::unix_epoch_ms();
        stored.updated_at_ms = now;
        stored.provenance.updated_by = actor.clone();
        stored.history.push(workflow_code_artifact_history_entry(
            action,
            now,
            actor,
            stored.source_sha256.clone(),
            None,
            Some(report.workflow_id.clone()),
            report.warnings.clone(),
        ));
        stored.history = workflow_code_artifact_history(stored.history);
        write_stored_artifact(&path, &stored)?;
        Ok(stored.into_artifact(path))
    }

    fn artifact_path(&self, name: &str) -> Result<PathBuf, crate::DaemonError> {
        Ok(self.primary_root()?.join(format!("{name}.json")))
    }

    fn find_path(&self, name: &str) -> Result<Option<PathBuf>, crate::DaemonError> {
        for root in &self.roots {
            let path = root.join(format!("{name}.json"));
            if path.exists() {
                return Ok(Some(path));
            }
        }
        Ok(None)
    }

    fn primary_root(&self) -> Result<&PathBuf, crate::DaemonError> {
        self.roots.first().ok_or(crate::DaemonError::InvalidConfig {
            field: "workflow-code registry roots",
            message: "must include at least one root",
        })
    }
}

impl WorkflowCodeArtifact {
    pub fn into_package(self) -> WorkflowCodeArtifactPackage {
        let definition_sha256 = workflow_code_definition_sha256_hex(&self.definition);
        WorkflowCodeArtifactPackage {
            package_version: WORKFLOW_CODE_ARTIFACT_PACKAGE_VERSION,
            name: self.metadata.name,
            language: self.metadata.language,
            source: self.source,
            source_sha256: self.metadata.source_sha256,
            source_bytes: self.metadata.source_bytes,
            definition_sha256,
            definition: self.definition,
            validation: self.metadata.validation,
            exported_at_ms: crate::session::unix_epoch_ms(),
        }
    }

    pub fn export_source(
        &self,
        format: WorkflowCodeSourceExportFormat,
    ) -> Result<WorkflowCodeSourceExport, crate::DaemonError> {
        let definition_sha256 = workflow_code_definition_sha256_hex(&self.definition);
        match format {
            WorkflowCodeSourceExportFormat::Inline => {
                let source_sha256 = sha256_hex(self.source.as_bytes());
                Ok(WorkflowCodeSourceExport {
                    name: self.metadata.name.clone(),
                    language: self.metadata.language,
                    format,
                    source_path: "workflow.js".to_string(),
                    source: self.source.clone(),
                    source_sha256,
                    source_bytes: self.source.len() as u64,
                    definition_sha256,
                    files: Vec::new(),
                })
            }
            WorkflowCodeSourceExportFormat::Directory => export_workflow_code_source_directory(
                &self.metadata.name,
                &self.definition,
                definition_sha256,
            ),
        }
    }
}

impl WorkflowCodeArtifactPackage {
    pub fn validate_integrity(&self) -> Result<(), crate::DaemonError> {
        if self.package_version != WORKFLOW_CODE_ARTIFACT_PACKAGE_VERSION {
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_code.import",
                message: format!(
                    "unsupported workflow-code package version {}; expected {}",
                    self.package_version, WORKFLOW_CODE_ARTIFACT_PACKAGE_VERSION
                ),
            });
        }
        validate_registry_name(&self.name, "workflow-code artifact package name")?;
        let source_bytes = self.source.len() as u64;
        if source_bytes != self.source_bytes {
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_code.import",
                message: format!(
                    "workflow-code package source byte count mismatch: declared {}, actual {source_bytes}",
                    self.source_bytes
                ),
            });
        }
        let source_sha256 = sha256_hex(self.source.as_bytes());
        if source_sha256 != self.source_sha256 {
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_code.import",
                message: "workflow-code package source sha256 mismatch".to_string(),
            });
        }
        let definition_sha256 = workflow_code_definition_sha256_hex(&self.definition);
        if definition_sha256 != self.definition_sha256 {
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_code.import",
                message: "workflow-code package definition sha256 mismatch".to_string(),
            });
        }
        Ok(())
    }
}

impl StoredWorkflowCodeArtifact {
    fn into_artifact(self, path: PathBuf) -> WorkflowCodeArtifact {
        let source_bytes = self.source.len() as u64;
        WorkflowCodeArtifact {
            metadata: WorkflowCodeArtifactMetadata {
                name: self.name,
                language: self.language,
                path,
                source_sha256: self.source_sha256,
                source_bytes,
                validation: self.validation,
                provenance: self.provenance,
                history: self.history,
                created_at_ms: self.created_at_ms,
                updated_at_ms: self.updated_at_ms,
            },
            source: self.source,
            definition: self.definition,
        }
    }
}

fn read_stored_artifact(path: &Path) -> Result<StoredWorkflowCodeArtifact, crate::DaemonError> {
    let contents = fs::read_to_string(path).map_err(io_error("workflow_code.read"))?;
    serde_json::from_str(&contents).map_err(|error| crate::DaemonError::LocalTransport {
        operation: "workflow_code.read",
        message: format!(
            "failed to parse workflow-code artifact `{}`: {error}",
            path.display()
        ),
    })
}

fn workflow_code_artifact_history_entry(
    action: WorkflowCodeArtifactHistoryAction,
    at_ms: u64,
    actor: WorkflowCodeArtifactActor,
    source_sha256: String,
    validation_ok: Option<bool>,
    workflow_id: Option<String>,
    warnings: Vec<WorkflowCodeApplyWarning>,
) -> WorkflowCodeArtifactHistoryEntry {
    WorkflowCodeArtifactHistoryEntry {
        action,
        at_ms,
        actor,
        source_sha256,
        validation_ok,
        workflow_id,
        warnings,
    }
}

fn workflow_code_artifact_history(
    mut history: Vec<WorkflowCodeArtifactHistoryEntry>,
) -> Vec<WorkflowCodeArtifactHistoryEntry> {
    if history.len() > WORKFLOW_CODE_ARTIFACT_HISTORY_LIMIT {
        history.drain(0..history.len() - WORKFLOW_CODE_ARTIFACT_HISTORY_LIMIT);
    }
    history
}

fn write_stored_artifact(
    path: &Path,
    artifact: &StoredWorkflowCodeArtifact,
) -> Result<(), crate::DaemonError> {
    let payload = serde_json::to_string_pretty(artifact).map_err(|error| {
        crate::DaemonError::LocalTransport {
            operation: "workflow_code.write",
            message: format!("failed to serialize workflow-code artifact: {error}"),
        }
    })?;
    fs::write(path, format!("{payload}\n")).map_err(io_error("workflow_code.write"))
}
