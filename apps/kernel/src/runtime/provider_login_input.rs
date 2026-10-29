//! Claude's browser callback contains a code and state, separated by `#`.
//! Validate before writing to the CLI so an incomplete paste can be retried.
use crate::error::DaemonError;
use zeroize::Zeroizing;

pub(super) fn claude_authorization_input(input: &[u8]) -> Result<Zeroizing<Vec<u8>>, DaemonError> {
    let input = std::str::from_utf8(input).map_err(|_| DaemonError::ProviderLoginCodeInvalid)?;
    let input = Zeroizing::new(input.replace("\x1b[200~", "").replace("\x1b[201~", ""));
    if input
        .chars()
        .any(|value| value.is_control() && !value.is_whitespace())
    {
        return Err(DaemonError::ProviderLoginCodeInvalid);
    }
    let trimmed = input.trim();
    // A wrapped code has no prose separators. Keep prose line boundaries so
    // words such as "Copy" cannot become part of the state.
    let wrapped = trimmed.contains(['\r', '\n'])
        && trimmed
            .chars()
            .all(|value| code_character(value) || value == '#' || matches!(value, '\r' | '\n'));
    if wrapped
        && trimmed.lines().any(|line| {
            line.split_once('#')
                .is_some_and(|(code, state)| !code.is_empty() && !state.is_empty())
        })
    {
        // Additional code-like lines could be wrapped fragments or surrounding
        // words. Do not silently drop them or append them to a complete state.
        return Err(DaemonError::ProviderLoginCodeInvalid);
    }
    let joined;
    let text = if wrapped {
        joined = Zeroizing::new(trimmed.replace(['\r', '\n'], ""));
        joined.as_str()
    } else {
        trimmed
    };
    let mut candidates = text
        .split_whitespace()
        .filter(|value| value.contains('#'))
        .map(|value| {
            value.trim_matches([
                '\"', '\'', '<', '>', '(', ')', '[', ']', '{', '}', ',', '.', ';', ':',
            ])
        });
    let Some(candidate) = candidates.next() else {
        return Err(DaemonError::ProviderLoginCodeIncomplete);
    };
    if candidates.next().is_some() {
        return Err(DaemonError::ProviderLoginCodeInvalid);
    }
    let (code, state) = candidate.split_once('#').expect("candidate contains #");
    if state.contains('#') || !code.chars().chain(state.chars()).all(code_character) {
        return Err(DaemonError::ProviderLoginCodeInvalid);
    }
    if code.is_empty() || state.is_empty() {
        return Err(DaemonError::ProviderLoginCodeIncomplete);
    }
    let mut bytes = candidate.as_bytes().to_vec();
    bytes.push(b'\r');
    Ok(Zeroizing::new(bytes))
}

fn code_character(value: char) -> bool {
    value.is_ascii_alphanumeric() || matches!(value, '_' | '-' | '+' | '/' | '=')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_callback_preserves_state_and_normalizes_a_copied_code() {
        for value in [
            "abc123#def456\r",
            " \nabc123#def456\r\n ",
            "Your code: abc123#def456. Copy it into the terminal.",
            "abc123\n#\ndef456",
            "Your code:\nabc123#def456\nCopy it",
            "\x1b[200~abc123#def456\x1b[201~\r",
        ] {
            assert_eq!(
                claude_authorization_input(value.as_bytes())
                    .unwrap()
                    .as_slice(),
                b"abc123#def456\r"
            );
        }
    }

    #[test]
    fn incomplete_or_ambiguous_claude_callbacks_are_not_sent_to_the_cli() {
        for value in ["abc123\r", "abc123#", "#def456", " \n"] {
            assert!(matches!(
                claude_authorization_input(value.as_bytes()),
                Err(DaemonError::ProviderLoginCodeIncomplete)
            ));
        }
        for value in [
            "abc#def#ghi",
            "abc#def other#state",
            "abc#def\x03",
            "abc123#def456\nCopy",
            "abc123\n456#def",
        ] {
            assert!(matches!(
                claude_authorization_input(value.as_bytes()),
                Err(DaemonError::ProviderLoginCodeInvalid)
            ));
        }
        assert_eq!(
            claude_authorization_input(b"abc#def+ghi")
                .unwrap()
                .as_slice(),
            b"abc#def+ghi\r"
        );
        assert!(matches!(
            claude_authorization_input(&[0xff]),
            Err(DaemonError::ProviderLoginCodeInvalid)
        ));
    }
}
