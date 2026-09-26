use zeroize::Zeroizing;

/// The PTY `claude setup-token` runs in. Ink wraps at the terminal width, so
/// the sign-in URL and the token each stay on one row at this width.
pub(in crate::runtime) const CLAUDE_SETUP_TOKEN_ROWS: u16 = 40;
pub(in crate::runtime) const CLAUDE_SETUP_TOKEN_COLUMNS: u16 = 1_000;

/// Any credential-shaped run on screen is hidden, captured or not.
const CREDENTIAL_PREFIX: &str = "sk-ant-";
/// Only a Claude Code OAuth token is ever captured for storage.
const SETUP_TOKEN_PREFIX: &str = "sk-ant-oat01-";
const MIN_SETUP_TOKEN_BODY_CHARS: usize = 80;
pub(super) const REDACTED_TOKEN_MARKER: &str = "[setup token captured; hidden]";

#[derive(Debug, PartialEq, Eq)]
pub(in crate::runtime) enum SetupTokenScan {
    /// No complete token is on screen yet.
    Pending,
    Found(Zeroizing<String>),
    /// More than one distinct token-shaped value; refuse to guess.
    Ambiguous,
}

/// Renders `claude setup-token` PTY output through a terminal emulator. Ink
/// positions text with cursor movement, so neither redaction nor capture can
/// work on raw bytes; both operate on the rendered screen, and only redacted
/// screen text is ever projected.
pub(in crate::runtime) struct ClaudeSetupTokenScreen {
    parser: vt100::Parser,
}

impl Default for ClaudeSetupTokenScreen {
    fn default() -> Self {
        Self {
            parser: vt100::Parser::new(CLAUDE_SETUP_TOKEN_ROWS, CLAUDE_SETUP_TOKEN_COLUMNS, 0),
        }
    }
}

impl ClaudeSetupTokenScreen {
    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    pub fn redacted_text(&self) -> String {
        let rows = self.rows();
        let mut text = rows
            .iter()
            .map(|row| redact_credentials(row))
            .collect::<Vec<_>>()
            .join("\n");
        text.truncate(text.trim_end().len());
        text
    }

    /// A token counts only once it is followed by more output (on its row or
    /// a later one) or the provider process exited, so a token still being
    /// printed is never captured truncated.
    pub fn scan(&self, exited: bool) -> SetupTokenScan {
        let rows = self.rows();
        let mut found: Option<Zeroizing<String>> = None;
        for (index, row) in rows.iter().enumerate() {
            let mut rest = row.as_str();
            while let Some(offset) = rest.find(SETUP_TOKEN_PREFIX) {
                let candidate = &rest[offset..];
                let end = SETUP_TOKEN_PREFIX.len()
                    + candidate[SETUP_TOKEN_PREFIX.len()..]
                        .find(|character: char| !is_token_char(character))
                        .unwrap_or(candidate.len() - SETUP_TOKEN_PREFIX.len());
                let terminated = end < candidate.len()
                    || exited
                    || rows[index + 1..].iter().any(|later| !later.is_empty());
                if !terminated {
                    return SetupTokenScan::Pending;
                }
                if end - SETUP_TOKEN_PREFIX.len() >= MIN_SETUP_TOKEN_BODY_CHARS {
                    let token = &candidate[..end];
                    match found.as_ref() {
                        Some(existing) if existing.as_str() != token => {
                            return SetupTokenScan::Ambiguous;
                        }
                        Some(_) => {}
                        None => found = Some(Zeroizing::new(token.to_string())),
                    }
                }
                rest = &candidate[end..];
            }
        }
        found.map_or(SetupTokenScan::Pending, SetupTokenScan::Found)
    }

    fn rows(&self) -> Vec<Zeroizing<String>> {
        self.parser
            .screen()
            .rows(0, CLAUDE_SETUP_TOKEN_COLUMNS)
            .map(|row| Zeroizing::new(row.trim_end().to_string()))
            .collect()
    }
}

fn is_token_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
}

fn redact_credentials(row: &str) -> String {
    let mut redacted = String::with_capacity(row.len());
    let mut rest = row;
    while let Some(offset) = rest.find(CREDENTIAL_PREFIX) {
        redacted.push_str(&rest[..offset]);
        redacted.push_str(REDACTED_TOKEN_MARKER);
        let after = &rest[offset + CREDENTIAL_PREFIX.len()..];
        rest = &after[after
            .find(|character: char| !is_token_char(character))
            .unwrap_or(after.len())..];
    }
    redacted.push_str(rest);
    redacted
}

#[cfg(test)]
mod tests {
    use super::*;

