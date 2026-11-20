//! Terminal output keeps its record metadata, with raw bytes after the JSON header.
use std::borrow::Cow;

use base64::Engine as _;
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
    let fields = room_terminal_fields(response);
    if !fields.is_empty() {
        let mut payload = Vec::new();
        for &(pointer, encoding) in fields {
            let field = response
                .pointer_mut(pointer)
                .ok_or_else(invalid_room_reply)?;
            let text = field.as_str().ok_or_else(invalid_room_reply)?;
            let data = match encoding {
                RoomByteEncoding::Base64 => base64::engine::general_purpose::STANDARD
                    .decode(text)
                    .map_err(|_| invalid_room_reply())?,
                RoomByteEncoding::Text => text.as_bytes().to_vec(),
            };
            *field = serde_json::json!([payload.len(), data.len()]);
            payload.extend_from_slice(&data);
        }
        return pack(response, &payload);
    }
    if let Some(wire) = encode_mirror_reply(response)? {
        return Ok(wire);
    }
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

/// A Room terminal stream event: its header, with `bytes` naming the raw output after it.
pub(crate) fn encode_room_output(
    mut header: Value,
    data: &[u8],
) -> Result<Vec<u8>, serde_json::Error> {
    header["bytes"] = serde_json::json!([0, data.len()]);
    pack(&header, data)
}

#[derive(Clone, Copy)]
enum RoomByteEncoding {
    Base64,
    Text,
}

fn room_terminal_fields(value: &Value) -> &'static [(&'static str, RoomByteEncoding)] {
    match value
        .pointer("/RoomTerminal/result/result")
        .and_then(Value::as_str)
    {
        Some("output") => &[(
            "/RoomTerminal/result/output/data_base64",
            RoomByteEncoding::Base64,
        )],
        Some("snapshot") => &[
            (
                "/RoomTerminal/result/snapshot/screen_ansi_base64",
                RoomByteEncoding::Base64,
            ),
            (
                "/RoomTerminal/result/snapshot/screen_text",
                RoomByteEncoding::Text,
            ),
            (
                "/RoomTerminal/result/snapshot/scrollback_text",
                RoomByteEncoding::Text,
            ),
        ],
        _ => &[],
    }
}

/// A Browser reply's byte fields in wire order: a mirror packet's deflated body, then each whole resource, then a
/// picture's pixels (`screenshot`). A field holds base64 text before encoding and its `[offset, length]` in the
/// payload after it.
fn mirror_fields(response: &Value) -> Vec<String> {
    let Some(result) = response.pointer("/KernelBrowser/result") else {
        return Vec::new();
    };
    let is_bytes = |field: Option<&Value>| field.is_some_and(|f| f.is_string() || f.is_array());
    let mut fields = Vec::new();
    if is_bytes(result.get("packet_base64")) {
        fields.push("/KernelBrowser/result/packet_base64".to_string());
    }
    let resources = result.get("resources").and_then(Value::as_array);
    for (index, resource) in resources.into_iter().flatten().enumerate() {
        if is_bytes(resource.get("data_base64")) {
            fields.push(format!(
                "/KernelBrowser/result/resources/{index}/data_base64"
            ));
        }
    }
    if is_bytes(result.get("data_base64")) {
        fields.push("/KernelBrowser/result/data_base64".to_string());
    }
    fields
}

/// Moves the base64 of a Browser reply's packet body, resources and picture out of the JSON as raw bytes. A resource slice
/// that is not canonical base64 on its own stays text, and so does an empty field (a retained font): with no bytes to
/// move, a reply of only those must stay plain JSON with its text, not ranges into a payload it never sends. `None`
/// when no field moved.
fn encode_mirror_reply(response: &mut Value) -> Result<Option<Vec<u8>>, serde_json::Error> {
    let engine = base64::engine::general_purpose::STANDARD;
    let mut payload = Vec::new();
    for pointer in mirror_fields(response) {
        let Some(field) = response.pointer_mut(&pointer) else {
            continue;
        };
        let Some(data) = field
            .as_str()
            .and_then(|text| engine.decode(text).ok())
            // An empty field (a retained font has no bytes) stays a string: a `[0, 0]` range would
            // read as bytes the client never expects there.
            .filter(|data| {
                !data.is_empty() && engine.encode(data) == field.as_str().unwrap_or_default()
            })
        else {
            continue;
        };
        *field = serde_json::json!([payload.len(), data.len()]);
        payload.extend_from_slice(&data);
    }
    if payload.is_empty() {
        return Ok(None);
    }
    pack(response, &payload).map(Some)
}

