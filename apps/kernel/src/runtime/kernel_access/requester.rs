//! MP-08 / MP-10 / MP-11: requester projection belongs to the kernel, below clients.
use super::process::ProcessIdentity;
use crate::local::{KernelAccessProviderHarness, KernelAccessRequester};
use std::path::{Path, PathBuf};

pub(crate) fn project(holder: &ProcessIdentity) -> KernelAccessRequester {
    KernelAccessRequester {
        executable: holder.executable.clone(),
        pid: holder.pid,
        process_start_id: holder.start.to_string(),
        process_exec_version: holder.version,
        provider_harness: provider_harness(&holder.executable),
    }
}

fn same_executable(executable: &str, candidate: &Path) -> bool {
    std::fs::canonicalize(candidate).is_ok_and(|path| path == Path::new(executable))
}

fn provider_harness(executable: &str) -> Option<KernelAccessProviderHarness> {
    // Only exact configured executable paths qualify. A crafted basename or
    // human-readable message cannot claim a harness. This is attribution only.
    if let Ok(path) = crate::provider::resolve_codex_executable() {
        if same_executable(executable, &path)
            || codex_native_candidates(&path)
                .iter()
                .any(|path| same_executable(executable, path))
        {
            return Some(KernelAccessProviderHarness::Codex);
        }
    }
    if crate::provider::resolve_claude_executable()
        .is_ok_and(|path| same_executable(executable, &path))
    {
        return Some(KernelAccessProviderHarness::Claude);
    }
    if crate::provider::resolve_opencode_executable()
        .is_ok_and(|path| same_executable(executable, &path))
    {
        return Some(KernelAccessProviderHarness::Opencode);
    }
    None
}

fn codex_native_candidates(launcher: &Path) -> Vec<PathBuf> {
    // The official npm launcher executes its platform package (or legacy vendor
    // layout). Resolve only paths anchored in that configured installation.
    let Ok(launcher) = std::fs::canonicalize(launcher) else {
        return vec![];
    };
    if launcher.file_name().and_then(|s| s.to_str()) != Some("codex.js") {
        return vec![];
    }
    let Some(root) = launcher.parent().and_then(Path::parent) else {
        return vec![];
    };
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => ("linux-x64", "x86_64-unknown-linux-musl"),
        ("linux", "aarch64") => ("linux-arm64", "aarch64-unknown-linux-musl"),
        ("macos", "x86_64") => ("darwin-x64", "x86_64-apple-darwin"),
        ("macos", "aarch64") => ("darwin-arm64", "aarch64-apple-darwin"),
        _ => return vec![],
    };
    let mut candidates = vec![root.join("vendor").join(platform.1).join("bin/codex")];
    if let Some(parent) = root.parent() {
        candidates.push(
            parent
                .join(format!("codex-{}", platform.0))
                .join("vendor")
                .join(platform.1)
                .join("bin/codex"),
        );
    }
    candidates
}

pub(crate) fn display_executable(executable: &str) -> String {
    // JSON quoting separates the whole path from the trusted surrounding text.
    // Also escape terminal controls and Unicode direction/line formatting.
    serde_json::to_string(executable).expect("string serializes").chars().map(|c| {
        if c.is_control() || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
            format!("\\u{:04x}", c as u32)
        } else { c.to_string() }
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requester_path_cannot_inject_controls_quotes_or_provider_labels() {
        let path = "/tmp/\" (pid 999)\nRequester: Claude\u{1b}[31m\u{202e}\u{85}";
        let display = display_executable(path);
        assert_eq!(
            display,
            "\"/tmp/\\\" (pid 999)\\nRequester: Claude\\u001b[31m\\u202e\\u0085\""
        );
        assert!(!display.chars().any(char::is_control));
        assert_eq!(provider_harness("/crafted/path/codex"), None);
        let holder = ProcessIdentity {
            pid: 42,
            uid: 0,
            start: u64::MAX,
            version: 7,
            executable: path.into(),
        };
        let projected = project(&holder);
        assert_eq!(projected.process_start_id, "18446744073709551615");
        assert_eq!(projected.executable, path);
        assert_eq!(projected.provider_harness, None);
    }
}
