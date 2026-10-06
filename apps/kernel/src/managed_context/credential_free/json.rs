//! MP-08/MP-11: JSON keys are classified by schema role; values stay inspected.
use super::{credential_text, credential_url, refused, validate_bytes, MAX_FILE};
use crate::error::DaemonError;
use base64::Engine;
use serde_json::Value;

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
                if credential_text(key)
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
                if key == "path" && !role.names() {
                    validate_bytes(value.as_str().ok_or_else(refused)?, b"")?;
                }
                if matches!(
                    key.as_str(),
                    "content_base64" | "contentBase64" | "source_base64"
                ) && !role.names()
                {
                    let decoded = base64::engine::general_purpose::STANDARD
                        .decode(value.as_str().ok_or_else(refused)?)
                        .map_err(|_| refused())?;
                    if decoded.len() as u64 > MAX_FILE {
                        return Err(refused());
                    }
                    validate_bytes("package-content", &decoded)?;
                } else {
                    validate(value, role.child(key))?;
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                validate(value, Role::Fields)?;
            }
        }
        Value::String(text) => {
            if text.contains("PRIVATE KEY-----")
                || credential_text(text)
                || url::Url::parse(text).is_ok_and(|url| credential_url(&url))
            {
                return Err(refused());
            }
        }
        _ => {}
    }
    Ok(())
}
