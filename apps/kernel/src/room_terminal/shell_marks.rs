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

/// bash gets its hooks as `PS0` (before a command runs) and `PROMPT_COMMAND`
/// (before the prompt). They stop being exported at the first prompt, so
/// shells started inside the terminal do not repeat them.
const BASH_PS0: &str = "\\e]133;C\\a";
const BASH_PROMPT_COMMAND: &str =
    "printf '\\033]133;D;%s\\007' \"$?\"; export -n PROMPT_COMMAND PS0";

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
        Self { zsh_dir }
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
                env.insert("PS0".to_string(), BASH_PS0.to_string());
                env.insert(
                    "PROMPT_COMMAND".to_string(),
                    BASH_PROMPT_COMMAND.to_string(),
                );
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
    fn only_bash_and_zsh_are_given_hooks() {
        let root = std::env::temp_dir().join(format!("chariox-marks-{}", std::process::id()));
        let integration = ShellIntegration::install(&root);
        assert!(integration.environment("/bin/bash").contains_key("PS0"));
        assert!(integration
            .environment("/usr/bin/zsh")
            .contains_key("ZDOTDIR"));
        assert!(integration.environment("/bin/sh").is_empty());
        assert!(root.join("zsh/.zshenv").is_file());
        let _ = std::fs::remove_dir_all(root);
    }
}
