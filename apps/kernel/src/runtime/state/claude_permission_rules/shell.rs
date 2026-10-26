//! Splits a shell command into its simple commands, the way Claude Code does before it matches
//! permission rules: `&&`, `||`, `;`, `|`, `|&` and newlines separate commands, and a rule must
//! match each one.
//!
//! The reader is deliberately small. Anything it does not fully understand (expansions,
//! groups, here-documents, background jobs, comments, backslashes, a dangling operator) makes the
//! whole command unreadable, so no rule is ever derived from text that was only half understood.

/// One word of a simple command, with its quotes removed.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Word {
    pub text: String,
    pub quoted: bool,
}

/// A command cut apart: its simple commands with the redirections removed, and where the
/// redirections write or read (a file descriptor copy such as `2>&1` has no target).
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Script {
    pub commands: Vec<Vec<Word>>,
    pub redirect_targets: Vec<String>,
}

pub(super) fn read_script(command: &str) -> Option<Script> {
    Reader::default().read(command)
}

#[derive(Default)]
struct Reader {
    commands: Vec<Vec<Word>>,
    redirect_targets: Vec<String>,
    words: Vec<Word>,
    text: String,
    started: bool,
    quoted: bool,
    /// An `&&`, `||`, `|` or `|&` is waiting for the command that follows it.
    operator_open: bool,
}

impl Reader {
    fn read(mut self, command: &str) -> Option<Script> {
        let chars: Vec<char> = command.chars().collect();
        let mut at = 0;
        while at < chars.len() {
            let ch = chars[at];
            let next = chars.get(at + 1).copied();
            match ch {
                '\'' => at = self.single_quoted(&chars, at)?,
                '"' => at = self.double_quoted(&chars, at)?,
                ' ' | '\t' => {
                    self.end_word();
                    at += 1;
                }
                '\n' => {
                    self.end_word();
                    self.end_command_at_newline()?;
                    at += 1;
                }
                ';' => {
                    self.end_word();
                    self.end_command(false)?;
                    at += 1;
                }
                '&' if next == Some('&') => {
                    self.end_word();
                    self.end_command(true)?;
                    at += 2;
                }
                '&' if next == Some('>') => at = self.redirection(&chars, at, 1)?,
                '|' if next == Some('|') => {
                    self.end_word();
                    self.end_command(true)?;
                    at += 2;
                }
                '|' => {
                    self.end_word();
                    self.end_command(true)?;
                    at += if next == Some('&') { 2 } else { 1 };
                }
                '>' | '<' => at = self.redirection(&chars, at, 0)?,
                '$' | '`' | '(' | ')' | '{' | '}' | '\\' | '&' => return None,
                '#' if !self.started => return None,
                _ if ch.is_control() => return None,
                _ => {
                    self.push(ch);
                    at += 1;
                }
            }
        }
        self.end_word();
        if self.operator_open {
            return None;
        }
        if !self.words.is_empty() {
            self.commands.push(std::mem::take(&mut self.words));
        }
        (!self.commands.is_empty()).then_some(Script {
            commands: self.commands,
            redirect_targets: self.redirect_targets,
        })
    }

    fn push(&mut self, ch: char) {
        self.started = true;
        self.text.push(ch);
    }

    fn end_word(&mut self) {
        if self.started {
            self.operator_open = false;
            self.words.push(Word {
                text: std::mem::take(&mut self.text),
                quoted: std::mem::take(&mut self.quoted),
            });
            self.started = false;
        }
    }

    /// `operator` is true for `&&`, `||` and the pipes, which need a command on both sides.
    fn end_command(&mut self, operator: bool) -> Option<()> {
        if self.words.is_empty() {
            return None;
        }
        self.commands.push(std::mem::take(&mut self.words));
        self.operator_open = operator;
        Some(())
    }

    /// A blank line is nothing; after an operator the next line continues the command.
    fn end_command_at_newline(&mut self) -> Option<()> {
        if self.words.is_empty() {
            return Some(());
        }
        self.end_command(false)
    }

    fn single_quoted(&mut self, chars: &[char], at: usize) -> Option<usize> {
        let close = chars[at + 1..].iter().position(|ch| *ch == '\'')? + at + 1;
        self.started = true;
        self.quoted = true;
        self.text.extend(&chars[at + 1..close]);
        Some(close + 1)
    }

    fn double_quoted(&mut self, chars: &[char], at: usize) -> Option<usize> {
        let mut index = at + 1;
        self.started = true;
        self.quoted = true;
        while let Some(&ch) = chars.get(index) {
            match ch {
                '"' => return Some(index + 1),
                '`' => return None,
                '$' => return None,
                '\\' => {
                    self.text.push(*chars.get(index + 1)?);
                    index += 2;
                }
                _ => {
                    self.text.push(ch);
                    index += 1;
                }
            }
        }
        None
    }

