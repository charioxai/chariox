use super::*;

#[test]
fn source_export_keeps_component_variable_namespaces_separate() {
    let mut definition = minimal_definition();
    definition.schemas[0].handle = "entry".to_string();
    definition.schemas[0].alias = Some("Entry schema".to_string());
    definition.workflow.run_output_schema = Some("entry".to_string());

    let inline = workflow_code_definition_to_javascript(&definition, None)
        .expect("inline source export should serialize");
    assert!(inline.contains("const schema_entry = workflow.schema"));
    assert!(inline.contains("const endpoint_entry = workflow.endpoint"));
    assert!(inline.contains("runOutputSchema: schema_entry"));

    let directory = export_workflow_code_source_from_definition(
        "entry-collision",
        &definition,
        WorkflowCodeSourceExportFormat::Directory,
    )
    .expect("source directory export should serialize");
    assert!(directory
        .source
        .contains("const schema_entry = workflow.schemaFromFile"));
    assert!(directory
        .source
        .contains("const endpoint_entry = workflow.endpoint"));
    assert!(directory.source.contains("runOutputSchema: schema_entry"));
}

#[test]
fn workflow_registry_lists_and_resolves_builtin_entries() {
    let root = std::env::temp_dir().join(format!(
        "arroba-workflow-registry-builtin-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let registry = WorkflowRegistry::new(Some(root.join("workspace")), Some(root.join("user")));

    let entries = registry
        .list()
        .expect("builtin workflow registry entries should list");
    let slugs: Vec<_> = entries.iter().map(|entry| entry.name.as_str()).collect();
    for expected in [
        "prompt-chaining",
        "routing",
        "fan-out-synthesize",
        "parallelization",
        "adversarial-verification",
        "generate-filter",
        "tournament",
        "loop-until-done",
        "orchestrator-workers",
        "evaluator-optimizer",
    ] {
        assert!(
            slugs.contains(&expected),
            "builtin workflow registry should include {expected}"
        );
    }

    let resolved = registry
        .resolve("prompt-chaining")
        .expect("builtin workflow registry entry should resolve");
    assert_eq!(
        resolved.metadata.source_scope,
        WorkflowRegistrySourceScope::Builtin
    );
    assert_eq!(
        resolved.metadata.source_kind,
        WorkflowRegistrySourceKind::SingleFile
    );
    assert!(resolved.source.contains("workflow.define"));

    let error = registry
        .delete("prompt-chaining", None)
        .expect_err("builtin registry entries must not be deleted");
    assert!(format!("{error}").contains("builtin workflow registry entry"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn workflow_registry_enriches_builtin_summary_and_keeps_invalid_entry_metadata() {
    let Some(node) = find_node() else {
        eprintln!("skipping workflow registry summary test because node is not available");
        return;
    };
    let example = WORKFLOW_CODE_PATTERN_EXAMPLES
        .iter()
        .find(|example| example.slug == "prompt-chaining")
        .expect("prompt-chaining builtin should exist");
    let enriched = enrich_workflow_registry_entry_summary(
        WorkflowRegistryResolvedEntry {
            metadata: builtin_workflow_registry_metadata(example),
            source: example.source.to_string(),
            node_path: example.path.to_string(),
            schema_import_root: None,
        },
        &node,
        &WorkflowCodeLimitsConfig::default(),
    );
    let summary = enriched.summary.expect("builtin summary should compile");
    assert_eq!(summary.endpoints, vec!["entry"]);
    assert_eq!(summary.default_endpoint.as_deref(), Some("entry"));
    assert!(summary.nodes.contains(&"drafter".to_string()));

    let invalid = enrich_workflow_registry_entry_summary(
        WorkflowRegistryResolvedEntry {
            metadata: WorkflowRegistryEntryMetadata {
                name: "broken".to_string(),
                source_scope: WorkflowRegistrySourceScope::Workspace,
                source_kind: WorkflowRegistrySourceKind::SingleFile,
                source_path: "broken.js".to_string(),
                source_sha256: sha256_hex(b"not valid workflow code"),
                source_bytes: 23,
                definition_sha256: None,
                created_at_ms: 0,
                updated_at_ms: 0,
                validation: WorkflowRegistryValidationSummary {
                    ok: true,
                    diagnostics: Vec::new(),
                },
                summary: None,
                parameters_schema: None,
            },
            source: "not valid workflow code".to_string(),
            node_path: "broken.js".to_string(),
            schema_import_root: None,
        },
        &node,
        &WorkflowCodeLimitsConfig::default(),
    );
    assert_eq!(invalid.name, "broken");
    assert!(!invalid.validation.ok);
    assert!(invalid.summary.is_none());
    assert!(invalid
        .validation
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.starts_with("summary_unavailable:")));
}

#[test]
fn workflow_registry_applies_workspace_user_builtin_precedence() {
    let Some(node) = find_node() else {
        eprintln!("skipping workflow registry precedence test because node is not available");
        return;
    };
    let root = std::env::temp_dir().join(format!(
        "arroba-workflow-registry-precedence-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let registry = WorkflowRegistry::new(Some(root.join("workspace")), Some(root.join("user")));
    let source = workflow_code_definition_to_javascript(&minimal_definition(), None)
        .expect("workflow-code source should serialize");
    let node_path = node.to_string_lossy().to_string();

    registry
        .add(
            "prompt-chaining",
            WorkflowRegistrySourceScope::User,
            WorkflowRegistrySourceInput::SingleFile {
                source: source.clone(),
                source_path: Some("user.js".to_string()),
            },
            &node_path,
            &WorkflowCodeLimitsConfig::default(),
        )
        .expect("user registry entry should add");
    registry
        .add(
            "prompt-chaining",
            WorkflowRegistrySourceScope::Workspace,
            WorkflowRegistrySourceInput::SingleFile {
                source,
                source_path: Some("workspace.js".to_string()),
            },
            &node_path,
            &WorkflowCodeLimitsConfig::default(),
        )
        .expect("workspace registry entry should add");

    let resolved = registry
        .resolve("prompt-chaining")
        .expect("shadowed registry entry should resolve");
    assert_eq!(
        resolved.metadata.source_scope,
        WorkflowRegistrySourceScope::Workspace
    );
    assert_eq!(resolved.metadata.source_path, "workflow.js");

    registry
        .delete(
            "prompt-chaining",
            Some(WorkflowRegistrySourceScope::Workspace),
        )
        .expect("workspace registry entry should delete");
    let resolved = registry
        .resolve("prompt-chaining")
        .expect("user registry entry should resolve after workspace delete");
    assert_eq!(
        resolved.metadata.source_scope,
        WorkflowRegistrySourceScope::User
    );

    registry
        .delete("prompt-chaining", Some(WorkflowRegistrySourceScope::User))
        .expect("user registry entry should delete");
    let resolved = registry
        .resolve("prompt-chaining")
        .expect("builtin registry entry should resolve after user delete");
    assert_eq!(
        resolved.metadata.source_scope,
        WorkflowRegistrySourceScope::Builtin
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn workflow_registry_adds_source_directory_and_rejects_hash_mismatch() {
    let Some(node) = find_node() else {
        eprintln!("skipping workflow registry source directory test because node is not available");
        return;
    };
    let root = std::env::temp_dir().join(format!(
        "arroba-workflow-registry-directory-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let workspace_root = root.join("workspace");
    let registry = WorkflowRegistry::new(Some(workspace_root.clone()), Some(root.join("user")));
    let node_path = node.to_string_lossy().to_string();
    let export = export_workflow_code_source_from_definition(
        "directory-flow",
        &multi_endpoint_definition(),
        WorkflowCodeSourceExportFormat::Directory,
    )
    .expect("workflow-code source directory should export");

    let added = registry
        .add_from_export(
            "directory-flow",
            WorkflowRegistrySourceScope::Workspace,
            export.clone(),
            &node_path,
            &WorkflowCodeLimitsConfig::default(),
        )
        .expect("source directory registry entry should add");
    assert_eq!(
        added.source_kind,
        WorkflowRegistrySourceKind::SourceDirectory
    );
    assert!(added.definition_sha256.is_some());
    let summary = added.summary.expect("added entry should include summary");
    assert_eq!(summary.endpoints, vec!["entry", "review"]);
    assert_eq!(summary.queues, vec!["default", "urgent"]);
    assert_eq!(summary.default_endpoint.as_deref(), Some("entry"));

    let resolved = registry
        .resolve("directory-flow")
        .expect("source directory registry entry should resolve");
    assert!(resolved.metadata.summary.is_some());
    assert!(resolved.schema_import_root.is_some());
    let recompiled = compile_workflow_code_source_with_schema_import_root(
        &node,
        &resolved.source,
        WorkflowCodeLanguage::JavaScript,
        &WorkflowCodeLimitsConfig::default(),
        resolved.schema_import_root.as_deref(),
    )
    .expect("resolved source directory registry entry should compile");
    assert!(
        recompiled.validation.ok,
        "{:?}",
        recompiled.validation.diagnostics
    );
    assert_eq!(
        recompiled.definition.workflow.run_output_schema.as_deref(),
        Some("final")
    );

    fs::write(
        workspace_root.join("directory-flow").join("workflow.js"),
        "workflow.define({ alias: 'tampered' })\n",
    )
    .expect("registry source should tamper");
    let error = registry
        .resolve("directory-flow")
        .expect_err("tampered registry entry should fail hash validation");
    let message = format!("{error}");
    assert!(
        message.contains("sha256 mismatch") || message.contains("byte count mismatch"),
        "{message}"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn registry_saves_lists_reads_updates_and_deletes_artifacts() {
    let root = std::env::temp_dir().join(format!(
        "arroba-workflow-code-registry-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let registry = WorkflowCodeArtifactRegistry::new(vec![root.clone()]);
    let definition = minimal_definition();
    let validation = definition.validate_with_limits(&WorkflowCodeLimitsConfig::default());
    let creator = WorkflowCodeArtifactActor::new("user-1", None);
    let updater = WorkflowCodeArtifactActor::new("user-2", Some("meta-1".to_string()));

    let created = registry
        .save(
            "toy",
            WorkflowCodeLanguage::JavaScript,
            "workflow.define({ alias: 'toy' })",
            definition.clone(),
            validation,
            creator.clone(),
            WorkflowCodeArtifactHistoryAction::Created,
        )
        .expect("workflow-code artifact should save");

    assert_eq!(created.metadata.name, "toy");
    assert_eq!(created.metadata.language, WorkflowCodeLanguage::JavaScript);
    assert_eq!(created.metadata.source_bytes, 33);
    assert!(created.metadata.validation.ok);
    assert_eq!(created.metadata.provenance.created_by, creator);
    assert_eq!(created.metadata.provenance.updated_by, creator);
    assert_eq!(created.metadata.history.len(), 1);
    assert_eq!(
        created.metadata.history[0].action,
        WorkflowCodeArtifactHistoryAction::Created
    );
    assert_eq!(created.metadata.history[0].validation_ok, Some(true));
    assert_eq!(registry.list().expect("list should load").len(), 1);

    let loaded = registry
        .get("toy")
        .expect("get should succeed")
        .expect("artifact should exist");
    assert_eq!(loaded.source, "workflow.define({ alias: 'toy' })");
    assert_eq!(loaded.definition, definition);

    let updated = registry
        .update(
            "toy",
            WorkflowCodeLanguage::TypeScript,
            "workflow.define({ alias: 'toy-2' })",
            minimal_definition(),
            minimal_definition().validate_with_limits(&WorkflowCodeLimitsConfig::default()),
            updater.clone(),
            WorkflowCodeArtifactHistoryAction::Updated,
        )
        .expect("workflow-code artifact should update");
    assert_eq!(updated.metadata.language, WorkflowCodeLanguage::TypeScript);
    assert_eq!(
        updated.metadata.created_at_ms,
        created.metadata.created_at_ms
    );
    assert!(updated.metadata.updated_at_ms >= created.metadata.updated_at_ms);
    assert_eq!(updated.metadata.provenance.created_by, creator);
    assert_eq!(updated.metadata.provenance.updated_by, updater);
    assert_eq!(updated.metadata.history.len(), 2);
    assert_eq!(
        updated.metadata.history[1].action,
        WorkflowCodeArtifactHistoryAction::Updated
    );

    let deleted_path = registry.delete("toy").expect("artifact should delete");
    assert!(!deleted_path.exists());
    assert!(registry.get("toy").expect("get should succeed").is_none());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn registry_persists_validation_report_for_invalid_artifact() {
    let root = std::env::temp_dir().join(format!(
        "arroba-workflow-code-invalid-registry-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let registry = WorkflowCodeArtifactRegistry::new(vec![root.clone()]);
    let mut definition = minimal_definition();
    definition.endpoints.clear();
    let validation = definition.validate_with_limits(&WorkflowCodeLimitsConfig::default());
    let actor = WorkflowCodeArtifactActor::new("user-1", None);

    let artifact = registry
        .save(
            "invalid",
            WorkflowCodeLanguage::JavaScript,
            "workflow.define({ alias: 'invalid' })",
            definition,
            validation,
            actor,
            WorkflowCodeArtifactHistoryAction::Created,
        )
        .expect("invalid workflow-code artifact should still save diagnostics");

    assert!(!artifact.metadata.validation.ok);
    assert!(artifact
        .metadata
        .validation
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "missing_endpoint"));
    assert_eq!(artifact.metadata.history[0].validation_ok, Some(false));

    let _ = fs::remove_dir_all(root);
}
