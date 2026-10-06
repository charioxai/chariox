//! MP-11: credential-bearing HTTP output crosses one value-aware secret boundary.
use super::*;

const OPERATION: &str = "http_request_with_credential";
const MAX_BODY_BYTES: u64 = 16 * 1024 * 1024;

pub(super) struct HttpSecretBoundary {
    values: Vec<Zeroizing<String>>,
}

impl HttpSecretBoundary {
    pub(super) fn new(secret: &str) -> Self {
        let mut boundary = Self { values: Vec::new() };
        boundary.add(secret);
        boundary
    }

    pub(super) fn add(&mut self, value: &str) {
        if value.is_empty() {
            return;
        }
        let form = url::form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>();
        let json = serde_json::to_string(value).expect("string JSON serialization");
        for encoded in [
            value.to_string(),
            json[1..json.len() - 1].to_string(),
            form.clone(),
            form.replace('+', "%20"),
            lowercase_percent_escapes(&form),
            lowercase_percent_escapes(&form.replace('+', "%20")),
            base64::engine::general_purpose::STANDARD.encode(value),
            base64::engine::general_purpose::STANDARD_NO_PAD.encode(value),
            base64::engine::general_purpose::URL_SAFE.encode(value),
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value),
            hex_bytes(value.as_bytes()),
            hex_bytes(value.as_bytes()).to_ascii_uppercase(),
        ] {
            if !self
                .values
                .iter()
                .any(|existing| existing.as_str() == encoded)
            {
                self.values.push(Zeroizing::new(encoded));
            }
        }
        self.values
            .sort_by_key(|value| std::cmp::Reverse(value.len()));
    }

    fn scrub(&self, input: &str) -> String {
        // Consume the original once, so a short secret never rewrites our marker.
        let mut output = String::with_capacity(input.len());
        let mut rest = input;
        while !rest.is_empty() {
            if let Some(value) = self
                .values
                .iter()
                .find(|value| rest.starts_with(value.as_str()))
            {
                output.push_str("[redacted]");
                rest = &rest[value.len()..];
            } else {
                let next = rest.chars().next().expect("nonempty string");
                output.push(next);
                rest = &rest[next.len_utf8()..];
            }
        }
        output
    }

    fn scrub_json(&self, value: &mut serde_json::Value) {
        match value {
            serde_json::Value::String(text) => *text = self.scrub(text),
            serde_json::Value::Array(items) => {
                items.iter_mut().for_each(|item| self.scrub_json(item))
            }
            serde_json::Value::Object(fields) => {
                *fields = std::mem::take(fields)
                    .into_iter()
                    .map(|(key, mut value)| {
                        self.scrub_json(&mut value);
                        (self.scrub(&key), value)
                    })
                    .collect();
            }
            _ => {
                let text = value.to_string();
                let scrubbed = self.scrub(&text);
                if scrubbed != text {
                    *value = serde_json::Value::String(scrubbed);
                }
            }
        }
    }
}

fn lowercase_percent_escapes(value: &str) -> String {
    let mut bytes = value.as_bytes().to_vec();
    for offset in 0..bytes.len().saturating_sub(2) {
        if bytes[offset] == b'%' {
            bytes[offset + 1].make_ascii_lowercase();
            bytes[offset + 2].make_ascii_lowercase();
        }
    }
    String::from_utf8(bytes).expect("ASCII escape changes preserve UTF-8")
}

pub(super) fn decode_http_response(
    response: ureq::Response,
    maximum: u64,
    boundary: &HttpSecretBoundary,
) -> Result<CredentialHttpResponse, DaemonError> {
    let maximum = maximum.min(MAX_BODY_BYTES);
    let status = response.status();
    let mut body = Zeroizing::new(String::new());
    response
        .into_reader()
        .take(maximum + 1)
        .read_to_string(&mut body)
        .map_err(|_| secret_error(OPERATION, "failed to read HTTP response body".to_string()))?;
    if body.len() as u64 > maximum {
        return Err(secret_error(
            OPERATION,
            format!("response exceeded max_response_bytes ({maximum})"),
        ));
    }
    let mut body_json = serde_json::from_str::<serde_json::Value>(&body).ok();
    if let Some(json) = &mut body_json {
        boundary.scrub_json(json);
    }
    Ok(CredentialHttpResponse {
        status,
        body_text: body_json.is_none().then(|| boundary.scrub(&body)),
        body_json,
    })
}

pub(super) fn http_error(
    error: ureq::Error,
    maximum: u64,
    boundary: &HttpSecretBoundary,
) -> DaemonError {
    match error {
        ureq::Error::Status(code, response) => {
            match decode_http_response(response, maximum, boundary) {
                Ok(response) => {
                    let body = response
                        .body_text
                        .unwrap_or_else(|| response.body_json.unwrap_or_default().to_string());
                    secret_error(OPERATION, format!("HTTP {code}: {body}"))
                }
                Err(error) => error,
            }
        }
        // Provider diagnostics can contain URLs, credentials and server-controlled text.
        ureq::Error::Transport(_) => secret_error(OPERATION, "HTTP transport failed".to_string()),
    }
}
