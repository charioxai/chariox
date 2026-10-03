use super::*;

/// Stable identity for an original source event. Keep both the returned ID and
/// original timestamp across retries. A different source key/time is a new event.
pub fn occurrence_id(source_key: &str, occurred_at_ms: u64) -> Result<String> {
    if source_key.is_empty() || source_key.len() > 4096 || occurred_at_ms > MAX_SAFE_TIMESTAMP {
        return Err(OutboxError::Invalid);
    }
    let encoded = serde_json::to_vec(&serde_json::json!([
        "chariox.app-occurrence-id.v1",
        source_key,
        occurred_at_ms
    ]))
    .map_err(|_| OutboxError::Invalid)?;
    Ok(format!(
        "evt1.{occurred_at_ms}.{:x}",
        Sha256::digest(encoded)
    ))
}

pub(super) fn validate(id: &str, occurred_at_ms: u64) -> Result<()> {
    if occurred_at_ms > MAX_SAFE_TIMESTAMP || id.len() > 87 {
        return Err(OutboxError::Invalid);
    }
    let mut parts = id.split('.');
    let prefix = parts.next();
    let timestamp = parts.next().ok_or(OutboxError::Invalid)?;
    let hash = parts.next().ok_or(OutboxError::Invalid)?;
    if prefix != Some("evt1")
        || timestamp != occurred_at_ms.to_string()
        || hash.len() != 64
        || !hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || parts.next().is_some()
    {
        return Err(OutboxError::Invalid);
    }
    Ok(())
}
