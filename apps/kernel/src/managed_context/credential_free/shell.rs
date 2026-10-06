//! MP-08/MP-11: inspect shell words without executing or expanding input.

pub(super) fn sensitive(key: &str) -> bool {
    let key = key.to_ascii_lowercase().replace('-', "_");
    [
        "password",
        "passwd",
        "passphrase",
        "secret",
        "token",
        "credential",
        "private_key",
        "api_key",
        "apikey",
        "authorization",
        "access_key",
    ]
    .iter()
    .any(|name| key.contains(name))
        || key.split('_').any(|part| {
            matches!(
                part,
                "auth" | "oauth" | "oauth2" | "bearer" | "cookie" | "cookies" | "pass" | "pwd"
            )
        })
}

fn safe_flag(flag: &str) -> bool {
    matches!(
        flag,
        "max_tokens"
            | "max_input_tokens"
            | "max_output_tokens"
            | "token_limit"
            | "user_agent"
            | "user_data_dir"
    )
}

fn credential_word(word: &str) -> bool {
    let word = word.trim_matches(['\'', '"', ',', ';', '{', '}', '[', ']']);
    let word = word.strip_prefix("$env:").unwrap_or(word);
    if let Some(flag) = word.strip_prefix("--") {
        let flag = flag
            .split('=')
            .next()
            .unwrap_or(flag)
            .to_ascii_lowercase()
            .replace('-', "_");
        // MP-11: inspect all long flags by meaning, not a suffix-only denylist.
        if !safe_flag(&flag)
            && (sensitive(&flag)
                || flag.contains(['$', '`'])
                || flag
                    .split(|ch: char| !ch.is_ascii_alphanumeric())
                    .any(|part| {
                        matches!(
                            part,
                            "auth" | "oauth" | "oauth2" | "bearer" | "cookie" | "cookies"
                        )
                    }))
        {
            return true;
        }
    }
    // MP-11: inspect every assignment, including append/indexed assignments and
    // assignments nested in data arguments. Dynamic names are unclassifiable.
    for (index, ch) in word.char_indices() {
        if matches!(ch, '=' | ':') {
            let key = word[..index]
                .rsplit(['=', ':'])
                .next()
                .unwrap_or("")
                .trim_matches(['{', '}', '[', ']', ',']);
            let count_or_path = key
                .strip_prefix("--")
                .is_some_and(|flag| safe_flag(&flag.to_ascii_lowercase().replace('-', "_")));
            if (!count_or_path && sensitive(key)) || (ch == '=' && key.contains(['$', '`'])) {
                return true;
            }
        }
    }
    // MP-11: URLs may be a whole argument or the value of an assignment/flag.
    if [Some(word), word.split_once('=').map(|(_, value)| value)]
        .into_iter()
        .flatten()
        .any(|value| url::Url::parse(value).is_ok_and(|url| super::credential_url(&url)))
    {
        return true;
    }
    word.strip_prefix('-')
        .is_some_and(|flags| !flags.starts_with('-') && sensitive(flags))
}

fn raw_credential_words(text: &str) -> bool {
    text.split(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                '\'' | '"' | ',' | ';' | '|' | '&' | '(' | ')' | '{' | '}' | '[' | ']'
            )
    })
    .any(credential_word)
}

// MP-11: shell-words handles quote concatenation, escapes and continuations.
// Separate operators outside quotes first; the tokenizer does not split them.
fn shell_word_input(text: &str) -> String {
    let mut input = String::with_capacity(text.len());
    let mut quote = None;
    let mut escaped = false;
    for ch in text.chars() {
        if quote.is_none() && !escaped && matches!(ch, ';' | '|' | '&' | '(' | ')' | '\n') {
            input.push_str(" ; ");
        } else {
            input.push(if quote.is_none() && !escaped && matches!(ch, '<' | '>') {
                ' '
            } else {
                ch
            });
        }
        if escaped {
            escaped = false;
        } else if ch == '\\' && quote != Some('\'') {
            escaped = true;
        } else if quote == Some(ch) {
            quote = None;
        } else if quote.is_none() && matches!(ch, '\'' | '"') {
            quote = Some(ch);
        }
    }
    input
}

