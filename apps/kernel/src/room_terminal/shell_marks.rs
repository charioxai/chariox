//! Shell integration: the shell itself says when a typed command starts and
//! finishes (OSC 133 `C` and `D;<status>`), so completion does not depend on
//! sampling which process group owns the terminal. The hooks are handed to
//! the shell at start through its environment; no rc file is edited.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What a shell reports about the command line it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ShellMark {
    CommandStarted,
    CommandFinished { exit_code: Option<u32> },
}

const OSC_PREFIX: &[u8] = b"133;";
/// Longer bodies are not ours: `D;255` is the longest mark.
const MAX_BODY: usize = 32;

#[derive(Default)]
enum Scan {
    #[default]
    Idle,
    Escape,
    Body(Vec<u8>),
    BodyEscape(Vec<u8>),
}

/// Finds marks in the raw output, whatever the chunking.
#[derive(Default)]
pub(super) struct MarkScanner {
    scan: Scan,
}

impl MarkScanner {
    pub(super) fn feed(&mut self, bytes: &[u8]) -> Vec<ShellMark> {
        let mut marks = Vec::new();
        for &byte in bytes {
            self.scan = match (std::mem::take(&mut self.scan), byte) {
                (Scan::Idle, 0x1b) | (Scan::Escape, 0x1b) => Scan::Escape,
                (Scan::Idle, _) => Scan::Idle,
                (Scan::Escape, b']') => Scan::Body(Vec::new()),
                (Scan::Escape, _) => Scan::Idle,
                (Scan::Body(body), 0x07) => {
                    marks.extend(parse(&body));
                    Scan::Idle
                }
                (Scan::Body(body), 0x1b) => Scan::BodyEscape(body),
                (Scan::Body(mut body), byte) => {
                    body.push(byte);
                    if body.len() > MAX_BODY {
                        Scan::Idle
                    } else {
                        Scan::Body(body)
                    }
                }
                (Scan::BodyEscape(body), b'\\') => {
                    marks.extend(parse(&body));
                    Scan::Idle
                }
                (Scan::BodyEscape(_), _) => Scan::Idle,
            };
        }
        marks
    }
}

fn parse(body: &[u8]) -> Option<ShellMark> {
    let rest = body.strip_prefix(OSC_PREFIX)?;
    match rest {
        b"C" => Some(ShellMark::CommandStarted),
        b"D" => Some(ShellMark::CommandFinished { exit_code: None }),
        _ => {
            let status = rest.strip_prefix(b"D;")?;
            Some(ShellMark::CommandFinished {
                exit_code: std::str::from_utf8(status).ok()?.parse().ok(),
            })
        }
    }
}

/// The hooks for the shells that support them.
pub(super) struct ShellIntegration {
    zsh_dir: Option<PathBuf>,
    bash_startup: Option<PathBuf>,
}

/// zsh reads its startup files from `$ZDOTDIR`. This one points there only
/// for `.zshenv`: it hands `$ZDOTDIR` back, reads the user's own `.zshenv`,
/// and adds the marks at the first prompt, after every rc file ran.
const ZSH_ENV: &str = r#"# Chariox terminal integration. Not a place to edit: the kernel rewrites it.
if [[ -n "${CHARIOX_ORIG_ZDOTDIR+x}" ]]; then
  if [[ -n "$CHARIOX_ORIG_ZDOTDIR" ]]; then ZDOTDIR="$CHARIOX_ORIG_ZDOTDIR"; else unset ZDOTDIR; fi
  unset CHARIOX_ORIG_ZDOTDIR
