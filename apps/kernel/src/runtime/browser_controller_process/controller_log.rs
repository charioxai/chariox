//! The browser controller's own log lines: of its stderr only the lines it marks as log lines reach the daemon log,
//! so a browser that stopped or reset itself says why. Everything else it writes stays unread.
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

/// Reads the controller's stderr until it closes.
pub(super) fn forward(stderr: impl Read) {
    for line in BufReader::new(stderr).lines() {
        let Ok(line) = line else { break };
        if let Some(text) = log_text(&line) {
            crate::logging::info_with_fields(
                "daemon.browser_controller",
                "browser controller log",
                serde_json::json!({ "event": text }),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::log_text;

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
}
