//! The plaintext of a gateway request and its answer, with the bytes as bytes. A file chunk is a
//! `data_base64` string inside the JSON; sealed and sent as it is, it is one third larger on the
//! wire than the file bytes. Between two kernels that advertise `framed_plaintext_v1` the
//! plaintext starts with `CXF1`, a big-endian u32 header length, the JSON with every
//! `data_base64` string replaced by `"$bytes":[offset,length]`, then the raw bytes those
//! entries point to. Segments are contiguous and cover the payload exactly. JSON never starts
//! with this magic, so a kernel reads either form; it answers in the form it was asked in.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::Value;

pub(crate) const CAPABILITY: &str = "framed_plaintext_v1";
const MAGIC: &[u8; 4] = b"CXF1";
const DATA_KEY: &str = "data_base64";
const BYTES_KEY: &str = "$bytes";
/// Request, variant, field: a chunk sits at most this deep.
const MAX_DEPTH: usize = 4;

/// The plaintext in framed form. Whatever has no `data_base64` to lift is framed with its JSON
/// untouched, so the receiver learns that this sender reads framed answers.
pub(crate) fn frame(json: &[u8]) -> Vec<u8> {
    let (header, payload) = lift(json).unwrap_or_else(|| (json.to_vec(), Vec::new()));
    let mut bytes = Vec::with_capacity(8 + header.len() + payload.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&(header.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&header);
    bytes.extend_from_slice(&payload);
    bytes
}

fn lift(json: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    if !contains(json, DATA_KEY.as_bytes()) {
        return None;
    }
    let mut value: Value = serde_json::from_slice(json).ok()?;
    let mut payload = Vec::new();
    take_bytes(&mut value, 0, &mut payload)?;
    if payload.is_empty() {
        return None;
    }
    Some((serde_json::to_vec(&value).ok()?, payload))
}

fn take_bytes(value: &mut Value, depth: usize, payload: &mut Vec<u8>) -> Option<()> {
    if depth > MAX_DEPTH {
        return Some(());
    }
    match value {
        Value::Object(object) => {
            if let Some(Value::String(encoded)) = object.get(DATA_KEY) {
                let bytes = STANDARD.decode(encoded).ok()?;
                object.remove(DATA_KEY);
                object.insert(
                    BYTES_KEY.into(),
                    serde_json::json!([payload.len(), bytes.len()]),
                );
                payload.extend_from_slice(&bytes);
            }
            for child in object.values_mut() {
                take_bytes(child, depth + 1, payload)?;
            }
        }
        Value::Array(items) => {
            for child in items {
                take_bytes(child, depth + 1, payload)?;
            }
        }
        _ => {}
    }
    Some(())
}

/// The JSON of a plaintext and whether it was framed. A plaintext that is not framed is JSON
/// already and is returned as it is.
pub(crate) fn open(plaintext: &[u8]) -> Result<(std::borrow::Cow<'_, [u8]>, bool), &'static str> {
    if !plaintext.starts_with(MAGIC) {
        return Ok((plaintext.into(), false));
    }
    let size = plaintext
        .get(4..8)
        .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
        .map(|bytes| u32::from_be_bytes(bytes) as usize)
        .ok_or("framed plaintext is cut short")?;
    let header = plaintext
        .get(8..8usize.saturating_add(size))
        .ok_or("framed plaintext header is cut short")?;
    let payload = &plaintext[8 + size..];
    if !contains(header, BYTES_KEY.as_bytes()) {
        if !payload.is_empty() {
            return Err("framed plaintext carries bytes nothing points to");
        }
        return Ok((header.into(), true));
    }
    let mut value: Value = serde_json::from_slice(header).map_err(|_| "framed plaintext header")?;
    let mut next = 0;
    put_bytes(&mut value, 0, payload, &mut next)?;
    if next != payload.len() {
        return Err("framed plaintext carries bytes nothing points to");
    }
    let json = serde_json::to_vec(&value).map_err(|_| "framed plaintext header")?;
    Ok((json.into(), true))
}

fn put_bytes(
    value: &mut Value,
    depth: usize,
    payload: &[u8],
    next: &mut usize,
) -> Result<(), &'static str> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    match value {
        Value::Object(object) => {
            if let Some(segment) = object.remove(BYTES_KEY) {
                let (offset, length) = segment
                    .as_array()
                    .and_then(|pair| Some((pair.first()?.as_u64()?, pair.get(1)?.as_u64()?)))
                    .ok_or("framed plaintext segment shape")?;
                let (offset, length) = (
                    usize::try_from(offset).map_err(|_| "framed plaintext segment bounds")?,
                    usize::try_from(length).map_err(|_| "framed plaintext segment bounds")?,
                );
                if offset != *next
                    || offset
                        .checked_add(length)
                        .is_none_or(|end| end > payload.len())
                {
                    return Err("framed plaintext segment bounds");
                }
                object.insert(
                    DATA_KEY.into(),
                    Value::String(STANDARD.encode(&payload[offset..offset + length])),
                );
                *next = offset + length;
            }
            for child in object.values_mut() {
                put_bytes(child, depth + 1, payload, next)?;
            }
        }
        Value::Array(items) => {
            for child in items {
                put_bytes(child, depth + 1, payload, next)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk_request(bytes: &[u8]) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "command_id": "put-1",
            "request": {"WriteKernelFile": {
                "path": "/tmp/f", "offset": 0, "total_size": bytes.len(),
                "data_base64": STANDARD.encode(bytes), "overwrite": true,
            }},
        }))
        .unwrap()
    }

    #[test]
    fn a_chunk_travels_as_its_bytes_and_comes_back_as_the_same_json() {
        let bytes: Vec<u8> = (0..65536u32).map(|n| (n * 31 % 251) as u8).collect();
        let json = chunk_request(&bytes);
        let framed = frame(&json);
        assert!(framed.starts_with(MAGIC));
        assert!(
            framed.len() < bytes.len() + 512,
            "{} bytes for {} file bytes",
            framed.len(),
            bytes.len()
        );
        let (opened, was_framed) = open(&framed).unwrap();
        assert!(was_framed);
        let before: Value = serde_json::from_slice(&json).unwrap();
        let after: Value = serde_json::from_slice(&opened).unwrap();
        assert!(before == after, "the JSON is restored with its data");
    }

    #[test]
    fn an_answer_nested_under_response_is_lifted_too() {
        let json = serde_json::to_vec(&serde_json::json!({"response": {"KernelFileChunk": {
            "path": "/tmp/f", "offset": 0, "size": 3, "eof": true,
            "data_base64": STANDARD.encode(b"abc"),
        }}}))
        .unwrap();
        let framed = frame(&json);
        assert!(framed.ends_with(b"abc"));
        let (opened, _) = open(&framed).unwrap();
        assert!(
            serde_json::from_slice::<Value>(&opened).unwrap()
                == serde_json::from_slice::<Value>(&json).unwrap()
        );
    }

    #[test]
    fn json_without_data_is_framed_untouched_and_json_is_read_as_it_is() {
        let json = br#"{"request":{"ListSessions":null}}"#;
        let framed = frame(json);
        assert_eq!(&framed[8..], json);
        let (opened, was_framed) = open(&framed).unwrap();
        assert!(was_framed && opened.as_ref() == json);
        let (plain, was_framed) = open(json).unwrap();
        assert!(!was_framed && plain.as_ref() == json);
    }

    #[test]
    fn a_header_that_does_not_account_for_the_payload_is_refused() {
        let json = chunk_request(b"0123456789");
        let framed = frame(&json);
        let mut longer = framed.clone();
        longer.push(7);
        assert!(open(&longer).is_err(), "bytes nothing points to");
        assert!(
            open(&framed[..framed.len() - 1]).is_err(),
            "bytes cut short"
        );
        assert!(open(&framed[..6]).is_err());
        let mut lying = framed;
        lying[4..8].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(open(&lying).is_err());
    }
}