fi
[[ -r "${ZDOTDIR:-$HOME}/.zshenv" ]] && source "${ZDOTDIR:-$HOME}/.zshenv"
__chariox_precmd() { local status_code=$?; print -rn -- $'\e]133;D;'"$status_code"$'\a'; }
__chariox_preexec() { print -rn -- $'\e]133;C\a'; }
__chariox_install() {
  precmd_functions=(${precmd_functions:#__chariox_install})
  precmd_functions=(__chariox_precmd $precmd_functions)
  preexec_functions=(__chariox_preexec $preexec_functions)
}
precmd_functions+=(__chariox_install)
"#;

impl ShellIntegration {
    /// Write the zsh startup stub under `root`. A shell that cannot be given
    /// marks falls back to sampling the foreground process group.
    pub(super) fn install(root: &Path) -> Self {
        let dir = root.join("zsh");
        let written = std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(dir.join(".zshenv"), ZSH_ENV));
        let zsh_dir = match written {
            Ok(()) => Some(dir),
            Err(error) => {
                crate::logging::error_with_fields(
                    "room_terminal.shell_integration",
                    "zsh terminals will not report command completion: stub not written",
                    serde_json::json!({ "error": error.to_string() }),
                );
                None
            }
        };
        let bash_path = root.join("bash-startup.sh");
        let bash_startup = match std::fs::write(&bash_path, include_str!("bash_startup.sh")) {
            Ok(()) => Some(bash_path),
            Err(error) => {
                crate::logging::error_with_fields(
                    "room_terminal.shell_integration",
                    "bash terminals will not report command completion: stub not written",
                    serde_json::json!({ "error": error.to_string() }),
                );
                None
            }
        };
        Self {
            zsh_dir,
            bash_startup,
        }
    }

    pub(super) fn arguments(&self, shell: &str) -> Vec<String> {
        let mut args = Vec::new();
        if Path::new(shell)
            .file_name()
            .is_some_and(|name| name == "bash")
            && self.bash_startup.is_some()
        {
            // Run the normal login files from the first prompt hook, then
            // compose integration with what those files installed.
            args.extend(["--noprofile".into(), "--norc".into()]);
        }
        args.extend(["-l".into(), "-i".into()]);
        args
    }

    /// Environment that makes `shell` (a login shell path) report marks.
    pub(super) fn environment(&self, shell: &str) -> BTreeMap<String, String> {
        let name = Path::new(shell)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let mut env = BTreeMap::new();
        match name {
            "bash" => {
                if let Some(path) = &self.bash_startup {
                    let quoted = path.to_string_lossy().replace('\'', "'\\''");
                    env.insert("PROMPT_COMMAND".into(), format!("source '{quoted}'"));
                }
            }
            "zsh" => {
                if let Some(dir) = &self.zsh_dir {
                    env.insert(
                        "CHARIOX_ORIG_ZDOTDIR".to_string(),
                        std::env::var("ZDOTDIR").unwrap_or_default(),
                    );
                    env.insert("ZDOTDIR".to_string(), dir.display().to_string());
                }
            }
            _ => {}
        }
        env
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_marks_split_across_chunks_with_either_terminator() {
        let mut scanner = MarkScanner::default();
        let mut marks = scanner.feed(b"out\x1b]133");
        marks.extend(scanner.feed(b";C\x07more\x1b]133;D;7\x1b"));
        marks.extend(scanner.feed(b"\\\x1b]133;D\x07"));
        assert_eq!(
            marks,
            vec![
                ShellMark::CommandStarted,
                ShellMark::CommandFinished { exit_code: Some(7) },
                ShellMark::CommandFinished { exit_code: None },
            ]
        );
    }

    #[test]
    fn other_sequences_and_oversized_bodies_are_not_marks() {
        let mut scanner = MarkScanner::default();
        let mut input = b"\x1b]0;title\x07\x1b[31mred\x1b[0m\x1b]133;".to_vec();
        input.extend(vec![b'x'; 64]);
        input.extend(b"\x07\x1b]1337;D;1\x07");
        assert!(scanner.feed(&input).is_empty());
        assert_eq!(
            scanner.feed(b"\x1b]133;D;0\x07"),
            vec![ShellMark::CommandFinished { exit_code: Some(0) }]
        );
    }

    #[test]
    fn login_profile_can_replace_both_bash_hooks_without_losing_completion() {
        use crate::pty::{PtyManager, PtySpawnRequest};
        use std::time::{Duration, Instant};
        let bash = std::env::var("CHARIOX_TEST_BASH").unwrap_or_else(|_| "/bin/bash".into());
        for (index, hooks) in [
            "PROMPT_COMMAND=:",
            "PS0='profile-start '",
            "PROMPT_COMMAND=:\nPS0='profile-start '",
            "PROMPT_COMMAND='printf hook-one; printf hook-two'",
            "PROMPT_COMMAND=('printf hook-one' 'printf hook-two')",
            "declare -ax PROMPT_COMMAND=('printf hook-one' 'printf hook-two')",
            r#"hook=profile-hook; status_code=profile-status; PROMPT_COMMAND='printf "$hook/$status_code"'"#,
        ]
        .into_iter()
        .enumerate()
        {
            let root = std::env::temp_dir().join(format!(
                "chariox-hook-profile-{}-{index}",
                std::process::id()
            ));
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(
                root.join(".bash_profile"),
                format!("{hooks}\nalias profile_alias='echo profile-loaded'\n"),
            )
            .unwrap();
            let integration = ShellIntegration::install(&root.join("integration"));
            let mut env = integration.environment(&bash);
            env.insert("HOME".into(), root.display().to_string());
            env.insert("TERM".into(), "dumb".into());
            let mut pty = PtyManager::default();
            pty.spawn_interactive(PtySpawnRequest {
                process_key: "hooks".into(),
                provider_run_id: "hooks".into(),
                program: bash.clone(),
                args: integration.arguments(&bash),
                env,
                env_remove: vec![],
                working_directory: Some(root.clone()),
                cols: 100,
                rows: 30,
            })
            .unwrap();
            let mut scanner = MarkScanner::default();
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut initial = Vec::new();
            while Instant::now() < deadline && initial.is_empty() {
                for chunk in pty.drain_output("hooks").unwrap() {
                    initial.extend(scanner.feed(&chunk.bytes));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            pty.write_input("hooks", b"profile_alias; (exit 7)\r")
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut marks = Vec::new();
            let mut output = Vec::new();
            while Instant::now() < deadline
                && !marks.contains(&ShellMark::CommandFinished { exit_code: Some(7) })
            {
                for chunk in pty.drain_output("hooks").unwrap() {
                    marks.extend(scanner.feed(&chunk.bytes));
                    output.extend_from_slice(&chunk.bytes);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            pty.remove_process("hooks").unwrap();
            std::fs::remove_dir_all(&root).unwrap();
            assert!(
                String::from_utf8_lossy(&output).contains("profile-loaded"),
                "the login profile alias did not run"
            );
            if hooks.contains("hook-one") {
                let output = String::from_utf8_lossy(&output);
                assert!(output.contains("hook-onehook-two"), "{hooks}: {output}");
            }
            if hooks.contains("profile-hook") {
                let output = String::from_utf8_lossy(&output);
                assert!(output.contains("profile-hook/profile-status"), "{output}");
            }
            assert!(
                marks.contains(&ShellMark::CommandStarted),
                "{hooks}: {marks:?}"
            );
            assert!(
                marks.contains(&ShellMark::CommandFinished { exit_code: Some(7) }),
                "{hooks}: {marks:?}"
            );
        }
    }

    #[test]
    fn only_bash_and_zsh_are_given_hooks() {
        let root = std::env::temp_dir().join(format!("chariox-marks-{}", std::process::id()));
        let integration = ShellIntegration::install(&root);
        assert!(integration
            .environment("/bin/bash")
            .contains_key("PROMPT_COMMAND"));
        assert!(integration
            .environment("/usr/bin/zsh")
            .contains_key("ZDOTDIR"));
        assert!(integration.environment("/bin/sh").is_empty());
        assert!(root.join("zsh/.zshenv").is_file());
        let _ = std::fs::remove_dir_all(root);
    }
}