fn invalid_room_reply() -> serde_json::Error {
    serde_json::Error::io(std::io::Error::other("invalid Room terminal reply bytes"))
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
    let fields = room_terminal_fields(&event);
    if !fields.is_empty() {
        if event.as_object().is_none_or(|object| object.len() != 1) {
            return Err(invalid);
        }
        let mut cursor = 0;
        for &(pointer, encoding) in fields {
            let field = event.pointer_mut(pointer).ok_or(invalid)?;
            let range = field
                .as_array()
                .filter(|range| range.len() == 2)
                .ok_or(invalid)?;
            let offset = range[0]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or(invalid)?;
            let length = range[1]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or(invalid)?;
            if offset != cursor || length > payload.len() - cursor {
                return Err(invalid);
            }
            let data = &payload[cursor..cursor + length];
            *field = Value::String(match encoding {
                RoomByteEncoding::Base64 => base64::engine::general_purpose::STANDARD.encode(data),
                RoomByteEncoding::Text => {
                    std::str::from_utf8(data).map_err(|_| invalid)?.to_owned()
                }
            });
            cursor += length;
        }
        if cursor != payload.len() {
            return Err(invalid);
        }
        return Ok(serde_json::to_vec(&event).map_err(|_| invalid)?.into());
    }
    let mirror = mirror_fields(&event);
    if !mirror.is_empty() {
        let mut cursor = 0;
        for pointer in mirror {
            let field = event.pointer_mut(&pointer).ok_or(invalid)?;
            let Some(range) = field.as_array() else {
                continue;
            };
            let (offset, length) = match range.as_slice() {
                [offset, length] => (
                    offset.as_u64().and_then(|n| usize::try_from(n).ok()),
                    length.as_u64().and_then(|n| usize::try_from(n).ok()),
                ),
                _ => return Err(invalid),
            };
            let (offset, length) = offset.zip(length).ok_or(invalid)?;
            if offset != cursor || length > payload.len() - cursor {
                return Err(invalid);
            }
            *field = Value::String(
                base64::engine::general_purpose::STANDARD.encode(&payload[cursor..cursor + length]),
            );
            cursor += length;
        }
        if cursor != payload.len() {
            return Err(invalid);
        }
        return Ok(serde_json::to_vec(&event).map_err(|_| invalid)?.into());
    }
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

#[cfg(test)]
mod room_terminal_tests {
    use super::*;

    #[test]
    fn room_terminal_replies_keep_output_and_snapshots_raw() {
        let data = vec![0xff; 65536];
        for original in [
            serde_json::json!({"RoomTerminal":{"result":{"result":"output","output":{"data_base64":base64::engine::general_purpose::STANDARD.encode(&data),"next_offset":65536,"gap":false}}}}),
            serde_json::json!({"RoomTerminal":{"result":{"result":"snapshot","snapshot":{"screen_ansi_base64":"G1szMW0=","screen_text":"你好","scrollback_text":"old🙂","output_offset":42}}}}),
        ] {
            let mut response = original.clone();
            let wire = encode_reply(&mut response).unwrap();
            assert!(wire.starts_with(b"CXT1"));
            assert_eq!(
                serde_json::from_slice::<Value>(&open(&wire).unwrap()).unwrap(),
                original
            );
            if original["RoomTerminal"]["result"]["result"] == "output" {
                assert!(
                    wire.len() < data.len() + 1024,
                    "native VT output still expands as base64"
                );
            }
        }
    }

    #[test]
    fn malformed_room_terminal_base64_is_an_error() {
        let mut response = serde_json::json!({"RoomTerminal":{"result":{"result":"output","output":{"data_base64":"not base64!"}}}});
        assert!(encode_reply(&mut response).is_err());
    }
}

#[cfg(test)]
mod mirror_reply_tests {
    use super::*;

    fn packet() -> Value {
        let engine = base64::engine::general_purpose::STANDARD;
        serde_json::json!({"KernelBrowser":{"result":{
            "wire":2,"sequence":1,"encoding":"deflate","packet_bytes":9,
            "packet_base64":engine.encode(vec![0xa5u8; 4096]),
            "resources":[
                {"key":"f","resource_id":"r","mime_type":"font/woff2","data_base64":engine.encode(vec![7u8; 300])},
                {"key":"s","resource_id":"s","mime_type":"text/css","data_base64":"AAAA","offset":1,"total":9}]}}})
    }

    #[test]
    fn a_mirror_reply_carries_its_deflated_body_and_resources_as_raw_bytes() {
        let original = packet();
        let mut response = original.clone();
        let wire = encode_reply(&mut response).unwrap();
        assert!(wire.starts_with(b"CXT1"));
        assert!(
            wire.len() < 4096 + 300 + 512,
            "bytes still travel as base64: {}",
            wire.len()
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&open(&wire).unwrap()).unwrap(),
            original
        );
    }

    #[test]
    fn a_screenshot_reply_carries_its_picture_as_raw_bytes() {
        let engine = base64::engine::general_purpose::STANDARD;
        let original = serde_json::json!({"KernelBrowser":{"result":{
            "tab_id":"t","generation":2,"mime_type":"image/png","width":800,"height":600,
            "data_base64":engine.encode(vec![0x89u8; 6000])}}});
        let mut response = original.clone();
        let wire = encode_reply(&mut response).unwrap();
        assert!(
            wire.len() < 6000 + 512,
            "the picture still travels as base64: {}",
            wire.len()
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&open(&wire).unwrap()).unwrap(),
            original
        );
    }

    #[test]
    fn a_slice_that_is_not_canonical_base64_stays_text() {
        let mut response = serde_json::json!({"KernelBrowser":{"result":{
            "wire":2,"resources":[{"data_base64":"QUJD","offset":1,"total":9},{"data_base64":"QUJDRA"}]}}});
        let original = response.clone();
        let wire = encode_reply(&mut response).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&open(&wire).unwrap()).unwrap(),
            original
        );
    }

    #[test]
    fn a_small_reply_that_only_retains_a_font_keeps_the_empty_field_a_string() {
        let mut response = serde_json::json!({"KernelBrowser":{"result":{"wire":2,"resources":[
            {"key":"f","resource_id":"r","mime_type":"font/woff2","data_base64":""}]}}});
        let original = response.clone();
        let wire = encode_reply(&mut response).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&open(&wire).unwrap()).unwrap(),
            original,
            "the client reads a string where a retained font has no bytes"
        );
    }

    #[test]
    fn a_mirror_reply_without_byte_fields_is_plain_json() {
        let mut response =
            serde_json::json!({"KernelBrowser":{"result":{"wire":2,"sequence":2,"ops":[]}}});
        let wire = encode_reply(&mut response).unwrap();
        assert!(!wire.starts_with(b"CXT1"));
    }

    // A packet whose only byte field is a retained font (its digest, no bytes) has nothing to move: it stays plain JSON
    // with the empty text, never a range into a payload that is not sent.
    #[test]
    fn a_mirror_reply_whose_only_resource_is_retained_is_unchanged_json() {
        let mut response = serde_json::json!({"KernelBrowser":{"result":{"wire":2,"sequence":15,"ops":[],
            "resources":[{"key":"r1","resource_id":"r","mime_type":"font/woff2","data_base64":"","retained":true}]}}});
        let original = response.clone();
        let wire = encode_reply(&mut response).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&wire).unwrap(), original);
    }

    #[test]
    fn a_mirror_reply_with_a_range_outside_its_payload_is_refused() {
        let wire = frame_for_mirror(
            serde_json::json!({"KernelBrowser":{"result":{"packet_base64":[0,9]}}}),
            b"abc",
        );
        assert!(open(&wire).is_err());
    }

    fn frame_for_mirror(header: Value, payload: &[u8]) -> Vec<u8> {
        pack(&header, payload).unwrap()
    }
}
