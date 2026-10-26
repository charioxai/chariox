use base64::{engine::general_purpose::STANDARD, Engine};
use chariox_app_sdk::{app_types::AppError, error};
use serde_json::Value;

// Leave room for the tool envelope within the broker's 256 KiB JSON limit.
pub const MAX_BYTES: usize = 192 * 1024;

pub fn parse(text: &str) -> Result<Value, AppError> {
    if text.len() > MAX_BYTES {
        return Err(error(
            "SCENE_TOO_LARGE",
            "diagram exceeds 192 KiB; split it into smaller files",
        ));
    }
    let scene: Value = serde_json::from_str(text)
        .map_err(|_| error("INVALID_SCENE", "open an Excalidraw JSON scene"))?;
    if scene["type"] != "excalidraw"
        || scene["version"] != 2
        || !scene["elements"].is_array()
        || !scene["appState"].is_object()
        || !scene["files"].is_object()
    {
        return Err(error(
            "INVALID_SCENE",
            "expected Excalidraw v2 elements, appState and files",
        ));
    }
    Ok(scene)
}

pub fn filename(value: &Value) -> Result<String, AppError> {
    let name = value
        .as_str()
        .ok_or_else(|| error("INVALID_ARGUMENT", "filename is required"))?;
    if name.is_empty()
        || name.len() > 80
        || !name.as_bytes()[0].is_ascii_alphanumeric()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    {
        return Err(error(
            "INVALID_ARGUMENT",
            "use a filename of 1–80 letters, digits, dots, dashes or underscores",
        ));
    }
    Ok(name.into())
}

pub fn file_bytes(input: &Value) -> Result<Vec<u8>, AppError> {
    let bytes = match (input.get("text"), input.get("base64")) {
        (Some(Value::String(text)), None) => text.as_bytes().to_vec(),
        (None, Some(Value::String(text))) => STANDARD
            .decode(text)
            .map_err(|_| error("INVALID_ARGUMENT", "export must be valid base64"))?,
        _ => {
            return Err(error(
                "INVALID_ARGUMENT",
                "provide exactly one of text or base64",
            ))
        }
    };
    if bytes.len() > MAX_BYTES {
        return Err(error(
            "FILE_TOO_LARGE",
            "export exceeds 192 KiB; use a smaller diagram",
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn rejects_host_paths_and_invalid_exports() {
        for name in ["", "../scene", "/tmp/scene", "a/b", "a\\b", ".", ".."] {
            assert!(filename(&json!(name)).is_err(), "{name}");
        }
        assert_eq!(
            filename(&json!("architecture.excalidraw")).unwrap(),
            "architecture.excalidraw"
        );
        assert!(file_bytes(&json!({"base64": "!bad!"})).is_err());
        assert!(file_bytes(&json!({"text": "x", "base64": "eA=="})).is_err());
        assert_eq!(file_bytes(&json!({"base64": "eA=="})).unwrap(), b"x");
    }
    #[test]
    fn accepts_round_trip_scene_and_refuses_wrong_or_oversized_data() {
        let scene =
            json!({"type":"excalidraw", "version":2, "elements":[], "appState":{}, "files":{}});
        assert_eq!(parse(&scene.to_string()).unwrap(), scene);
        assert!(parse("{}").is_err());
        assert!(parse(&" ".repeat(MAX_BYTES + 1)).is_err());
    }
}
