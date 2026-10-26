//! MP-08/MP-11: JSON keys are classified by schema role; values stay inspected.
use super::{credential_text, credential_url, refused};
use crate::error::DaemonError;
use base64::Engine;
use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};
use std::fmt;

#[derive(Clone, Copy)]
pub(super) enum Role {
    Fields,
    Package,
    Lock,
    Names,
    PackageNames,
    Entries,
    LockEntry,
}

impl Role {
    pub(super) fn for_path(path: &str) -> Self {
        match path.rsplit('/').next() {
            Some("package.json") => Self::Package,
            Some("package-lock.json" | "npm-shrinkwrap.json") => Self::Lock,
            _ => Self::Fields,
        }
    }

    fn names(self) -> bool {
        matches!(self, Self::Names | Self::PackageNames | Self::Entries)
    }

    fn child(self, key: &str) -> Self {
        match (self, key) {
            (
                Self::Package | Self::Lock | Self::LockEntry,
                "dependencies"
                | "devDependencies"
                | "optionalDependencies"
                | "peerDependencies"
                | "peerDependenciesMeta",
            ) => {
                if matches!(self, Self::Package) {
                    Self::PackageNames
                } else {
                    Self::Names
                }
            }
            (Self::Lock, "packages") => Self::Entries,
            (Self::Entries | Self::Names, _) => Self::LockEntry,
            _ => Self::Fields,
        }
    }
}

fn count(key: &str, value: &Value) -> bool {
    // A nonnegative numeric counter has a different role from an auth token.
    value.as_u64().is_some()
        && matches!(
            key,
            "input_tokens"
                | "output_tokens"
                | "total_tokens"
                | "token_count"
                | "max_tokens"
                | "max_input_tokens"
                | "max_output_tokens"
                | "token_limit"
        )
}

fn integrity(value: &Value) -> bool {
    let Some(text) = value.as_str().filter(|text| !text.trim().is_empty()) else {
        return false;
    };
    text.split_whitespace().all(|digest| {
        let Some((algorithm, encoded)) = digest.split_once('-') else {
            return false;
        };
        let size = match algorithm {
            "sha1" => 20,
            "sha256" => 32,
            "sha384" => 48,
            "sha512" => 64,
            _ => return false,
        };
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .is_ok_and(|bytes| bytes.len() == size)
    })
}

pub(super) fn validate(value: &Value, role: Role) -> Result<(), DaemonError> {
    match value {
        Value::Object(fields) => {
            if !role.names()
                && fields.contains_key("ciphertext")
                && (fields.contains_key("kdf")
                    || fields.contains_key("nonce")
                    || fields.contains_key("cipher"))
            {
                return Err(refused());
            }
            for (key, value) in fields {
                let lower = key.to_ascii_lowercase().replace('-', "_");
                // Names in package dependency dictionaries and lockfile entry maps
                // are identifiers, not authentication field names. Still inspect
                // names for embedded assignments/URLs and recurse into all values.
                if super::text::credential_format(key)
                    || credential_text(key)
                    || url::Url::parse(key).is_ok_and(|url| credential_url(&url))
                {
                    return Err(refused());
                }
                let metadata = role.names() || count(&lower, value);
                if !metadata
                    && (super::shell::sensitive(&lower)
                        || matches!(lower.as_str(), "vault_file_base64" | "sealed_unlock_key"))
                    && !value.is_null()
                    && value != ""
                {
                    return Err(refused());
                }
                if matches!(role, Role::LockEntry) && key == "integrity" && integrity(value) {
                    continue;
                }
                validate(value, role.child(key))?;
            }
        }
        Value::Array(values) => {
            for value in values {
                validate(value, Role::Fields)?;
            }
        }
        Value::String(text)
            if super::text::credential_format(text)
                || credential_text(text)
                || url::Url::parse(text).is_ok_and(|url| credential_url(&url)) =>
        {
            return Err(refused());
        }
        _ => {}
    }
    Ok(())
}