    /// First half of a real Claude Code 2.1.281 `setup-token` session in a
    /// 40x1000 PTY, up to the code prompt. OAuth state values are replaced.
    const RECORDED_START: &[u8] = include_bytes!("testdata/claude-setup-token-2.1.281-start.pty");
    const TOKEN: &str = "sk-ant-oat01-TESTONLYaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa_bbbb-cccc-dd";

    /// The token half of the session in the style Ink uses: column moves
    /// instead of spaces, styled spans, and a style change inside the token.
    fn recorded_finish() -> Vec<u8> {
        let (head, tail) = TOKEN.split_at(31);
        format!(
            "\x1b[2G\x1b[32m\u{2713}\x1b[39m\x1b[4GLong-lived\x1b[15Gauthentication\x1b[30Gtoken\x1b[36Gcreated\x1b[44Gsuccessfully!\r\r\n\r\r\n\
             \x1b[2GYour\x1b[7GOAuth\x1b[13Gtoken\x1b[19G(valid\x1b[26Gfor\x1b[30G1\x1b[32Gyear):\r\r\n\r\r\n\
             \x1b[2G\x1b[33m{head}\x1b[1m{tail}\x1b[22m\x1b[39m\r\r\n\r\r\n\
             \x1b[2GStore\x1b[8Gthis\x1b[13Gtoken\x1b[19Gsecurely.\r\r\n"
        )
        .into_bytes()
    }

    fn session() -> Vec<u8> {
        let mut bytes = RECORDED_START.to_vec();
        bytes.extend(recorded_finish());
        bytes
    }

    #[test]
    fn recorded_session_renders_readable_text_and_its_authorization_url() {
        let mut screen = ClaudeSetupTokenScreen::default();
        screen.process(RECORDED_START);
        let text = screen.redacted_text();

        assert!(text.contains("Welcome to Claude Code v2.1.281"), "{text}");
        assert!(text.contains("Paste code here if prompted >"), "{text}");
        assert!(text.contains("https://claude.com/cai/oauth/authorize?code=true"));
        assert!(!text.contains('\x1b'));
        assert_eq!(screen.scan(false), SetupTokenScan::Pending);
    }

    #[test]
    fn recorded_session_captures_the_token_at_every_chunk_boundary_without_projecting_it() {
        let bytes = session();
        let token_start = RECORDED_START.len();
        for split in (token_start..bytes.len())
            .step_by(7)
            .chain([bytes.len() - 1])
        {
            let mut screen = ClaudeSetupTokenScreen::default();
            screen.process(&bytes[..split]);
            let partial = screen.redacted_text();
            assert!(!partial.contains("TESTONLY"), "split {split}: {partial}");
            if let SetupTokenScan::Found(token) = screen.scan(false) {
                assert_eq!(
                    token.as_str(),
                    TOKEN,
                    "split {split} captured a truncated token"
                );
            }
            screen.process(&bytes[split..]);
            let text = screen.redacted_text();
            assert!(!text.contains("TESTONLY"), "split {split}: {text}");
            assert!(
                text.contains(REDACTED_TOKEN_MARKER),
                "split {split}: {text}"
            );
            assert!(text.contains("Store this token securely."));
            assert_eq!(
                screen.scan(false),
                SetupTokenScan::Found(Zeroizing::new(TOKEN.to_string()))
            );
        }
    }

    #[test]
    fn a_token_on_the_last_row_is_captured_only_after_the_provider_exits() {
        let mut screen = ClaudeSetupTokenScreen::default();
        screen.process(format!("token:\r\n{TOKEN}").as_bytes());

        assert_eq!(screen.scan(false), SetupTokenScan::Pending);
        assert_eq!(
            screen.scan(true),
            SetupTokenScan::Found(Zeroizing::new(TOKEN.to_string()))
        );
        assert!(!screen.redacted_text().contains("TESTONLY"));
    }

    #[test]
    fn only_complete_oauth_tokens_are_captured_but_every_credential_is_hidden() {
        let mut screen = ClaudeSetupTokenScreen::default();
        screen.process(
            b"short sk-ant-oat01-abc\r\napi sk-ant-api03-TESTONLYxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\r\ndone\r\n",
        );

        assert_eq!(screen.scan(true), SetupTokenScan::Pending);
        let text = screen.redacted_text();
        assert!(!text.contains("sk-ant"), "{text}");
        assert!(text.contains("short [setup token captured; hidden]"));
    }

    #[test]
    fn two_distinct_tokens_are_refused() {
        let other = TOKEN.replace("bbbb", "zzzz");
        let mut screen = ClaudeSetupTokenScreen::default();
        screen.process(format!("{TOKEN}\r\n{other}\r\ndone\r\n").as_bytes());

        assert_eq!(screen.scan(true), SetupTokenScan::Ambiguous);
    }
}