    /// A redirection such as `2>&1`, `> file`, `>> file`, `&> file` or `< file`. Claude Code
    /// checks the target against its own file rules, so the command text drops it.
    fn redirection(&mut self, chars: &[char], at: usize, lead: usize) -> Option<usize> {
        let fd_only = self.started && self.text.chars().all(|ch| ch.is_ascii_digit());
        if self.started && !fd_only {
            self.end_word();
        } else {
            self.text.clear();
            self.started = false;
            self.quoted = false;
        }
        let mut index = at + lead;
        let operator = chars[index];
        index += 1;
        if operator == '<' && chars.get(index) == Some(&'<') {
            return None;
        }
        if operator == '>' && chars.get(index) == Some(&'>') {
            index += 1;
        }
        if operator == '>' && chars.get(index) == Some(&'&') && lead == 0 {
            index += 1;
            let digits = chars[index..]
                .iter()
                .take_while(|ch| ch.is_ascii_digit() || **ch == '-')
                .count();
            return (digits > 0).then_some(index + digits);
        }
        while matches!(chars.get(index), Some(' ' | '\t')) {
            index += 1;
        }
        let target_ends = |ch: &char| matches!(ch, ' ' | '\t' | '\n' | ';' | '&' | '|' | '<' | '>');
        let mut target = Reader::default();
        let mut end = index;
        while let Some(&ch) = chars.get(end) {
            match ch {
                '\'' => end = target.single_quoted(chars, end)?,
                '"' => end = target.double_quoted(chars, end)?,
                _ if target_ends(&ch) => break,
                '$' | '`' | '(' | ')' | '{' | '}' | '\\' => return None,
                _ => {
                    target.push(ch);
                    end += 1;
                }
            }
        }
        if !target.started {
            return None;
        }
        self.redirect_targets.push(target.text);
        Some(end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(command: &str) -> Option<Vec<Vec<String>>> {
        read_script(command).map(|script| {
            script
                .commands
                .into_iter()
                .map(|words| words.into_iter().map(|word| word.text).collect())
                .collect()
        })
    }

    #[test]
    fn separators_cut_a_command_into_the_simple_commands_a_rule_must_match() {
        assert_eq!(
            texts("cd app && cargo test -p x 2>&1 | tail -20; echo done"),
            Some(vec![
                vec!["cd".into(), "app".into()],
                vec!["cargo".into(), "test".into(), "-p".into(), "x".into()],
                vec!["tail".into(), "-20".into()],
                vec!["echo".into(), "done".into()],
            ])
        );
        assert_eq!(
            texts("a || b |& c\nd\n\n"),
            Some(vec![
                vec!["a".into()],
                vec!["b".into()],
                vec!["c".into()],
                vec!["d".into()]
            ])
        );
        assert_eq!(
            texts("a &&\n b"),
            Some(vec![vec!["a".into()], vec!["b".into()]])
        );
    }

    #[test]
    fn quotes_hide_separators_and_are_removed_from_the_words() {
        let commands = read_script("git commit -m 'a && b; c' \"d | e\"")
            .expect("readable")
            .commands;
        assert_eq!(commands.len(), 1);
        let words: Vec<_> = commands[0].iter().map(|word| word.text.as_str()).collect();
        assert_eq!(words, ["git", "commit", "-m", "a && b; c", "d | e"]);
        assert!(!commands[0][0].quoted && commands[0][3].quoted);
    }

    #[test]
    fn redirections_are_not_words() {
        assert_eq!(
            texts("cargo test >out.log 2>&1 </dev/null &> all.log >> 'x y'"),
            Some(vec![vec!["cargo".into(), "test".into()]])
        );
        assert_eq!(
            texts("echo a 2>/dev/null"),
            Some(vec![vec!["echo".into(), "a".into()]])
        );
        let script =
            read_script("cargo test >out.log 2>&1 </dev/null &> 'x y' | tail").expect("readable");
        assert_eq!(script.redirect_targets, ["out.log", "/dev/null", "x y"]);
    }

    #[test]
    fn text_the_reader_does_not_fully_understand_is_unreadable() {
        for command in [
            "echo $(id)",
            "echo `id`",
            "echo \"$(id)\"",
            "echo ${HOME}",
            "echo $HOME",
            "echo \"$?\"",
            "ls > $OUT",
            "(cd a; ls)",
            "{ ls; }",
            "cat <<EOF\nx\nEOF",
            "cat <<< x",
            "sleep 1 &",
            "ls # note",
            "echo a\\ b",
            "npm test &&",
            "&& npm test",
            "a ;; b",
            "a | | b",
            "echo 'open",
            "echo \"open",
            "echo a >",
            "echo >(ls)",
            "ls >&",
            "",
            "   ",
        ] {
            assert_eq!(texts(command), None, "{command:?}");
        }
    }
}
