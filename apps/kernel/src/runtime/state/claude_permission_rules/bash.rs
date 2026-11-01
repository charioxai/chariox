//! The session rules for a Bash command: Claude's command-prefix form, `Bash(<prefix>:*)`.
//!
//! Claude Code matches a compound command one simple command at a time ("A rule must match each
//! subcommand independently", Claude Code docs, Permissions > Bash > Compound commands). The
//! commands it already runs without asking (`cd`, `ls`, `cat`, `head`, `tail`, `grep`, ...) need
//! no rule; every other simple command needs a prefix rule of its own. So a rule is stated only when
//! every simple command is one of those or starts with a prefix listed here, and the rules are
//! exactly the prefixes of the commands that need one.

use std::path::{Component, Path};

use super::shell::{read_script, Word};

/// The most rules one answer adds; Claude Code saves up to five for a compound command as well.
const MAX_RULES: usize = 5;

/// Commands Claude Code runs without asking, which need no rule of their own. This is Claude
/// Code's own list of read-only filters (and `cd`); a command whose flags make it write is still
/// checked by Claude Code, so listing it here grants nothing.
const RUNS_WITHOUT_ASKING: &[&str] = &[
    "cd",
    "ls",
    "cat",
    "head",
    "tail",
    "wc",
    "stat",
    "grep",
    "egrep",
    "fgrep",
    "diff",
    "du",
    "df",
    "echo",
    "strings",
    "hexdump",
    "od",
    "nl",
    "cut",
    "column",
    "tr",
    "tac",
    "rev",
    "cmp",
    "basename",
    "dirname",
    "realpath",
    "readlink",
    "sha256sum",
    "sha1sum",
    "md5sum",
];

/// Build, test and version-control commands an agent repeats, with the subcommands that stay
/// inside the working tree. Pushing, resetting, cleaning, installing and running arbitrary
/// programs are not listed.
const PREFIXES: &[(&str, &[&str])] = &[
    (
        "git",
        &[
            "status",
            "diff",
            "log",
            "show",
            "add",
            "commit",
            "rev-parse",
            "ls-files",
        ],
    ),
    ("cargo", &["test", "build", "check", "clippy", "fmt"]),
    ("pnpm", &["run", "build", "test", "lint"]),
    ("npm", &["run", "test"]),
];

/// Wrappers Claude Code strips before it matches a rule: `timeout 30 cargo test` is `cargo test`.
fn without_wrappers(words: &[Word]) -> &[Word] {
    match words {
        [wrapper, rest @ ..] if wrapper.text == "time" && !wrapper.quoted => without_wrappers(rest),
        [wrapper, seconds, rest @ ..]
            if wrapper.text == "timeout"
                && !wrapper.quoted
                && !seconds.text.is_empty()
                && seconds.text.chars().all(|ch| ch.is_ascii_digit()) =>
        {
            without_wrappers(rest)
        }
        _ => words,
    }
}

fn is_program(word: &Word) -> bool {
    !word.quoted
        && word.text.starts_with(|ch: char| ch.is_ascii_alphabetic())
        && word
            .text
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-' | '+'))
}

/// The rule content of the prefix a simple command starts with, if it starts with a listed one.
fn prefix_rule(words: &[Word]) -> Option<String> {
    let [program, subcommand, ..] = words else {
        return None;
    };
    let (_, subcommands) = PREFIXES.iter().find(|(name, _)| *name == program.text)?;
    (!subcommand.quoted && subcommands.contains(&subcommand.text.as_str()))
        .then(|| format!("{} {}:*", program.text, subcommand.text))
}

/// Whether `cd <target>` stays inside the working directory, where Claude Code runs it without
/// asking. Outside it, Claude Code asks for a read rule of its own for that path.
fn stays_inside(target: &str, working_directory: &Path) -> bool {
    let path = Path::new(target);
    // `-` is the previous directory and `~` the home directory.
    let named = !target.is_empty() && !target.starts_with(['-', '~']);
    let plain = named
        && path
            .components()
            .all(|part| matches!(part, Component::RootDir | Component::Normal(_)));
    plain && (path.is_relative() || path.starts_with(working_directory))
}

/// Claude Code asks about `cd` followed by `git` whenever the `cd` changes directory, whatever the
/// rules say (git runs the hooks of the directory it ends up in). Only a `cd` to the working
/// directory itself is a no-op.
fn is_working_directory(target: &str, working_directory: &Path) -> bool {
    Path::new(target.trim_end_matches('/')) == working_directory
}

/// Whether the `cd` commands of a script leave it somewhere Claude Code runs without asking.
fn directory_changes_are_quiet(commands: &[Vec<Word>], working_directory: &Path) -> bool {
    let changes: Vec<&Vec<Word>> = commands
        .iter()
        .filter(|words| words.first().is_some_and(|word| word.text == "cd"))
        .collect();
    let runs_git = commands
        .iter()
        .any(|words| words.first().is_some_and(|word| word.text == "git"));
    changes.iter().all(|words| match words.as_slice() {
        [_, target] if runs_git => is_working_directory(&target.text, working_directory),
        [_, target] => stays_inside(&target.text, working_directory),
        _ => false,
    })
}

