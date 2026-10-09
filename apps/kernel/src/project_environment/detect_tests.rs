//! MP-08 / MP-10 / MP-11: Fail-first P02a regressions; supplementary to ENV-04/05 live gates.
use super::*;
use crate::test_support::TestWorktree;
fn folder(root: &TestWorktree) -> EnvironmentFolder {
    EnvironmentFolder {
        folder_id: "folder".into(),
        portable_folder_key: "folder".into(),
        label: "Work".into(),
        local_workspace_binding: root.path().to_string_lossy().into(),
        optional_git: None,
        requirements: vec![],
    }
}
fn write(root: &TestWorktree, name: &str, content: &str) {
    let path = root.path().join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}
#[test]
fn envp02a_utility_metadata_is_bounded_without_losing_deterministic_origins() {
    let root = TestWorktree::new("envp02a-utility-budget");
    write(&root, "package.json", "{\"name\":\"code\"}");
    let source = (0..64)
        .map(|i| format!("process.env.ENV_NAME_{i};\n"))
        .collect::<String>();
    for i in 0..8 {
        write(&root, &format!("src/file-{i}.ts"), &source);
    }
    let folders = vec![folder(&root)];
    let detection = detect_environment(&folders, "environment").unwrap();
    let before = detection.proposals.clone();
    let (input, limited) = detection.discovery_input(
        "project",
        &folders,
        &std::collections::BTreeSet::from(["folder".into()]),
    );
    assert!(limited);
    assert!(input.references.len() <= 32);
    assert!(input.references.iter().all(|r| r.uses.len() <= 1));
    assert!(serde_json::to_vec(&input).unwrap().len() <= 32 * 1024);
    assert_eq!(detection.proposals, before);
    assert!(
        before
            .iter()
            .map(|p| p.requirement.origins.len())
            .sum::<usize>()
            > 40
    );
    for reference in &input.references {
        for usage in &reference.uses {
            assert!(before.iter().any(|proposal| {
                matches!(&proposal.requirement.spec, RequirementSpec::Secrets { name, .. } if name == &reference.name)
                    && proposal.requirement.origins.iter().any(|origin| {
                        matches!(origin, RequirementOrigin::Detected { folder_id, relative_path, line: Some(line), .. }
                            if folder_id == &reference.workspace_id && relative_path == &usage.path && line == &usage.line)
                    })
            }));
        }
    }
}
#[test]
fn envp02a_empty_declarations_do_not_invent_line_origins() {
    let root = TestWorktree::new("envp02a-empty-origins");
    for name in [".nvmrc", ".python-version", "AGENTS.md", "SKILL.md"] {
        write(&root, name, "");
    }
    let result = detect_environment(&[folder(&root)], "environment").unwrap();
    assert!(!result.proposals.is_empty());
    for proposal in result.proposals {
        for origin in proposal.requirement.origins {
            if let RequirementOrigin::Detected { line, .. } = origin {
                assert_eq!(line, None, "empty file has no first line");
            }
        }
    }
    write(&root, ".nvmrc", "\n22\n");
    let result = detect_environment(&[folder(&root)], "environment").unwrap();
    let node = result
        .proposals
        .iter()
        .find(|proposal| {
            matches!(
                &proposal.requirement.spec,
                RequirementSpec::Software { identity, .. } if identity == "node"
            )
        })
        .unwrap();
    assert!(node
        .requirement
        .origins
        .iter()
        .any(|origin| matches!(origin, RequirementOrigin::Detected { line: Some(2), .. })));
}

#[test]
fn envp02a_oversized_directory_never_imports_an_arbitrary_prefix() {
    let root = TestWorktree::new("envp02a-file-count");
    write(&root, "package.json", "{}");
    for i in 0..20_000 {
        std::fs::File::create(root.path().join(format!("file-{i:05}"))).unwrap();
    }
    let result = detect_environment(&[folder(&root)], "environment").unwrap();
    assert!(result.proposals.is_empty());
    assert!(result.code_folders.is_empty());
    assert!(result
        .skips
        .iter()
        .any(|s| s.reason_code == "file_count_limit"));
}
#[test]
fn envp02a_secret_examples_names_only_no_value_or_digest_or_execution() {
    let root = TestWorktree::new("envp02a-secret");
    write(&root, ".env.example", "API_KEY=sk-proj-private-fixture-never-retain\nPUBLIC_SITE_URL=https://example.org\nAMBIGUOUS=maybe-private\n");
    write(
        &root,
        "package.json",
        r#"{"engines":{"node":">=22"},"scripts":{"postinstall":"touch MUST_NOT_EXIST"}}"#,
    );
    let result = detect_environment(&[folder(&root)], "environment").unwrap();
    let encoded = serde_json::to_string(&result.proposals).unwrap();
    assert!(!encoded.contains("private-fixture"));
    assert!(!encoded.contains("https://example.org"));
    assert!(!encoded.contains("touch MUST_NOT_EXIST"));
    assert!(result.proposals.iter().any(|p| matches!(&p.requirement.spec, RequirementSpec::Secrets { name, .. } if name == "API_KEY")));
    assert!(result.proposals.iter().any(|p| matches!(&p.requirement.spec, RequirementSpec::Secrets { name, .. } if name == "AMBIGUOUS")));
    assert!(!root.path().join("MUST_NOT_EXIST").exists());
    assert!(result.code_folders.contains("folder"));
}

