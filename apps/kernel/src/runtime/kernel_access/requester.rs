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

pub(crate) fn default_holder(peer: &ProcessIdentity) -> std::io::Result<ProcessIdentity> {
    let chain = super::process::ancestry(peer)?;
    // MP-08 / MP-10 / MP-11: npm launchers and shell exec optimizations change
    // depth. Select only an installed harness in this exact OS-verified chain,
    // never a sibling, descendant, or a process found by its basename.
    chain
        .iter()
        .skip(1)
        .find(|ancestor| {
            ancestor.uid == peer.uid && provider_harness(&ancestor.executable).is_some()
        })
        .or_else(|| chain.get(2))
        .cloned()
        .ok_or_else(|| std::io::Error::other("holder ancestor unavailable"))
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
    if crate::provider::resolve_claude_executable().is_ok_and(|path| {
        same_executable(executable, &path) || same_claude_installation(executable, &path)
    }) {
        return Some(KernelAccessProviderHarness::Claude);
    }
    if crate::provider::resolve_opencode_executable()
        .is_ok_and(|path| same_executable(executable, &path))
    {
        return Some(KernelAccessProviderHarness::Opencode);
    }
    None
}

// MP-08 / MP-10 / MP-11: a native updater can move the configured symlink
// while an older official process is still running. Attribute only existing
// version files in that same configured installation, never arbitrary paths
// with a Claude-looking basename. OS ancestry and human approval still admit
// the holder; attribution does not confer authority.
fn same_claude_installation(executable: &str, configured: &Path) -> bool {
    let Ok(configured) = std::fs::canonicalize(configured) else {
        return false;
    };
    let Some(versions) = configured.parent() else {
        return false;
    };
    let candidate = Path::new(executable);
    versions.file_name().and_then(|s| s.to_str()) == Some("versions")
        && versions
            .parent()
            .and_then(Path::file_name)
            .and_then(|s| s.to_str())
            == Some("claude")
        && native_claude_version(&configured)
        && native_claude_version(candidate)
        && candidate.parent() == Some(versions)
        && std::fs::canonicalize(candidate).is_ok_and(|path| path == candidate)
}

fn native_claude_version(path: &Path) -> bool {
    path.file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|name| {
            let parts: Vec<_> = name.split('.').collect();
            parts.len() == 3
                && parts
                    .iter()
                    .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        })
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
    let native = Path::new("vendor").join(platform.1).join("bin/codex");
    let package = format!("codex-{}", platform.0);
    let mut candidates = vec![
        root.join(&native),
        root.join("node_modules/@openai")
            .join(&package)
            .join(&native),
    ];
    if let Some(parent) = root.parent() {
        candidates.push(parent.join(&package).join(&native));
    }
    candidates
}

