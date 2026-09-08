use super::*;

pub const MAX_PROMPT_BYTES: usize = 64 * 1024;
pub const MAX_INVOCATION_BYTES: usize = 256 * 1024;
pub const MAX_ARTIFACTS: usize = 32;

/// App-authored canonical work, independent of the signed event payload schema.
/// Artifact references are untrusted metadata, never file/network/asset grants.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Invocation {
    pub prompt: String,
    pub artifacts: Vec<Artifact>,
}

/// This maps to existing workflow EventArtifact metadata only. A later content
/// export must resolve an installation-scoped kernel grant; it must not promote
/// `reference` to PromptAttachment, open a host path, or fetch a URL implicitly.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Artifact {
    pub name: String,
    pub media_type: String,
    pub reference: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_size"
    )]
    pub size_bytes: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_digest"
    )]
    pub digest: Option<String>,
}
fn optional_size<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<u64>, D::Error> {
    <u64 as serde::Deserialize>::deserialize(d).map(Some)
}
fn optional_digest<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<String>, D::Error> {
    <String as serde::Deserialize>::deserialize(d).map(Some)
}
impl Invocation {
    pub(super) fn encode(&self) -> Result<String> {
        if self.prompt.len() > MAX_PROMPT_BYTES || self.artifacts.len() > MAX_ARTIFACTS {
            return Err(OutboxError::Limit);
        }
        if self.prompt.trim().is_empty() {
            return Err(OutboxError::Invalid);
        }
        for artifact in &self.artifacts {
            for value in [&artifact.name, &artifact.media_type, &artifact.reference] {
                if value.trim().is_empty() || value.len() > 2048 {
                    return Err(OutboxError::Invalid);
                }
            }
            if artifact
                .size_bytes
                .is_some_and(|size| size > MAX_SAFE_TIMESTAMP)
                || artifact.digest.as_deref().is_some_and(|value| {
                    value.strip_prefix("sha256:").is_none_or(|hex| {
                        hex.len() != 64
                            || !hex
                                .bytes()
                                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                    })
                })
            {
                return Err(OutboxError::Invalid);
            }
        }
        // Scalars and count are bounded before serialization, including callers
        // constructing this data directly rather than through the bounded decoder.
        let encoded =
            serde_json_canonicalizer::to_string(self).map_err(|_| OutboxError::Invalid)?;
        if encoded.len() > MAX_INVOCATION_BYTES {
            return Err(OutboxError::Limit);
        }
        Ok(encoded)
    }
}
