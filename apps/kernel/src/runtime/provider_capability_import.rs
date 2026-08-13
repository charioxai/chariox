use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::error::DaemonError;
use crate::local::{
    ImportProviderCapabilitiesRequest, LocalDaemonResponse, ProviderCapabilityImportDuplicate,
    ProviderCapabilityImportEntry, ProviderCapabilityImportReport, ProviderCapabilityImportSummary,
};

const DEFAULT_PROVIDERS: &[&str] = &["codex", "opencode", "claude"];

pub(crate) fn execute_import_provider_capabilities_request(
    request: ImportProviderCapabilitiesRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let workspace =
        super::capability_registry::registry_workspace_root(request.workspace_id.as_deref())?;
    let providers = normalize_providers(&request.providers)?;
    let kind = request
        .kind
        .as_deref()
        .unwrap_or("all")
        .to_ascii_lowercase();
    let include_mcp = matches!(kind.as_str(), "all" | "mcp" | "mcps");
    let include_skill = matches!(kind.as_str(), "all" | "skill" | "skills");
    if !include_mcp && !include_skill {
        return Err(DaemonError::InvalidConfig {
            field: "kind",
            message: "kind must be all, mcp, or skill",
        });
    }
    if let Some(name) = request.name.as_deref() {
        crate::mcp::validate_registry_name(name, "capability name")?;
    }

    let mut report = ProviderCapabilityImportReport {
        dry_run: request.dry_run,
        providers: providers.clone(),
        summary: ProviderCapabilityImportSummary::default(),
        mcps: Vec::new(),
        skills: Vec::new(),
    };

    if include_mcp {
        import_mcp_candidates(
            &mut report,
            &providers,
            &workspace,
            request.name.as_deref(),
            request.dry_run,
            request.workspace_id.as_deref(),
        )?;
    }
    if include_skill {
        import_skill_candidates(
            &mut report,
            &providers,
            &workspace,
            request.name.as_deref(),
            request.dry_run,
            request.workspace_id.as_deref(),
        )?;
    }
    report.summary = summarize_report(&report);
    Ok(LocalDaemonResponse::ProviderCapabilitiesImported { report })
}

fn normalize_providers(providers: &[String]) -> Result<Vec<String>, DaemonError> {
    let requested: Vec<String> = if providers.is_empty() {
        DEFAULT_PROVIDERS
            .iter()
            .map(|provider| provider.to_string())
            .collect()
    } else {
        providers.to_vec()
    };
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for provider in requested {
        let Some(normalized) = crate::provider::canonical_provider_family(&provider) else {
            return Err(DaemonError::InvalidConfig {
                field: "provider",
                message: "providers must be Codex, OpenCode, or Claude",
            });
        };
        if seen.insert(normalized.to_string()) {
            out.push(normalized.to_string());
        }
    }
    Ok(out)
}

