//! Terminal output keeps its record metadata, with raw bytes after the JSON header.
use std::borrow::Cow;

use serde_json::Value;

use super::kernel_protocol::KernelEvent;

const MAGIC: &[u8; 4] = b"CXT1";
const MAX_HEADER: usize = 1024 * 1024;

pub(crate) fn encode(event: &mut KernelEvent) -> Result<Vec<u8>, serde_json::Error> {
    let KernelEvent::TerminalOutput { records } = event else {
        return serde_json::to_vec(event);
    };
    let mut payload = Vec::new();
    let mut headers = Vec::with_capacity(records.len());
    for record in records {
        let bytes = std::mem::take(&mut record.bytes);
        let metadata = serde_json::to_value(&*record);
        record.bytes = bytes;
        let mut header = metadata?;
        header["bytes"] = serde_json::json!([payload.len(), record.bytes.len()]);
        headers.push(header);
        payload.extend_from_slice(&record.bytes);
    }
    pack(
        &serde_json::json!({"event": "terminal_output", "records": headers}),
        &payload,
    )
}

// Polled native renderers receive the same raw byte transport as push subscribers.
pub(crate) fn encode_reply(response: &mut Value) -> Result<Vec<u8>, serde_json::Error> {
    let Some(records) = response
        .get_mut("TerminalOutput")
        .and_then(|output| output.get_mut("records"))
        .and_then(Value::as_array_mut)
    else {
        return serde_json::to_vec(response);
    };
    let mut payload = Vec::new();
    for record in records {
        let bytes = record["bytes"].take();
        let bytes = bytes.as_array().ok_or_else(|| {
            serde_json::Error::io(std::io::Error::other("invalid terminal reply bytes"))
        })?;
        let offset = payload.len();
        for byte in bytes {
            let byte = byte
                .as_u64()
                .and_then(|byte| u8::try_from(byte).ok())
                .ok_or_else(|| {
                    serde_json::Error::io(std::io::Error::other("invalid terminal reply byte"))
                })?;
            payload.push(byte);
        }
        record["bytes"] = serde_json::json!([offset, bytes.len()]);
    }
    pack(response, &payload)
}

fn pack(header: &Value, payload: &[u8]) -> Result<Vec<u8>, serde_json::Error> {
    let header = serde_json::to_vec(header)?;
    if header.len() > MAX_HEADER {
        return Err(serde_json::Error::io(std::io::Error::other(
            "terminal output header exceeds 1 MiB",
        )));
    }
    let mut bytes = Vec::with_capacity(8 + header.len() + payload.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&(header.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&header);
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

/// Gateways restore the ordinary event shape before forwarding to their local client.
pub(crate) fn open(bytes: &[u8]) -> Result<Cow<'_, [u8]>, &'static str> {
    if !bytes.starts_with(MAGIC) {
        return Ok(bytes.into());
    }
    let invalid = "invalid terminal output event";
    let size = bytes
        .get(4..8)
        .and_then(|value| <[u8; 4]>::try_from(value).ok())
        .map(u32::from_be_bytes)
        .ok_or(invalid)? as usize;
    if !(2..=MAX_HEADER).contains(&size) {
        return Err(invalid);
    }
    let header = bytes.get(8..8 + size).ok_or(invalid)?;
    let payload = &bytes[8 + size..];
    let mut event: Value = serde_json::from_slice(header).map_err(|_| invalid)?;
    let output = if event["event"] == "terminal_output" {
        &mut event
    } else if event.as_object().is_some_and(|object| object.len() == 1) {
        event.get_mut("TerminalOutput").ok_or(invalid)?
    } else {
        return Err(invalid);
    };
    let records = output
        .get_mut("records")
        .and_then(Value::as_array_mut)
        .ok_or(invalid)?;
    let mut cursor = 0;
    for record in records {
        let range = record
            .get("bytes")
            .and_then(Value::as_array)
            .filter(|range| range.len() == 2)
            .ok_or(invalid)?;
        let offset = range[0]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or(invalid)?;
        let length = range[1]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or(invalid)?;
        if offset != cursor || length > payload.len() - cursor {
            return Err(invalid);
        }
        record["bytes"] = serde_json::json!(&payload[cursor..cursor + length]);
        cursor += length;
    }
    if cursor != payload.len() {
        return Err(invalid);
    }
    Ok(serde_json::to_vec(&event).map_err(|_| invalid)?.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_preserves_the_event_and_all_record_metadata() {
        let mut event: KernelEvent = serde_json::from_value(serde_json::json!({
            "event":"terminal_output", "records":[{
                "record_id":42, "sequence":7, "sequence_end":8, "sequence_byte_lengths":[2,1],
                "timestamp_ms":12, "session_id":"room", "provider_run_id":"run",
                "kind":"provider_output", "agent_id":"agent", "merge_key":"turn",
                "recipient_attachment_ids":["a"], "pending_recipient_attachment_ids":["a"],
                "bytes":[0,255,27]
            }]
        }))
        .unwrap();
        let original = serde_json::to_value(&event).unwrap();
        let wire = encode(&mut event).unwrap();
        assert_eq!(serde_json::to_value(&event).unwrap(), original);
        assert_eq!(
            serde_json::from_slice::<Value>(&open(&wire).unwrap()).unwrap(),
            original
        );
    }

    fn frame(header: Value, payload: &[u8]) -> Vec<u8> {
        let json = serde_json::to_vec(&header).unwrap();
        [
            MAGIC.as_slice(),
            &(json.len() as u32).to_be_bytes(),
            &json,
            payload,
        ]
        .concat()
    }

    #[test]
    fn malformed_terminal_segments_are_refused() {
        for header in [
            Value::Null,
            serde_json::json!({"event":"other", "records":[]}),
            serde_json::json!({"event":"terminal_output", "records":[null]}),
            serde_json::json!({"event":"terminal_output", "records":[{"bytes":[1,2]}]}),
            serde_json::json!({"event":"terminal_output", "records":[{"bytes":[0,4]}]}),
            serde_json::json!({"event":"terminal_output", "records":[{"bytes":[0,2]}]}),
            serde_json::json!({"event":"terminal_output", "records":[{"bytes":[0,3,0]}]}),
        ] {
            assert!(open(&frame(header, b"abc")).is_err());
        }
        assert!(open(b"CXT1").is_err());
        assert!(open(b"CXT1\0\0\xff\xff").is_err());
    }

    #[test]
    fn polled_reply_bytes_roundtrip_without_json_array_expansion() {
        let source = vec![255u8; 64 * 1024];
        let mut response = serde_json::json!({"TerminalOutput":{"records":[
            {"record_id":42,"bytes":source}, {"record_id":43,"bytes":[]}]}});
        let original = response.clone();
        let wire = encode_reply(&mut response).unwrap();
        assert!(wire.len() < source.len() + 256);
        assert_eq!(
            serde_json::from_slice::<Value>(&open(&wire).unwrap()).unwrap(),
            original
        );
    }

    #[test]
    fn byte_ranges_restore_empty_and_non_utf8_records_exactly() {
        let wire = frame(
            serde_json::json!({"event":"terminal_output", "records":[
            {"bytes":[0,3], "record_id":42}, {"bytes":[3,0], "record_id":43}]}),
            &[0, 255, 27],
        );
        let opened = open(&wire).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&opened).unwrap(),
            serde_json::json!({
            "event":"terminal_output", "records":[{"bytes":[0,255,27],"record_id":42}, {"bytes":[],"record_id":43}]})
        );
    }
}