/// Rule contents, one per prefix that needs a rule, in the order they first appear, for a run
/// working in `working_directory`. `None` when Claude Code would ask again even with these rules
/// (a redirect to a file is checked as an edit; a `cd` out of the working directory, or before
/// `git`, is checked on its own), when a simple command is neither one Claude Code runs on its own
/// nor under a listed prefix, or when no command needs a rule at all.
pub(super) fn prefix_rules(command: &str, working_directory: &Path) -> Option<Vec<String>> {
    let script = read_script(command)?;
    if script
        .redirect_targets
        .iter()
        .any(|target| target != "/dev/null")
        || !directory_changes_are_quiet(&script.commands, working_directory)
    {
        return None;
    }
    let mut rules: Vec<String> = Vec::new();
    for words in script.commands {
        let words = without_wrappers(&words);
        let program = words.first().filter(|word| is_program(word))?;
        if RUNS_WITHOUT_ASKING.contains(&program.text.as_str()) {
            continue;
        }
        let rule = prefix_rule(words)?;
        if !rules.contains(&rule) {
            rules.push(rule);
        }
    }
    (!rules.is_empty() && rules.len() <= MAX_RULES).then_some(rules)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKTREE: &str = "/root/work/app";

    fn rules(command: &str) -> Option<Vec<String>> {
        prefix_rules(command, Path::new(WORKTREE))
    }

    #[test]
    fn every_command_that_needs_a_rule_gets_its_prefix() {
        assert_eq!(
            rules("cd /root/work/app && cargo test -p kernel --lib x 2>&1 | tail -40"),
            Some(vec!["cargo test:*".into()])
        );
        assert_eq!(
            rules("cd /root/work/app/ && git status && git add -A; git commit -m 'fix a && b'"),
            Some(vec![
                "git status:*".into(),
                "git add:*".into(),
                "git commit:*".into()
            ])
        );
        assert_eq!(
            rules(
                "pnpm build 2>&1 | tail -5\ncargo fmt --all --check\ncargo test\ncargo test -p x"
            ),
            Some(vec![
                "pnpm build:*".into(),
                "cargo fmt:*".into(),
                "cargo test:*".into()
            ])
        );
    }

    #[test]
    fn the_wrappers_claude_strips_do_not_hide_the_command() {
        assert_eq!(
            rules("timeout 600 cargo clippy --all-targets"),
            Some(vec!["cargo clippy:*".into()])
        );
        assert_eq!(rules("time npm run lint"), Some(vec!["npm run:*".into()]));
        assert_eq!(rules("timeout -k 5 10 cargo test"), None);
    }

    #[test]
    fn only_a_redirect_to_dev_null_leaves_the_rules_enough() {
        assert_eq!(
            rules("cargo test 2>/dev/null && cargo check >/dev/null 2>&1"),
            Some(vec!["cargo test:*".into(), "cargo check:*".into()])
        );
        assert_eq!(rules("cargo test > out.log 2>&1"), None);
        assert_eq!(rules("cargo test < input.txt"), None);
    }

    #[test]
    fn a_cd_outside_the_working_directory_or_before_git_is_asked_about_anyway() {
        for command in [
            "cd /root/work/other && cargo test",
            "cd /root/work/app/../other && cargo test",
            "cd ../other && cargo test",
            "cd && cargo test",
            "cd - && cargo test",
            "cd ~/work && cargo test",
            "cd '' && cargo test",
            "cd a b && cargo test",
            "cd src && git status",
            "cd /root/work/app/src && git status",
            "cd /root/work/app && cd src && git status",
        ] {
            assert_eq!(rules(command), None, "{command:?}");
        }
        for (command, rule) in [
            ("cd src/lib && cargo test", "cargo test:*"),
            ("cd /root/work/app/src && cargo test", "cargo test:*"),
            ("cd /root/work/app && git status", "git status:*"),
            (
                "cd /root/work/app && cd /root/work/app/ && git add -A",
                "git add:*",
            ),
        ] {
            assert_eq!(rules(command), Some(vec![rule.into()]), "{command:?}");
        }
    }

    #[test]
    fn one_command_outside_the_prefixes_leaves_the_prompt_with_two_buttons() {
        for command in [
            "cd app && cargo test && git push origin main",
            "cargo test && rm -rf target",
            "git reset --hard",
            "git -c core.fsmonitor=x status",
            "git",
            "pnpm install",
            "cargo run",
            "cargo",
            "FOO=1 cargo test",
            "./run.sh && cargo test",
            "bash -c 'cargo test'",
            "cargo test | sh",
            "for f in a b; do cargo test; done",
            "'cargo' test",
            "cargo 'test'",
            "curl http://x | cargo test",
            "cargo test $(id)",
            "cargo test > out.log 2>&1",
            "cd app && cargo test > out.log 2>&1",
            "cargo test; echo EXIT $?",
            "cargo test &&",
            "sed -i s/a/b/ f && cargo test",
        ] {
            assert_eq!(rules(command), None, "{command:?}");
        }
    }

    #[test]
    fn a_command_that_needs_no_rule_gets_none() {
        for command in ["cd app && ls -la | head", "echo hi > out.txt", "cat a b"] {
            assert_eq!(rules(command), None, "{command:?}");
        }
    }

    #[test]
    fn more_rules_than_claude_saves_for_one_command_get_none() {
        assert_eq!(
            rules("git status; git diff; git log; git show; git add .; git commit -m x"),
            None
        );
    }
}