fn import_mcp_candidates(
    report: &mut ProviderCapabilityImportReport,
    providers: &[String],
    workspace: &Path,
    requested_name: Option<&str>,
    dry_run: bool,
    workspace_id: Option<&str>,
) -> Result<(), DaemonError> {
    let registry = crate::mcp::CharioxMcpRegistry::new(
        super::capability_registry::mcp_registry_roots(workspace_id)?,
    );
    let mut candidates = Vec::new();
    for provider in providers {
        match crate::mcp::discover_provider_mcp_import_candidates(
            provider,
            workspace,
            requested_name,
        ) {
            Ok(discovery) => {
                for skip in discovery.skipped {
                    report.mcps.push(entry(
                        "mcp",
                        &skip.name,
                        provider,
                        "",
                        None,
                        "skipped",
                        &skip.reason,
                        Vec::new(),
                    ));
                }
                candidates.extend(discovery.candidates);
            }
            Err(error) => report.mcps.push(entry(
                "mcp",
                requested_name.unwrap_or("*"),
                provider,
                "",
                None,
                "error",
                &error.to_string(),
                Vec::new(),
            )),
        }
    }

    for mut group in group_mcp_candidates(candidates).into_values() {
        group.sort_by(|left, right| {
            compare_candidate_recency(
                right.source_modified_ms,
                provider_rank(&right.provider),
                &right.name,
                left.source_modified_ms,
                provider_rank(&left.provider),
                &left.name,
            )
        });
        let chosen = group.remove(0);
        let duplicates = duplicates_for_mcp(&chosen, &group);
        let (action, reason) = match registry.get(&chosen.name)? {
            Some(existing) => {
                let existing_hash = existing.definition_hash()?;
                if existing_hash == chosen.definition_hash {
                    (
                        "already_installed".to_string(),
                        "matching definition already installed in Chariox".to_string(),
                    )
                } else if dry_run {
                    (
                        "would_update".to_string(),
                        "newer provider definition would update installed Chariox MCP".to_string(),
                    )
                } else {
                    registry.update(&chosen.config)?;
                    (
                        "updated".to_string(),
                        "updated installed Chariox MCP from newest provider definition".to_string(),
                    )
                }
            }
            None if dry_run => (
                "would_import".to_string(),
                "would import newest provider definition".to_string(),
            ),
            None => {
                registry.install(&chosen.config)?;
                (
                    "imported".to_string(),
                    "imported newest provider definition".to_string(),
                )
            }
        };
        report.mcps.push(entry(
            "mcp",
            &chosen.name,
            &chosen.provider,
            &chosen.source,
            Some(chosen.definition_hash.clone()),
            &action,
            &reason,
            duplicates,
        ));
        for duplicate in group {
            let reason = if duplicate.definition_hash == chosen.definition_hash {
                "duplicate of selected provider definition"
            } else {
                "older provider definition superseded by selected provider definition"
            };
            report.mcps.push(entry(
                "mcp",
                &duplicate.name,
                &duplicate.provider,
                &duplicate.source,
                Some(duplicate.definition_hash),
                "deduped",
                reason,
                Vec::new(),
            ));
        }
    }
    Ok(())
}

fn import_skill_candidates(
    report: &mut ProviderCapabilityImportReport,
    providers: &[String],
    workspace: &Path,
    requested_name: Option<&str>,
    dry_run: bool,
    workspace_id: Option<&str>,
) -> Result<(), DaemonError> {
    let registry = crate::skill::CharioxSkillRegistry::new(
        super::capability_registry::skill_registry_roots(workspace_id)?,
    );
    let mut candidates = Vec::new();
    for provider in providers {
        match crate::skill::discover_provider_skill_import_candidates(
            provider,
            workspace,
            requested_name,
        ) {
            Ok(discovery) => {
                for skip in discovery.skipped {
                    report.skills.push(entry(
                        "skill",
                        &skip.name,
                        provider,
                        &skip.path.display().to_string(),
                        None,
                        "skipped",
                        &skip.reason,
                        Vec::new(),
                    ));
                }
                candidates.extend(discovery.candidates);
            }
            Err(error) => report.skills.push(entry(
                "skill",
                requested_name.unwrap_or("*"),
                provider,
                "",
                None,
                "error",
                &error.to_string(),
                Vec::new(),
            )),
        }
    }

    for mut group in group_skill_candidates(candidates).into_values() {
        group.sort_by(|left, right| {
            compare_candidate_recency(
                right.source_modified_ms,
                provider_rank(&right.provider),
                &right.name,
                left.source_modified_ms,
                provider_rank(&left.provider),
                &left.name,
            )
        });
        let chosen = group.remove(0);
        let duplicates = duplicates_for_skill(&chosen, &group);
        let (action, reason) = match registry.get(&chosen.name)? {
            Some(_) => {
                let existing_hash = registry
                    .package(&chosen.name)?
                    .map(|package| package.version_hash)
                    .unwrap_or_default();
                if existing_hash == chosen.version_hash {
                    (
                        "already_installed".to_string(),
                        "matching skill package already installed in Chariox".to_string(),
                    )
                } else if dry_run {
                    (
                        "would_update".to_string(),
                        "newer provider skill would update installed Chariox skill".to_string(),
                    )
                } else {
                    registry.update_from_path(&chosen.source_path)?;
                    (
                        "updated".to_string(),
                        "updated installed Chariox skill from newest provider package".to_string(),
                    )
                }
            }
            None if dry_run => (
                "would_import".to_string(),
                "would import newest provider skill package".to_string(),
            ),
            None => {
                registry.install_from_path(&chosen.source_path)?;
                (
                    "imported".to_string(),
                    "imported newest provider skill package".to_string(),
                )
            }
        };
        report.skills.push(entry(
            "skill",
            &chosen.name,
            &chosen.provider,
            &chosen.source,
            Some(chosen.version_hash.clone()),
            &action,
            &reason,
            duplicates,
        ));
        for duplicate in group {
            let reason = if duplicate.version_hash == chosen.version_hash {
                "duplicate of selected provider skill"
            } else {
                "older provider skill superseded by selected provider skill"
            };
            report.skills.push(entry(
                "skill",
                &duplicate.name,
                &duplicate.provider,
                &duplicate.source,
                Some(duplicate.version_hash),
                "deduped",
                reason,
                Vec::new(),
            ));
        }
    }
    Ok(())
}

