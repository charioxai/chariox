//! MP-08 / MP-10 / MP-11: skill-only roots from the resolved launch environment.
//! Filesystem discovery is a candidate list, not proof of harness visibility.
use super::*;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

/// Extra roots are already allowlisted, resolved settings/launch arguments:
/// Claude --add-dir contributes <dir>/.claude/skills; plugins contribute their
/// enabled <plugin>/skills root. OpenCode skills.paths contributes skill roots.
/// Never read arbitrary provider settings/credential payloads here.
pub fn filesystem_skill_candidates(
    binding: &SkillInventoryBinding,
    worktree_root: &Path,
    additional_directories: &[PathBuf],
    enabled_plugin_skill_roots: &[PathBuf],
    configured_skill_roots: &[PathBuf],
) -> Result<Vec<NativeSkillCandidate>, DaemonError> {
    binding.validate()?;
    if !worktree_root.is_absolute() || !binding.cwd.starts_with(worktree_root) {
        return Err(failure(
            "native skill discovery needs the actual execution worktree boundary",
        ));
    }
    if binding.provider == "codex" {
        return Err(failure("Codex inventory requires official skills/list"));
    }
    let env_path = |key: &str| binding.environment.get(key).map(PathBuf::from);
    let mut roots = Vec::new();
    let home = env_path("HOME");
    if binding.provider == "claude" {
        if let Some(config) =
            env_path("CLAUDE_CONFIG_DIR").or_else(|| home.as_ref().map(|h| h.join(".claude")))
        {
            roots.push(config.join("skills"));
        }
        roots.extend(
            additional_directories
                .iter()
                .map(|p| p.join(".claude/skills")),
        );
        roots.extend_from_slice(enabled_plugin_skill_roots);
    } else {
        if let Some(config) = env_path("OPENCODE_CONFIG_DIR") {
            roots.extend([config.join("skills"), config.join("skill")]);
        }
        if let Some(config) =
            env_path("XDG_CONFIG_HOME").or_else(|| home.as_ref().map(|h| h.join(".config")))
        {
            roots.extend([
                config.join("opencode/skills"),
                config.join("opencode/skill"),
            ]);
        }
        if let Some(home) = &home {
            if !is_true(binding.environment.get("OPENCODE_DISABLE_CLAUDE_SKILLS"))
                && !is_true(binding.environment.get("OPENCODE_DISABLE_EXTERNAL_SKILLS"))
            {
                roots.push(home.join(".claude/skills"));
            }
            if !is_true(binding.environment.get("OPENCODE_DISABLE_EXTERNAL_SKILLS")) {
                roots.push(home.join(".agents/skills"));
            }
        }
        roots.extend_from_slice(configured_skill_roots);
    }
    for ancestor in binding
        .cwd
        .ancestors()
        .take_while(|p| p.starts_with(worktree_root))
    {
        if binding.provider == "claude" {
            roots.push(ancestor.join(".claude/skills"));
        } else {
            roots.extend([
                ancestor.join(".opencode/skills"),
                ancestor.join(".opencode/skill"),
            ]);
            if !is_true(binding.environment.get("OPENCODE_DISABLE_CLAUDE_SKILLS"))
                && !is_true(binding.environment.get("OPENCODE_DISABLE_EXTERNAL_SKILLS"))
            {
                roots.push(ancestor.join(".claude/skills"));
            }
            if !is_true(binding.environment.get("OPENCODE_DISABLE_EXTERNAL_SKILLS")) {
                roots.push(ancestor.join(".agents/skills"));
            }
        }
    }
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    for root in roots {
        if !root.is_absolute() {
            return Err(failure(
                "native skill roots must be resolved absolute paths",
            ));
        }
        if !seen.insert(root.clone()) || !root.exists() {
            continue;
        }
        collect_candidates(
            &root,
            if binding.provider == "opencode" {
                32
            } else {
                1
            },
            &mut 0,
            &mut candidates,
        )?;
    }
    Ok(candidates)
}
fn is_true(value: Option<&String>) -> bool {
    value.is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

fn collect_candidates(
    root: &Path,
    depth: usize,
    visited: &mut usize,
    candidates: &mut Vec<NativeSkillCandidate>,
) -> Result<(), DaemonError> {
    use std::io::Read;
    *visited += 1;
    if *visited > 4096 {
        return Err(failure(
            "native skill-only roots exceed the bounded directory scan",
        ));
    }
    let file = root.join("SKILL.md");
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    if let Ok(input) = options.open(&file) {
        const MAX_BYTES: u64 = 10 * 1024 * 1024;
        if input
            .metadata()
            .is_ok_and(|m| m.is_file() && m.len() <= MAX_BYTES)
        {
            let mut body = String::new();
            if input.take(MAX_BYTES + 1).read_to_string(&mut body).is_ok()
                && body.len() as u64 <= MAX_BYTES
            {
                if let Ok(metadata) = parse_frontmatter(&body) {
                    if candidates.len() >= 512 {
                        return Err(failure(
                            "native inventory exceeds 512 candidates; narrow the configured roots",
                        ));
                    }
                    candidates.push(NativeSkillCandidate { name: metadata.name, description: metadata.description, skill_file: file,
                        exposure: NativeSkillExposure::Unverified, supports_load_ack: false, observed_body_digest: None,
                        discovery_error: Some("directory candidate; effective harness exposure and precedence have not been confirmed".into()) });
                }
            }
        }
    }
    if depth == 0 {
        return Ok(());
    }
    for entry in
        std::fs::read_dir(root).map_err(|_| failure("cannot enumerate native skill-only root"))?
    {
        let entry = entry.map_err(|_| failure("cannot enumerate native skill candidate"))?;
        if entry
            .file_type()
            .map_err(|_| failure("cannot inspect native skill candidate"))?
            .is_dir()
        {
            collect_candidates(&entry.path(), depth - 1, visited, candidates)?;
        }
    }
    Ok(())
}
