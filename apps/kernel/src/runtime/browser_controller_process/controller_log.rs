//! The browser controller's own log lines: of its stderr the lines it marks as log lines reach the daemon log,
//! so a browser that stopped or reset itself says why. Of everything else only one line is kept: the last error
//! line, reported once when stderr closes, so an uncaught crash is not invisible.
use std::io::{BufRead, BufReader, Read};

const PREFIX: &str = "CHARIOX-LOG ";
const MAX_CHARS: usize = 160;

/// The text of a marked line, limited to printable ASCII and a fixed length; `None` for any other line.
pub(super) fn log_text(line: &str) -> Option<String> {
    let text = line.strip_prefix(PREFIX)?;
    let cleaned: String = text
        .chars()
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .take(MAX_CHARS)
        .collect();
    (!cleaned.is_empty()).then_some(cleaned)
}

/// An unmarked line that names an error (`Error: ...`, `TypeError: ...`), cleaned like a marked line, with words
/// that can carry a secret (URLs, long token-like runs) replaced; `None` for any other line.
pub(super) fn error_text(line: &str) -> Option<String> {
    let line = line.trim();
    let name = line.split(':').next()?;
    if !name.ends_with("Error") || !name.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let words: Vec<&str> = line
        .split_whitespace()
        .map(|word| {
            if word.contains("://") {
                "<url>"
            } else if word.len() >= 32
                && word
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_-+/=".contains(c))
            {
                "<redacted>"
            } else {
                word
            }
        })
        .collect();
    let cleaned: String = words
        .join(" ")
        .chars()
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .take(MAX_CHARS)
        .collect();
    (!cleaned.is_empty()).then_some(cleaned)
}

/// Reads the controller's stderr until it closes.
pub(super) fn forward(stderr: impl Read) {
    let mut last_error = None;
    for line in BufReader::new(stderr).lines() {
        let Ok(line) = line else { break };
        if let Some(text) = log_text(&line) {
            crate::logging::info_with_fields(
                "daemon.browser_controller",
                "browser controller log",
                serde_json::json!({ "event": text }),
            );
        } else if let Some(text) = error_text(&line) {
            last_error = Some(text);
        }
    }
    if let Some(text) = last_error {
        crate::logging::warn_with_fields(
            "daemon.browser_controller",
            "browser controller exited with an error",
            serde_json::json!({ "error": text }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{error_text, log_text};

    #[test]
    fn only_marked_lines_are_logged() {
        assert_eq!(
            log_text("CHARIOX-LOG browser_reset after browser_target_not_found").as_deref(),
            Some("browser_reset after browser_target_not_found")
        );
        assert_eq!(
            log_text("MP-11: kernel browser managed policies unavailable"),
            None
        );
        assert_eq!(
            log_text("Error: something at https://example.com/page"),
            None
        );
        assert_eq!(log_text("CHARIOX-LOG "), None);
    }

    #[test]
    fn a_marked_line_is_printable_ascii_and_bounded() {
        let long = format!("CHARIOX-LOG {}\u{1b}[31mx\u{e9}", "a".repeat(400));
        let text = log_text(&long).unwrap();
        assert_eq!(text.chars().count(), 160);
        assert!(text.chars().all(|c| c.is_ascii_graphic() || c == ' '));
        assert_eq!(
            log_text("CHARIOX-LOG a\u{1b}[31m b\u{e9}").as_deref(),
            Some("a[31m b")
        );
    }

    #[test]
    fn a_crash_keeps_only_its_error_line_without_urls_or_tokens() {
        let crash = "/x/kernel-browser-timing.mjs:25\n    ^\n\nError: ENOENT: no such file or directory, open '/x/display-timing.jsonl'\n    at Timeout.flush (/x/kernel-browser-timing.mjs:25:33)\n\nNode.js v22.22.1";
        let errors: Vec<String> = crash.lines().filter_map(error_text).collect();
        assert_eq!(
            errors,
            ["Error: ENOENT: no such file or directory, open '/x/display-timing.jsonl'"]
        );
        assert_eq!(
            error_text("TypeError: fetch failed for https://example.com/a?token=abc").as_deref(),
            Some("TypeError: fetch failed for <url>")
        );
        let token = "a".repeat(40);
        assert_eq!(
            error_text(&format!("Error: bad key {token}")).as_deref(),
            Some("Error: bad key <redacted>")
        );
        assert_eq!(
            error_text(&format!("Error: {}", "b ".repeat(200)))
                .unwrap()
                .len(),
            160
        );
        assert_eq!(error_text("Node.js v22.22.1"), None);
        assert_eq!(error_text("at Timeout.flush (/x/a.mjs:25:33)"), None);
        assert_eq!(error_text("CHARIOX-LOG Error: marked"), None);
    }
}