fn group_mcp_candidates(
    candidates: Vec<crate::mcp::ProviderMcpImportCandidate>,
) -> BTreeMap<String, Vec<crate::mcp::ProviderMcpImportCandidate>> {
    let mut groups: BTreeMap<String, Vec<crate::mcp::ProviderMcpImportCandidate>> = BTreeMap::new();
    for candidate in candidates {
        groups
            .entry(normalized_capability_name(&candidate.name))
            .or_default()
            .push(candidate);
    }
    groups
}

fn group_skill_candidates(
    candidates: Vec<crate::skill::ProviderSkillImportCandidate>,
) -> BTreeMap<String, Vec<crate::skill::ProviderSkillImportCandidate>> {
    let mut groups: BTreeMap<String, Vec<crate::skill::ProviderSkillImportCandidate>> =
        BTreeMap::new();
    for candidate in candidates {
        groups
            .entry(normalized_capability_name(&candidate.name))
            .or_default()
            .push(candidate);
    }
    groups
}

fn normalized_capability_name(name: &str) -> String {
    name.to_ascii_lowercase()
}

fn compare_candidate_recency(
    left_modified_ms: u64,
    left_provider_rank: u8,
    left_name: &str,
    right_modified_ms: u64,
    right_provider_rank: u8,
    right_name: &str,
) -> std::cmp::Ordering {
    left_modified_ms
        .cmp(&right_modified_ms)
        .then_with(|| right_provider_rank.cmp(&left_provider_rank))
        .then_with(|| right_name.cmp(left_name))
}

fn provider_rank(provider: &str) -> u8 {
    match provider {
        "codex" => 0,
        "opencode" => 1,
        "claude" => 2,
        _ => 3,
    }
}

fn duplicates_for_mcp(
    chosen: &crate::mcp::ProviderMcpImportCandidate,
    duplicates: &[crate::mcp::ProviderMcpImportCandidate],
) -> Vec<ProviderCapabilityImportDuplicate> {
    duplicates
        .iter()
        .map(|duplicate| ProviderCapabilityImportDuplicate {
            provider: duplicate.provider.clone(),
            source: duplicate.source.clone(),
            hash: Some(duplicate.definition_hash.clone()),
            reason: if duplicate.definition_hash == chosen.definition_hash {
                "same definition hash".to_string()
            } else {
                "different definition hash, older source".to_string()
            },
        })
        .collect()
}

fn duplicates_for_skill(
    chosen: &crate::skill::ProviderSkillImportCandidate,
    duplicates: &[crate::skill::ProviderSkillImportCandidate],
) -> Vec<ProviderCapabilityImportDuplicate> {
    duplicates
        .iter()
        .map(|duplicate| ProviderCapabilityImportDuplicate {
            provider: duplicate.provider.clone(),
            source: duplicate.source.clone(),
            hash: Some(duplicate.version_hash.clone()),
            reason: if duplicate.version_hash == chosen.version_hash {
                "same package hash".to_string()
            } else {
                "different package hash, older source".to_string()
            },
        })
        .collect()
}