/// MP-08/MP-11: inspect the JSON string tokens of text that is not complete
/// JSON, with escapes decoded. Each unescaped quote is paired with the next, so
/// text that starts inside a string cannot shift key/value alignment. The
/// lexical policy already sees raw tokens, so only escaped tokens are checked.
pub(super) fn validate_tokens(run: &str) -> Result<(), DaemonError> {
    let mut quotes = Vec::new();
    let mut slashes = 0;
    for (index, ch) in run.char_indices() {
        if ch == '"' && slashes % 2 == 0 {
            quotes.push(index);
        }
        slashes = if ch == '\\' { slashes + 1 } else { 0 };
    }
    let raw = |k: usize| Some(&run[*quotes.get(k)?..=*quotes.get(k + 1)?]);
    let token = |k: usize| serde_json::from_str::<String>(raw(k)?).ok();
    let escaped = |k: usize| raw(k).is_some_and(|raw| raw.contains('\\'));
    let mut keys = Map::new();
    for k in 0..quotes.len().saturating_sub(1) {
        let Some(text) = token(k) else {
            continue;
        };
        let Some(rest) = run[quotes[k + 1] + 1..].trim_start().strip_prefix(':') else {
            if escaped(k) {
                validate(&Value::String(text), Role::Fields)?;
            }
            continue;
        };
        // A truncated, nested or unknown value counts as present.
        let rest = rest.trim_start();
        if !(escaped(k) || (rest.starts_with('"') && escaped(k + 2))) {
            continue;
        }
        let value = if rest.starts_with('"') {
            token(k + 2).map(Value::String)
        } else {
            let end = rest
                .find(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.')))
                .unwrap_or(rest.len());
            serde_json::from_str(&rest[..end]).ok()
        };
        let pair = Map::from_iter([(text.clone(), value.unwrap_or(Value::Bool(true)))]);
        validate(&Value::Object(pair), Role::Fields)?;
        keys.insert(text, Value::Null);
    }
    // Object markers such as an encrypted Vault document span several keys.
    validate(&Value::Object(keys), Role::Fields)
}

/// Paths whose structural policy is JSON: they must parse to be classified.
pub(super) fn named(path: &str) -> bool {
    matches!(path.rsplit_once('.'), Some((_, "json" | "jsonc")))
}

/// JSON with comments and trailing commas, as tsconfig, VS Code and devcontainer
/// files use: returns the JSON with both blanked, and the comment text. An
/// unterminated block comment is `None`.
pub(super) fn strip_comments(text: &str) -> Option<(String, String)> {
    let mut json = String::with_capacity(text.len());
    let mut comments = String::new();
    let mut chars = text.chars().peekable();
    let (mut string, mut escaped, mut comma) = (false, false, None);
    while let Some(ch) = chars.next() {
        if string {
            string = escaped || ch != '"';
            escaped = !escaped && ch == '\\';
        } else if ch == '/' && matches!(chars.peek(), Some('/' | '*')) {
            let block = chars.next() == Some('*');
            let mut previous = ' ';
            loop {
                match chars.next() {
                    Some('\n') if !block => break,
                    Some('/') if block && previous == '*' => break,
                    Some(next) => {
                        comments.push(next);
                        previous = next;
                    }
                    None if block => return None,
                    None => break,
                }
            }
            comments.push('\n');
            json.push('\n');
            continue;
        } else if !ch.is_whitespace() {
            if let Some(index) = comma.take().filter(|_| matches!(ch, '}' | ']')) {
                json.replace_range(index..=index, " ");
            }
            if ch == ',' {
                comma = Some(json.len());
            }
            string = ch == '"';
        }
        json.push(ch);
    }
    Some((json, comments))
}

/// Decodes JSON like `serde_json::Value` but rejects a repeated object key at any
/// depth: `Value` keeps only the last occurrence, hiding an earlier credential.
struct Strict(Value);

struct StrictVisitor;

impl<'de> Visitor<'de> for StrictVisitor {
    type Value = Strict;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("JSON without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Strict, E> {
        Ok(Strict(value.into()))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Strict, E> {
        Ok(Strict(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Strict, E> {
        Ok(Strict(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Strict, E> {
        Ok(Strict(Value::from(value)))
    }

    fn visit_str<E>(self, value: &str) -> Result<Strict, E> {
        Ok(Strict(value.into()))
    }

    fn visit_unit<E>(self) -> Result<Strict, E> {
        Ok(Strict(Value::Null))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Strict, A::Error> {
        let mut values = Vec::new();
        while let Some(Strict(value)) = seq.next_element()? {
            values.push(value);
        }
        Ok(Strict(Value::Array(values)))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Strict, A::Error> {
        let mut fields = Map::new();
        while let Some((key, Strict(value))) = map.next_entry::<String, Strict>()? {
            if fields.insert(key, value).is_some() {
                return Err(serde::de::Error::custom("duplicate JSON object key"));
            }
        }
        Ok(Strict(Value::Object(fields)))
    }
}

impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(StrictVisitor)
    }
}

/// `None` when `text` is not JSON; a duplicate key is refused, not ignored.
pub(super) fn parse(text: &str) -> Result<Option<Value>, DaemonError> {
    match serde_json::from_str::<Strict>(text) {
        Ok(Strict(value)) => Ok(Some(value)),
        Err(_) if serde_json::from_str::<Value>(text).is_ok() => Err(refused()),
        Err(_) => Ok(None),
    }
}