// MP-11: short options have meaning only for their executable. Stop curl
// bundles at options taking a value, so -ooutput and -XPUT remain ordinary.
fn command_credentials(words: &[String]) -> bool {
    let mut command = None;
    for word in words {
        if word == ";" {
            command = None;
            continue;
        }
        let executable = word.rsplit('/').next().unwrap_or(word);
        if matches!(executable, "curl" | "wget") {
            command = Some(executable);
            continue;
        }
        match command {
            Some("curl") => {
                if word == "--" {
                    command = None;
                    continue;
                }
                if word == "--user"
                    || word.starts_with("--user=")
                    || word == "--proxy-user"
                    || word.starts_with("--proxy-user=")
                {
                    return true;
                }
                if let Some(flags) = word
                    .strip_prefix('-')
                    .filter(|flags| !flags.starts_with('-'))
                {
                    for flag in flags.chars() {
                        if matches!(flag, 'u' | 'U' | 'b') {
                            return true;
                        }
                        if "ACdDeEFHKmopPrtTwXxyYz".contains(flag) {
                            break;
                        }
                    }
                }
            }
            Some("wget") => {
                if word == "--user"
                    || word.starts_with("--user=")
                    || word == "--http-user"
                    || word.starts_with("--http-user=")
                    || word == "--ftp-user"
                    || word.starts_with("--ftp-user=")
                {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn inspect_words(text: &str, depth: usize) -> bool {
    // MP-11: raw candidates survive comments and syntax the POSIX lexer cannot
    // decode. ANSI-C/localized quoting is unsupported and fails closed.
    let raw_words = shell_word_input(text)
        .split(|ch: char| {
            ch.is_whitespace() || matches!(ch, '\'' | '"' | ',' | '{' | '}' | '[' | ']')
        })
        .map(str::to_string)
        .collect::<Vec<_>>();
    if raw_credential_words(text)
        || command_credentials(&raw_words)
        || text.contains("$'")
        || text.contains("$\"")
    {
        return true;
    }
    let Ok(words) = shell_words::split(&shell_word_input(text)) else {
        return false; // MP-11: raw candidates above remain checked for non-shell files.
    };
    command_credentials(&words)
        || words.iter().any(|word| {
            credential_word(word)
                || (word != text
                    && word.contains([' ', '\t', '\n', ';', '|', '&', '\'', '"', '\\'])
                    && (depth >= 4 || inspect_words(word, depth + 1)))
        })
}

pub(super) fn credential_text(text: &str) -> bool {
    // MP-11: retain spaced/dotted/JSON/PowerShell assignment detection.
    text.lines().any(|line| {
        let line = line.trim();
        let line = line
            .strip_prefix("export ")
            .or_else(|| line.strip_prefix("set "))
            .unwrap_or(line);
        let line = line.strip_prefix("$env:").unwrap_or(line);
        let Some(index) = line.find(['=', ':']) else {
            return false;
        };
        let key = line[..index].trim().trim_matches(['\'', '"', '{', ' ']);
        let key = key.rsplit('.').next().unwrap_or(key);
        key.bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            && !key
                .strip_prefix("--")
                .is_some_and(|flag| safe_flag(&flag.to_ascii_lowercase().replace('-', "_")))
            && sensitive(key)
    }) || inspect_words(text, 0)
}

pub(super) fn unsupported_shell(path: &str, text: &str) -> bool {
    // MP-11: do not admit malformed shell files using only the lexical fallback.
    matches!(path.rsplit('.').next(), Some("sh" | "bash" | "zsh"))
        && shell_words::split(&shell_word_input(text)).is_err()
}