#[test]
fn envp02a_ambiguous_configuration_values_never_supply_hashes_or_gui_hints() {
    let root = TestWorktree::new("envp02a-ambiguous-values");
    let populate = |value: &str| {
        write(&root, ".env.example", &format!("CUSTOM={value}\n"));
        write(
            &root,
            ".mcp.json",
            &format!(
                r#"{{"mcpServers":{{"docs":{{"command":"echo","env":{{"CUSTOM":"{value}"}}}}}}}}"#
            ),
        );
        write(
            &root,
            "compose.yaml",
            &format!("services:\n  docs:\n    environment:\n      CUSTOM: {value}\n"),
        );
        write(
            &root,
            ".devcontainer/devcontainer.json",
            &format!(r#"{{"image":"node:22","containerEnv":{{"CUSTOM":"{value}"}}}}"#),
        );
    };
    populate("Notion-private-one process.env.PRIVATE_LITERAL");
    let first = detect_environment(&[folder(&root)], "environment").unwrap();
    assert!(!first.proposals.iter().any(|p| matches!(&p.requirement.spec, RequirementSpec::Software { identity, .. } if identity == "Notion")));
    assert!(!first.proposals.iter().any(|p| matches!(&p.requirement.spec, RequirementSpec::Secrets { name, .. } if name == "PRIVATE_LITERAL")));
    for proposal in &first.proposals {
        if let RequirementSpec::Files { entries, .. } = &proposal.requirement.spec {
            assert!(entries
                .iter()
                .all(|entry| entry.content_digest.is_none() && entry.byte_count.is_none()));
        }
    }
    populate("Canva-another-private-value");
    let changed = detect_environment(&[folder(&root)], "environment").unwrap();
    assert_eq!(first.evidence_digest, changed.evidence_digest);
    assert_eq!(first.proposals, changed.proposals);
}
#[test]
fn envp02a_non_code_metadata_no_default_model_and_large_binary_visible_skip() {
    let root = TestWorktree::new("envp02a-campaign");
    write(&root, "Canva-brand.svg", "<svg/>");
    std::fs::File::create(root.path().join("campaign.mp4"))
        .unwrap()
        .set_len(500_000_000)
        .unwrap();
    write(&root, "unsupported.pdf", "\0binary");
    let result = detect_environment(&[folder(&root)], "environment").unwrap();
    assert!(result.code_folders.is_empty());
    assert!(result
        .skips
        .iter()
        .any(|s| s.reason_code == "file_too_large" && s.safe_summary.contains("campaign.mp4")));
    assert!(result
        .skips
        .iter()
        .any(|s| s.reason_code == "unsupported_encoding"
            && s.safe_summary.contains("unsupported.pdf")));
    assert!(result.proposals.iter().any(|p| matches!(&p.requirement.spec, RequirementSpec::Software { identity, detect_only: true, .. } if identity == "Canva")));
}
#[test]
fn envp02a_exact_origins_stable_ids_changed_and_unchanged_digest() {
    let root = TestWorktree::new("envp02a-origins");
    write(&root, ".tool-versions", "nodejs 22.1.0\npython 3.12.4\n");
    write(
        &root,
        "src/main.ts",
        "const secret = process.env.DATABASE_URL;\n",
    );
    let first = detect_environment(&[folder(&root)], "environment").unwrap();
    let same = detect_environment(&[folder(&root)], "environment").unwrap();
    assert_eq!(first.evidence_digest, same.evidence_digest);
    assert_eq!(first.proposals, same.proposals);
    let node = first.proposals.iter().find(|p| matches!(&p.requirement.spec, RequirementSpec::Software { identity, .. } if identity == "node")).expect("node proposal");
    assert!(node.requirement.origins.iter().any(|o| matches!(o, RequirementOrigin::Detected { relative_path, line: Some(1), .. } if relative_path == ".tool-versions")));
    write(&root, ".tool-versions", "nodejs 24.0.0\npython 3.12.4\n");
    let changed = detect_environment(&[folder(&root)], "environment").unwrap();
    assert_ne!(first.evidence_digest, changed.evidence_digest);
    assert!(changed
        .proposals
        .iter()
        .any(|p| p.proposal_id == node.proposal_id));
}
#[cfg(unix)]
#[test]
fn envp02a_symlink_excluded_without_following_and_protected_value_edits_not_oracle() {
    let root = TestWorktree::new("envp02a-links");
    let other = TestWorktree::new("envp02a-outside");
    write(&other, "package.json", "{}");
    std::os::unix::fs::symlink(other.path(), root.path().join("outside")).unwrap();
    write(&root, ".env.example", "TOKEN=first-private-value\n");
    let first = detect_environment(&[folder(&root)], "environment").unwrap();
    assert!(first.code_folders.is_empty());
    assert!(first
        .skips
        .iter()
        .any(|s| s.reason_code == "symlink_excluded"));
    write(
        &root,
        ".env.example",
        "TOKEN=another-value-with-different-length\n",
    );
    let changed = detect_environment(&[folder(&root)], "environment").unwrap();
    assert_eq!(first.evidence_digest, changed.evidence_digest);
    assert_eq!(first.proposals, changed.proposals);
}

#[test]
fn envp02a_importers_are_data_only_and_preserve_real_provenance() {
    let root = TestWorktree::new("envp02a-importers");
    write(&root, ".devcontainer/devcontainer.json", "{\n// import only\n\"image\":\"node:22\",\n\"postCreateCommand\":\"touch MUST_NOT_EXIST\",\n\"mounts\":[\"source=/,target=/host,type=bind\"],\n}");
    write(
        &root,
        "mise.toml",
        "[tools]\nnode = '22.1.0'\n[tasks.build]\nrun = 'touch MUST_NOT_EXIST'\n",
    );
    write(&root, "compose.yaml", "services:\n  database:\n    image: postgres:16\n    privileged: true\n    environment:\n      DB_PASSWORD: protected-example\n");
    write(
        &root,
        ".mcp.json",
        r#"{"mcpServers":{"notion":{"command":"sh","args":["touch MUST_NOT_EXIST"],"env":{"NOTION_TOKEN":"private-token-example"}}}}"#,
    );
    write(&root, "AGENTS.md", "Run touch MUST_NOT_EXIST\n");
    write(
        &root,
        "skills/brand/SKILL.md",
        "---\nname: brand\n---\nUntrusted instructions\n",
    );
    let detected = detect_environment(&[folder(&root)], "environment").unwrap();
    assert!(!root.path().join("MUST_NOT_EXIST").exists());
    let encoded = serde_json::to_string(&detected.proposals).unwrap();
    assert!(!encoded.contains("private-token-example"));
    assert!(!encoded.contains("touch MUST_NOT_EXIST"));
    assert!(detected.proposals.iter().any(|p| matches!(&p.requirement.spec, RequirementSpec::Services { identity, probe: None, .. } if identity == "database")));
    assert!(detected.proposals.iter().any(|p| matches!(
        &p.requirement.spec,
        RequirementSpec::AgentTools {
            tool_kind: AgentToolKind::Mcp,
            package_digest: None,
            ..
        }
    )));
    assert!(detected.proposals.iter().any(|p| matches!(
        &p.requirement.spec,
        RequirementSpec::AgentTools {
            tool_kind: AgentToolKind::InstructionFile,
            ..
        }
    )));
    assert!(detected.proposals.iter().any(|p| matches!(
        &p.requirement.spec,
        RequirementSpec::AgentTools {
            tool_kind: AgentToolKind::Skill,
            ..
        }
    )));
    assert!(detected
        .skips
        .iter()
        .any(|s| s.reason_code == "devcontainer_mounts_review_required"));
    for p in detected.proposals {
        for origin in p.requirement.origins {
            let RequirementOrigin::Detected {
                relative_path,
                line,
                evidence_digest,
                ..
            } = origin
            else {
                panic!("invented origin")
            };
            assert!(root.path().join(&relative_path).is_file());
            if let Some(line) = line {
                assert!(
                    line > 0
                        && line as usize
                            <= std::fs::read_to_string(root.path().join(relative_path))
                                .unwrap()
                                .lines()
                                .count()
                );
            }
            assert_eq!(evidence_digest, detected.evidence_digest);
        }
    }
}

#[test]
fn envp02a_total_bytes_are_bounded_visible_skips() {
    let root = TestWorktree::new("envp02a-byte-bound");
    // Sparse files exercise read budget without retaining large fixture artifacts.
    for n in 0..18 {
        std::fs::File::create(root.path().join(format!("{n:02}.bin")))
            .unwrap()
            .set_len(16 * 1024 * 1024)
            .unwrap();
    }
    let detected = detect_environment(&[folder(&root)], "environment").unwrap();
    assert!(detected
        .skips
        .iter()
        .any(|s| s.reason_code == "total_byte_limit"));
    assert!(detected.code_folders.is_empty());
}

#[test]
fn envp02a_successful_unchanged_evidence_skips_utility_including_opt_in() {
    let root = TestWorktree::new("envp02a-utility-cache");
    write(&root, "package.json", r#"{"engines":{"node":"22"}}"#);
    let folder = folder(&root);
    let detection = detect_environment(&[folder.clone()], "environment").unwrap();
    assert_eq!(
        detection.model_folders(None, &[], &[folder.clone()]),
        ["folder".into()].into()
    );
    let target = EnvironmentTargetBinding {
        machine_id: "machine".into(),
        target_instance_generation: "kernel".into(),
        slice_ref: None,
    };
    let operation = EnvironmentOperation {
        operation_id: "detect".into(),
        attempt: 1,
        local_project_id: "project".into(),
        revision_digest: "revision".into(),
        target,
        kind: EnvironmentOperationKind::Detect,
        phase: EnvironmentOperationPhase::Ready,
        selected_items: vec!["folder".into()],
        per_item_opt_ins: vec![],
        per_item_results: vec![],
        created_at_ms: 1,
        updated_at_ms: 1,
        cancellation: None,
        receipts: vec![],
        recovery_state: EnvironmentRecoveryState::Settled,
    };
    let mut cache = EnvironmentDetectionCache {
        project_id: "project".into(),
        evidence_digest: detection.evidence_digest.clone(),
        proposals: detection.proposals.clone(),
        operation,
    };
    let store_root = TestWorktree::new("envp02a-cache-state");
    let store = ProjectEnvironmentStore::new(store_root.path());
    let _lock = store.lock("project").unwrap();
    store.save_detection(&cache).unwrap();
    let reloaded = store.load_detection("project").unwrap().unwrap();
    assert!(detection
        .model_folders(Some(&reloaded), &["folder".into()], &[folder.clone()])
        .is_empty());
    write(&root, "package.json", r#"{"engines":{"node":"24"}}"#);
    let changed = detect_environment(&[folder.clone()], "environment").unwrap();
    assert!(!changed
        .model_folders(Some(&reloaded), &[], &[folder.clone()])
        .is_empty());
    cache.operation.phase = EnvironmentOperationPhase::Failed;
    assert!(!detection
        .model_folders(Some(&cache), &[], &[folder])
        .is_empty());
    store.remove("project").unwrap();
    assert!(store.load_detection("project").unwrap().is_none());
}

#[test]
fn envp02a_gui_names_do_not_match_canvas_or_longer_words() {
    let root = TestWorktree::new("envp02a-gui-names");
    write(&root, "Canvas-design.svg", "<svg/>\n");
    write(&root, "notes.md", "Canvas design and Notional examples.\n");
    let first = detect_environment(&[folder(&root)], "environment").unwrap();
    assert!(
        !first.proposals.iter().any(|p| matches!(&p.requirement.spec,
        RequirementSpec::Software { identity, .. } if identity == "Canva" || identity == "Notion"))
    );
    write(&root, "notes.md", "Canva and Notion. ChatGPT.\n");
    let actual = detect_environment(&[folder(&root)], "environment").unwrap();
    for name in ["Canva", "Notion", "ChatGPT"] {
        assert!(actual
            .proposals
            .iter()
            .any(|p| matches!(&p.requirement.spec,
            RequirementSpec::Software { identity, detect_only: true, .. } if identity == name)));
    }
}

#[path = "detect_review_2_tests.rs"]
mod review_2;

#[path = "detect_review_3_tests.rs"]
mod review_3;

#[path = "detect_review_4_tests.rs"]
mod review_4;

#[path = "detect_review_5_tests.rs"]
mod review_5;

#[path = "detect_review_1_tests.rs"]
mod review_1;