/// The owner-facing copy of an access decision. It names only a harness this
/// kernel recognises by exact configured path, never text from the process;
/// PID, path and start time stay in the requester record and the audit log.
pub(crate) fn popup_copy(
    harness: Option<KernelAccessProviderHarness>,
    grant: bool,
    remaining_minutes: u64,
) -> (&'static str, String) {
    let name = match harness {
        Some(KernelAccessProviderHarness::Codex) => Some("Codex"),
        Some(KernelAccessProviderHarness::Claude) => Some("Claude Code"),
        Some(KernelAccessProviderHarness::Opencode) => Some("OpenCode"),
        None => None,
    };
    if grant {
        let who = name.map_or("An external agent".to_owned(), |n| {
            format!("{n} (an external agent)")
        });
        return (
            "External agent access",
            format!("{who} wants access to Chariox. Allow?"),
        );
    }
    let minutes = remaining_minutes.max(1);
    let unit = if minutes == 1 { "minute" } else { "minutes" };
    let who = name.unwrap_or("The external agent");
    (
        "Extend external agent access",
        format!("{who}'s access ends in {minutes} {unit}. Extend?"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requester_path_cannot_claim_a_harness_or_reach_the_popup_copy() {
        let path = "/tmp/\" (pid 999)\nRequester: Claude\u{1b}[31m\u{202e}\u{85}";
        assert_eq!(provider_harness("/crafted/path/codex"), None);
        let copy = |harness, grant| popup_copy(harness, grant, 5).1;
        assert_eq!(
            copy(None, true),
            "An external agent wants access to Chariox. Allow?"
        );
        assert_eq!(
            copy(Some(KernelAccessProviderHarness::Claude), true),
            "Claude Code (an external agent) wants access to Chariox. Allow?"
        );
        assert_eq!(
            copy(Some(KernelAccessProviderHarness::Claude), false),
            "Claude Code's access ends in 5 minutes. Extend?"
        );
        assert_eq!(
            popup_copy(None, false, 0).1,
            "The external agent's access ends in 1 minute. Extend?"
        );
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

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod launcher_tests {
    use super::*;
    use std::process::Command;

    struct Install(PathBuf);
    impl Install {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!(
                "ka-launcher-{}-{:x}",
                std::process::id(),
                rand::random::<u64>()
            )))
        }
        fn file(&self, relative: &str) -> PathBuf {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "fixture").unwrap();
            path
        }
        fn native(&self, relative: &str) -> PathBuf {
            let path = self.file(relative);
            std::fs::copy("/bin/bash", &path).unwrap();
            path
        }
    }
    impl Drop for Install {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    const PROBE: &str = "runtime::kernel_access::requester::launcher_tests::launcher_probe";
    const SPAWN: &str = "const r=require('child_process').spawnSync(process.argv[1],process.argv.slice(2),{stdio:'inherit'});process.exitCode=r.status??1;";

    #[test]
    fn codex_nested_platform_install_is_recognized() {
        crate::test_support::isolated_env_test!();
        let _guard = crate::env_lock::lock();
        let install = Install::new();
        let launcher = install.file("lib/node_modules/@openai/codex/bin/codex.js");
        let native = install.native("lib/node_modules/@openai/codex/node_modules/@openai/codex-linux-x64/vendor/x86_64-unknown-linux-musl/bin/codex");
        std::env::set_var("CHARIOX_CODEX_BIN", &launcher);
        assert_eq!(
            provider_harness(native.to_str().unwrap()),
            Some(KernelAccessProviderHarness::Codex)
        );
        assert_eq!(provider_harness("/usr/bin/node"), None);
        assert_eq!(
            provider_harness(install.file("unrelated/codex").to_str().unwrap()),
            None
        );
    }

    #[test]
    fn default_holder_follows_npm_native_and_tool_helpers() {
        crate::test_support::isolated_env_test!();
        let install = Install::new();
        let launcher = install.file("lib/node_modules/@openai/codex/bin/codex.js");
        let native = install.native("lib/node_modules/@openai/codex/node_modules/@openai/codex-linux-x64/vendor/x86_64-unknown-linux-musl/bin/codex");
        for shell in ["\"$@\"; exit $?", "node -e 'const r=require(\"child_process\").spawnSync(process.argv[1],process.argv.slice(2),{stdio:\"inherit\"});process.exitCode=r.status??1' /bin/sh -c '\"$@\"; exit $?' tool \"$@\"; exit $?"] {
            let status = Command::new("node").args(["-e", SPAWN]).arg(&native)
                .args(["-c", shell, "provider"]).arg(std::env::current_exe().unwrap())
                .args(["--exact", PROBE, "--ignored", "--nocapture"])
                .env("CHARIOX_CODEX_BIN", &launcher).env("KA_EXPECT_EXECUTABLE", &native)
                .env("KA_EXPECT_HARNESS", "codex").status().unwrap();
            assert!(status.success(), "npm wrapper and native identity must remain distinct");
        }
    }

    #[test]
    fn default_holder_recognizes_native_claude_and_opencode() {
        crate::test_support::isolated_env_test!();
        let install = Install::new();
        for (provider, variable) in [
            ("claude", "CHARIOX_CLAUDE_BIN"),
            ("opencode", "CHARIOX_OPENCODE_BIN"),
        ] {
            let native = install.native(&format!("bin/{provider}"));
            let status = Command::new("node")
                .args(["-e", SPAWN])
                .arg(&native)
                .args(["-c", "\"$@\"; exit $?", "provider"])
                .arg(std::env::current_exe().unwrap())
                .args(["--exact", PROBE, "--ignored", "--nocapture"])
                .env(variable, &native)
                .env("KA_EXPECT_EXECUTABLE", &native)
                .env("KA_EXPECT_HARNESS", provider)
                .status()
                .unwrap();
            assert!(
                status.success(),
                "direct native harness identity must be selected"
            );
        }
    }

    #[test]
    fn default_holder_keeps_running_claude_after_native_update() {
        crate::test_support::isolated_env_test!();
        let install = Install::new();
        let previous = install.native("share/claude/versions/2.1.292");
        let current = install.native("share/claude/versions/2.1.296");
        let launcher = install.0.join("bin/claude");
        std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&current, &launcher).unwrap();
        let status = Command::new("node")
            .args(["-e", SPAWN])
            .arg(&previous)
            .args(["-c", "\"$@\"; exit $?", "provider"])
            .arg(std::env::current_exe().unwrap())
            .args(["--exact", PROBE, "--ignored", "--nocapture"])
            .env("CHARIOX_CLAUDE_BIN", &launcher)
            .env("KA_EXPECT_EXECUTABLE", &previous)
            .env("KA_EXPECT_HARNESS", "claude")
            .status()
            .unwrap();
        assert!(
            status.success(),
            "a native update must keep the running provider as holder"
        );
    }

    #[test]
    fn claude_version_attribution_stays_inside_configured_installation() {
        crate::test_support::isolated_env_test!();
        let _guard = crate::env_lock::lock();
        let install = Install::new();
        let current = install.native("share/claude/versions/2.1.296");
        let previous = install.native("share/claude/versions/2.1.292");
        std::env::set_var("CHARIOX_CLAUDE_BIN", &current);
        assert_eq!(
            provider_harness(previous.to_str().unwrap()),
            Some(KernelAccessProviderHarness::Claude)
        );
        for relative in [
            "other/claude/versions/2.1.292",
            "share/claude/versions/bash",
            "share/claude/versions/2.1.292.extra",
        ] {
            let unrelated = install.native(relative);
            assert_eq!(provider_harness(unrelated.to_str().unwrap()), None);
        }
        let outside = install.native("other/2.1.290");
        std::os::unix::fs::symlink(&outside, install.0.join("share/claude/versions/2.1.290"))
            .unwrap();
        assert_eq!(provider_harness(outside.to_str().unwrap()), None);
    }

    #[test]
    fn unknown_requester_does_not_select_a_provider_sibling() {
        crate::test_support::isolated_env_test!();
        let install = Install::new();
        let native = install.native("bin/codex");
        let mut sibling = Command::new(&native)
            .args(["-c", "sleep 2; exit 0"])
            .spawn()
            .unwrap();
        let status = Command::new("/bin/bash")
            .args(["-c", "\"$@\"; exit $?", "unknown"])
            .arg(std::env::current_exe().unwrap())
            .args(["--exact", PROBE, "--ignored", "--nocapture"])
            .env("CHARIOX_CODEX_BIN", &native)
            .env("KA_EXPECT_UNKNOWN", "1")
            .env("KA_SIBLING_PID", sibling.id().to_string())
            .status()
            .unwrap();
        sibling.wait().unwrap();
        assert!(
            status.success(),
            "known provider outside ancestry cannot become the holder"
        );
    }

    #[test]
    #[ignore = "subprocess entry point exercised by launcher parent regressions"]
    fn launcher_probe() {
        if std::env::var_os("KA_EXPECT_UNKNOWN").is_some() {
            let peer = super::super::process::inspect(std::process::id())
                .unwrap()
                .0;
            let (_, parent) = super::super::process::inspect(peer.pid).unwrap();
            let (_, grandparent) = super::super::process::inspect(parent).unwrap();
            let selected = crate::local::default_access_holder_pid().unwrap();
            assert_eq!(selected, grandparent);
            assert_eq!(
                project(&super::super::process::inspect(selected).unwrap().0).provider_harness,
                None
            );
            let sibling = std::env::var("KA_SIBLING_PID").unwrap().parse().unwrap();
            assert!(super::super::process::holder(&peer, sibling).is_err());
            return;
        }
        let expected = std::env::var("KA_EXPECT_EXECUTABLE").unwrap();
        let pid = crate::local::default_access_holder_pid().unwrap();
        let peer = super::super::process::inspect(std::process::id())
            .unwrap()
            .0;
        let selected = super::super::process::holder(&peer, pid).unwrap();
        assert_eq!(
            selected.executable, expected,
            "holder is the native process, not its launcher or tool helper"
        );
        let harness = match std::env::var("KA_EXPECT_HARNESS").unwrap().as_str() {
            "codex" => KernelAccessProviderHarness::Codex,
            "claude" => KernelAccessProviderHarness::Claude,
            "opencode" => KernelAccessProviderHarness::Opencode,
            _ => panic!("invalid fixture provider"),
        };
        assert_eq!(project(&selected).provider_harness, Some(harness));
        let mut reused = selected.clone();
        reused.start += 1;
        assert!(!reused.alive());
        assert!(!reused.contains(&peer));
    }
}
