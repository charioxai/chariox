//! The App can select only a declared destination or its own opaque handle.
//! Authority fields are never inferred from optional connection/effect IDs.
use super::{HttpError, Result, CHUNK_BYTES};
use base64::{engine::general_purpose::STANDARD, Engine};
use bytes::Bytes;
use serde_json::{Map, Value};

pub(super) struct Open {
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub has_body: bool,
    pub connection: Option<String>,
    pub operation: Option<String>,
}
pub(super) enum Command {
    Open(Open),
    Write { id: String, bytes: Bytes, end: bool },
    Headers(String),
    Read(String),
    Cancel(String),
}
pub(super) fn command(method: &str, params: Value) -> Result<Command> {
    match method {
        "http.open" => {
            let mut fields = fields(
                params,
                &[
                    "url",
                    "method",
                    "headers",
                    "hasBody",
                    "connectionId",
                    "operationId",
                ],
            )?;
            let url = string(take(&mut fields, "url")?, 8192)?;
            let parsed = url::Url::parse(&url).map_err(|_| HttpError::Invalid)?;
            if parsed.scheme() != "https" {
                return Err(HttpError::Destination);
            }
            let method = string(take(&mut fields, "method")?, 16)?;
            let has_body = boolean(take(&mut fields, "hasBody")?)?;
            if has_body && matches!(method.as_str(), "GET" | "HEAD") {
                return Err(HttpError::Invalid);
            }
            let Value::Array(values) = take(&mut fields, "headers")? else {
                return Err(HttpError::Invalid);
            };
            if values.len() > 64 {
                return Err(HttpError::Limit);
            }
            let mut headers = Vec::with_capacity(values.len());
            let mut total = 0usize;
            for value in values {
                let Value::Array(mut pair) = value else {
                    return Err(HttpError::Invalid);
                };
                if pair.len() != 2 {
                    return Err(HttpError::Invalid);
                }
                let value = string(pair.pop().unwrap(), 16 * 1024)?;
                let name = string(pair.pop().unwrap(), 16 * 1024)?;
                total = total
                    .checked_add(name.len())
                    .and_then(|n| n.checked_add(value.len()))
                    .ok_or(HttpError::Limit)?;
                if total > 16 * 1024 {
                    return Err(HttpError::Limit);
                }
                headers.push((name, value));
            }
            Ok(Command::Open(Open {
                url,
                method,
                headers,
                has_body,
                connection: optional(&mut fields, "connectionId")?,
                operation: optional(&mut fields, "operationId")?,
            }))
        }
        "http.write" => {
            let mut fields = fields(params, &["streamId", "bodyBase64", "end"])?;
            let id = identity(take(&mut fields, "streamId")?)?;
            let encoded = string(
                take(&mut fields, "bodyBase64")?,
                ((CHUNK_BYTES + 2) / 3) * 4,
            )?;
            let bytes = STANDARD.decode(encoded).map_err(|_| HttpError::Invalid)?;
            let end = boolean(take(&mut fields, "end")?)?;
            if bytes.len() > CHUNK_BYTES || (bytes.is_empty() && !end) {
                return Err(HttpError::Invalid);
            }
            Ok(Command::Write {
                id,
                bytes: bytes.into(),
                end,
            })
        }
        "http.headers" | "http.read" | "http.cancel" => {
            let mut fields = fields(params, &["streamId"])?;
            let id = identity(take(&mut fields, "streamId")?)?;
            Ok(match method {
                "http.headers" => Command::Headers(id),
                "http.read" => Command::Read(id),
                _ => Command::Cancel(id),
            })
        }
        _ => Err(HttpError::Invalid),
    }
}
fn fields(value: Value, allowed: &[&str]) -> Result<Map<String, Value>> {
    let Value::Object(fields) = value else {
        return Err(HttpError::Invalid);
    };
    if fields.len() > allowed.len() || fields.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(HttpError::Invalid);
    }
    Ok(fields)
}
fn take(fields: &mut Map<String, Value>, key: &str) -> Result<Value> {
    fields.remove(key).ok_or(HttpError::Invalid)
}
fn string(value: Value, limit: usize) -> Result<String> {
    let Value::String(value) = value else {
        return Err(HttpError::Invalid);
    };
    if value.len() > limit {
        return Err(HttpError::Limit);
    }
    Ok(value)
}
fn boolean(value: Value) -> Result<bool> {
    value.as_bool().ok_or(HttpError::Invalid)
}
fn optional(fields: &mut Map<String, Value>, name: &str) -> Result<Option<String>> {
    fields
        .remove(name)
        .map(|value| {
            let value = string(value, 128)?;
            if value.is_empty() || value.chars().any(|c| c.is_control() || c.is_whitespace()) {
                return Err(HttpError::Invalid);
            }
            Ok(value)
        })
        .transpose()
}
fn identity(value: Value) -> Result<String> {
    let value = string(value, 36)?;
    if uuid::Uuid::parse_str(&value).is_err() || value.len() != 36 {
        return Err(HttpError::Invalid);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const ID: &str = "07f5a249-a00b-4196-acdf-938265f88e0a";
    #[test]
    fn shared_sdk_http_snapshot_decodes_requests_and_encodes_actual_responses() {
        use super::super::{encode, streams::ReadResult, transport::ResponseHead};
        let fixture: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../packages/app-sdk/test/http-contract.json"
        )))
        .unwrap();
        assert_eq!(
            fixture["minimumKernelProtocol"],
            crate::local::LOCAL_DAEMON_PROTOCOL_VERSION
        );
        assert_eq!(
            fixture["sdkVersion"],
            chariox_app_package::SUPPORTED_SDK_VERSION
        );
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 8);
        let mut destination = None;
        for (index, case) in cases.iter().enumerate() {
            let decoded =
                command(case["method"].as_str().unwrap(), case["params"].clone()).unwrap();
            let actual = match (index, decoded) {
                (0, Command::Open(open)) => {
                    assert_eq!(open.method, "POST");
                    assert!(open.has_body);
                    assert!(open.connection.is_none() && open.operation.is_none());
                    destination = Some(ResponseHead {
                        status: 200,
                        headers: open.headers,
                        url: open.url,
                    });
                    encode::open(ID)
                }
                (1, Command::Write { id, bytes, end }) => {
                    assert_eq!(id, ID);
                    assert_eq!(bytes.as_ref(), &[0, 255, b'o', b'k']);
                    assert!(end);
                    encode::written(bytes.len())
                }
                (2, Command::Headers(id)) => {
                    assert_eq!(id, ID);
                    encode::headers(None)
                }
                (3, Command::Headers(id)) => {
                    assert_eq!(id, ID);
                    encode::headers(destination.take())
                }
                (4, Command::Read(id)) => {
                    assert_eq!(id, ID);
                    encode::read(ReadResult::Pending)
                }
                (5, Command::Read(id)) => {
                    assert_eq!(id, ID);
                    encode::read(ReadResult::Chunk(Bytes::from_static(&[0, 255, b'o', b'k'])))
                }
                (6, Command::Read(id)) => {
                    assert_eq!(id, ID);
                    encode::read(ReadResult::End)
                }
                (7, Command::Cancel(id)) => {
                    assert_eq!(id, ID);
                    Value::Null
                }
                _ => panic!("HTTP snapshot method/order changed without updating the contract"),
            };
            assert_eq!(actual, case["result"], "HTTP snapshot case {index}");
        }
    }
    #[test]
    fn strict_fields_opaque_identity_and_exact_chunk_bounds() {
        let open = json!({"url":"https://api.example.com/", "method":"POST", "headers":[], "hasBody":true});
        for patch in [
            json!({"owner":"alice"}),
            json!({"generation":"1"}),
            json!({"connectionId":null}),
            json!({"hasBody":null}),
            json!({"url":"http://api.example.com/"}),
        ] {
            let mut value = open.clone();
            value
                .as_object_mut()
                .unwrap()
                .extend(patch.as_object().unwrap().clone());
            assert!(command("http.open", value).is_err());
        }
        assert!(command("http.open", open).is_ok());
        for value in [
            json!({"streamId":ID,"offset":0}),
            json!({"streamId":"foreign-path"}),
            json!({"streamId":null}),
        ] {
            assert!(command("http.read", value).is_err());
        }
        let exact = STANDARD.encode(vec![0; CHUNK_BYTES]);
        assert!(
            matches!(command("http.write",json!({"streamId":ID,"bodyBase64":exact,"end":false})),Ok(Command::Write{bytes,..}) if bytes.len()==CHUNK_BYTES)
        );
        let large = STANDARD.encode(vec![0; CHUNK_BYTES + 1]);
        assert!(command(
            "http.write",
            json!({"streamId":ID,"bodyBase64":large,"end":true})
        )
        .is_err());
        assert!(command(
            "http.write",
            json!({"streamId":ID,"bodyBase64":"","end":true})
        )
        .is_ok());
        assert!(command(
            "http.write",
            json!({"streamId":ID,"bodyBase64":"","end":false})
        )
        .is_err());
    }
}