fn entry(
    kind: &str,
    name: &str,
    provider: &str,
    source: &str,
    hash: Option<String>,
    action: &str,
    reason: &str,
    duplicates: Vec<ProviderCapabilityImportDuplicate>,
) -> ProviderCapabilityImportEntry {
    ProviderCapabilityImportEntry {
        kind: kind.to_string(),
        name: name.to_string(),
        provider: provider.to_string(),
        source: source.to_string(),
        hash,
        action: action.to_string(),
        reason: reason.to_string(),
        duplicates,
    }
}

fn summarize_report(report: &ProviderCapabilityImportReport) -> ProviderCapabilityImportSummary {
    let mut summary = ProviderCapabilityImportSummary::default();
    for entry in report.mcps.iter().chain(report.skills.iter()) {
        match entry.action.as_str() {
            "imported" => summary.imported += 1,
            "updated" => summary.updated += 1,
            "already_installed" => summary.already_installed += 1,
            "deduped" => summary.deduped += 1,
            "skipped" => summary.skipped += 1,
            "error" => summary.errors += 1,
            "would_import" | "would_update" => {}
            _ => {}
        }
        if !matches!(entry.action.as_str(), "skipped" | "error") {
            summary.candidates += 1;
        }
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::Duration;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "chariox-provider-capability-import-{name}-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn normalizes_provider_aliases_and_deduplicates() {
        let providers = normalize_providers(&[
            "codex".to_string(),
            "claude-p".to_string(),
            "claude".to_string(),
            "opencode".to_string(),
        ])
        .unwrap();

        assert_eq!(providers, vec!["codex", "claude", "opencode"]);
    }

    #[test]
    fn imports_newest_provider_capabilities_after_deduplication() {
        let _guard = crate::env_lock::lock();
        let workspace = temp_root("workspace");
        let codex_home = temp_root("codex-home");
        let previous_codex_home = std::env::var_os("CODEX_HOME");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&codex_home).unwrap();
        std::env::set_var("CODEX_HOME", &codex_home);

        fs::write(
            codex_home.join("config.toml"),
            r#"
[mcp_servers.docs]
command = "codex-docs"
"#,
        )
        .unwrap();
        let codex_skill = workspace.join(".codex").join("skills").join("qa");
        fs::create_dir_all(&codex_skill).unwrap();
        fs::write(
            codex_skill.join("SKILL.md"),
            "---\nname: qa\ndescription: Codex QA\n---\nUse Codex QA.\n",
        )
        .unwrap();

        std::thread::sleep(Duration::from_millis(10));
        fs::write(
            workspace.join(".mcp.json"),
            r#"
{
  "mcpServers": {
    "docs": {
      "command": "claude-docs"
    }
  }
}
"#,
        )
        .unwrap();
        let claude_skill = workspace.join(".claude").join("skills").join("qa");
        fs::create_dir_all(&claude_skill).unwrap();
        fs::write(
            claude_skill.join("SKILL.md"),
            "---\nname: qa\ndescription: Claude QA\n---\nUse Claude QA.\n",
        )
        .unwrap();

        let response =
            execute_import_provider_capabilities_request(ImportProviderCapabilitiesRequest {
                workspace_id: Some(workspace.display().to_string()),
                providers: vec!["codex".to_string(), "claude".to_string()],
                kind: Some("all".to_string()),
                name: None,
                dry_run: false,
            })
            .unwrap();

        let LocalDaemonResponse::ProviderCapabilitiesImported { report } = response else {
            panic!("unexpected response");
        };
        assert!(report.summary.imported >= 2);
        assert!(report.summary.deduped >= 2);
        assert!(report.mcps.iter().any(|entry| entry.name == "docs"
            && entry.provider == "claude"
            && entry.action == "imported"));
        assert!(report.mcps.iter().any(|entry| entry.name == "docs"
            && entry.provider == "codex"
            && entry.action == "deduped"));
        assert!(report.skills.iter().any(|entry| entry.name == "qa"
            && entry.provider == "claude"
            && entry.action == "imported"));
        assert!(report.skills.iter().any(|entry| entry.name == "qa"
            && entry.provider == "codex"
            && entry.action == "deduped"));

        restore_env_var("CODEX_HOME", previous_codex_home);
        let _ = fs::remove_dir_all(workspace);
        let _ = fs::remove_dir_all(codex_home);
    }

    fn restore_env_var(key: &str, value: Option<std::ffi::OsString>) {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}
