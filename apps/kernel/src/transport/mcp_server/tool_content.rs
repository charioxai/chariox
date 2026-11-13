use serde_json::Value;

const RESULT_TEXT_BYTES: usize = 64 * 1024;
const RESULT_PREVIEW_BYTES: usize = 6 * 1024;
const RESULT_IMAGE_BYTES: usize = 8 * 1024 * 1024;

pub(super) fn runtime_tool_content(mut payload: Value) -> (Vec<Value>, Value) {
    // MCP structuredContent is an object, while script results may be any JSON value.
    if !payload.is_object() {
        payload = serde_json::json!({ "result": payload });
    }
    let mut image_omitted = false;
    let image = payload.as_object_mut().and_then(|object| {
        let mime_type = object
            .get("mime_type")
            .and_then(Value::as_str)
            .filter(|value| value.starts_with("image/") && value.len() <= 128)?
            .to_string();
        let data_len = object
            .get("image_base64")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())?
            .len();
        if data_len > RESULT_IMAGE_BYTES {
            object.remove("image_base64");
            image_omitted = true;
            return None;
        }
        let data = match object.remove("image_base64") {
            Some(Value::String(data)) => data,
            _ => return None,
        };
        Some(serde_json::json!({
            "type": "image",
            "data": data,
            "mimeType": mime_type,
        }))
    });
    if image_omitted {
        if let Some(object) = payload.as_object_mut() {
            object.insert("chariox_image_omitted".to_string(), Value::Bool(true));
            object.insert(
                "chariox_image_omission_reason".to_string(),
                Value::String("image exceeded the 8 MiB MCP result limit".to_string()),
            );
        }
    }
    // Keep the provider history bounded before any provider can persist this
    // result. A limited writer stops serialization at the cap instead of first
    // allocating a second copy of a huge browser snapshot.
    let mut serialized = BoundedJson::new(RESULT_TEXT_BYTES);
    let pretty = payload.get("snapshot").is_some_and(Value::is_object);
    let serialization = if pretty {
        serde_json::to_writer_pretty(&mut serialized, &payload)
    } else {
        serde_json::to_writer(&mut serialized, &payload)
    };
    let text_payload = if serialization.is_ok() {
        String::from_utf8(serialized.bytes).expect("serde_json emits UTF-8")
    } else {
        let preview_end = utf8_boundary(&serialized.bytes, RESULT_PREVIEW_BYTES);
        let preview = String::from_utf8_lossy(&serialized.bytes[..preview_end]).into_owned();
        let mut metadata = serde_json::Map::new();
        if let Some(object) = payload.as_object() {
            for (key, value) in object {
                if key == "snapshot" || key == "image_base64" || key.len() > 128 {
                    continue;
                }
                if !value.is_null()
                    && !value.is_boolean()
                    && !value.is_number()
                    && !value.is_string()
                {
                    continue;
                }
                if value.as_str().is_some_and(|value| value.len() > 1024) {
                    continue;
                }
                let Ok(encoded) = serde_json::to_vec(value) else {
                    continue;
                };
                if encoded.len() <= 1024 && metadata.len() < 32 {
                    metadata.insert(key.clone(), value.clone());
                    if serde_json::to_vec(&metadata)
                        .is_ok_and(|metadata| metadata.len() <= 4 * 1024)
                    {
                        continue;
                    }
                    metadata.remove(key);
                }
            }
        }
        metadata.insert("chariox_truncated".to_string(), Value::Bool(true));
        metadata.insert(
            "chariox_guidance".to_string(),
            Value::String(if pretty { "Snapshot exceeded the 64 KiB MCP text limit. Use chariox.kernel_browser with command {op:find,tab_id,generation,query,kind:field|button|link|any} to get matching controls and node_refs. Empty query lists controls. Follow next_offset with the returned document_id beside command to get more." } else { "Result exceeded the 64 KiB MCP text limit. Request smaller tool output." }.to_string()),
        );
        metadata.insert("preview".to_string(), Value::String(preview));
        payload = Value::Object(metadata);
        payload.to_string()
    };
    let text = serde_json::json!({
        "type": "text",
        "text": text_payload,
    });
    let content = match image {
        Some(image) => vec![image, text],
        None => vec![text],
    };
    (content, payload)
}

struct BoundedJson {
    bytes: Vec<u8>,
    limit: usize,
}

impl BoundedJson {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(limit),
            limit,
        }
    }
}

impl std::io::Write for BoundedJson {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let remaining = self.limit.saturating_sub(self.bytes.len());
        let accepted = bytes.len().min(remaining);
        self.bytes.extend_from_slice(&bytes[..accepted]);
        if accepted == bytes.len() {
            Ok(accepted)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "MCP result text limit exceeded",
            ))
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn utf8_boundary(bytes: &[u8], limit: usize) -> usize {
    let mut end = bytes.len().min(limit);
    while end > 0 && std::str::from_utf8(&bytes[..end]).is_err() {
        end -= 1;
    }
    end
}
