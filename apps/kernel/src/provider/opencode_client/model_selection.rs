//! MP-08/MP-10: account-local OpenCode model selection.

use super::{OpenCodeClient, OpenCodeProviderCatalog};
use crate::error::DaemonError;

impl OpenCodeClient {
    /// Validate against this run's native account server, never a merged home catalog.
    pub(crate) fn validated_model(
        &self,
        requested: Option<&str>,
    ) -> Result<Option<String>, DaemonError> {
        let Some(requested) = requested
            .map(str::trim)
            .filter(|value| !value.is_empty() && *value != "default")
        else {
            // With no selection, OpenCode owns its native default choice.
            return Ok(None);
        };
        let catalog = self.provider_catalog()?;
        resolve_model(&catalog, requested)
            .map(Some)
            .map_err(|message| self.protocol_error("opencode_prompt_model", message))
    }
}

fn resolve_model(catalog: &OpenCodeProviderCatalog, requested: &str) -> Result<String, String> {
    let available = catalog
        .all
        .iter()
        .filter(|provider| catalog.connected.contains(&provider.id))
        .flat_map(|provider| {
            provider
                .models
                .keys()
                .map(move |model| format!("{}/{model}", provider.id))
        })
        .collect::<Vec<_>>();
    let matches = available
        .iter()
        .filter(|qualified| {
            if requested.contains('/') {
                qualified.as_str() == requested
            } else {
                qualified
                    .split_once('/')
                    .is_some_and(|(_, model)| model == requested)
            }
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [selected] => Ok((*selected).clone()),
        [] => Err(format!(
            "OpenCode model `{requested}` is unavailable in this account's connected provider catalog. Select a catalog model before retrying. Available models: {}{}",
            if available.is_empty() { "none".to_string() } else { available.iter().take(12).cloned().collect::<Vec<_>>().join(", ") },
            if available.len() > 12 { " (list truncated)" } else { "" },
        )),
        _ => Err(format!(
            "OpenCode model `{requested}` is ambiguous. Select a provider-qualified catalog model: {}",
            matches.iter().map(|value| value.as_str()).collect::<Vec<_>>().join(", "),
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    use serde_json::{json, Value};

    use crate::provider::{AgentExecutionMode, OpenCodeClient};

    fn server(expect_prompt: bool) -> (String, thread::JoinHandle<Vec<Value>>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            loop {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 2048];
                loop {
                    let n = stream.read(&mut buffer).unwrap();
                    assert_ne!(n, 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    let text = String::from_utf8_lossy(&bytes);
                    if let Some((headers, body)) = text.split_once("\r\n\r\n") {
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.strip_prefix("Content-Length: ")?.parse::<usize>().ok()
                            })
                            .unwrap_or(0);
                        if body.len() >= length {
                            break;
                        }
                    }
                }
                let text = String::from_utf8(bytes).unwrap();
                let (headers, body) = text.split_once("\r\n\r\n").unwrap();
                let post = headers.starts_with("POST ");
                requests.push(json!({"post": post, "body": serde_json::from_str::<Value>(body).unwrap_or(Value::Null)}));
                let payload = if post {
                    String::new()
                } else {
                    assert!(headers.starts_with("GET /provider "));
                    json!({"all": [{"id": "linked", "name": "Linked", "models": {
                        "available": {"id": "available", "name": "Available"}
                    }}], "connected": ["linked"], "default": {"linked": "available"}})
                    .to_string()
                };
                let status = if post { "204 No Content" } else { "200 OK" };
                write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len()).unwrap();
                if post || !expect_prompt {
                    break;
                }
            }
            requests
        });
        (endpoint, handle)
    }

    fn submit(endpoint: &str, model: &str) -> Result<(), crate::error::DaemonError> {
        OpenCodeClient::new("test", endpoint)
            .unwrap()
            .submit_prompt(
                "session",
                "message",
                "hello",
                &[],
                None,
                Some(model),
                None,
                AgentExecutionMode::Build,
                false,
                false,
            )
    }

    #[test]
    fn unavailable_explicit_model_fails_before_prompt_post() {
        let (endpoint, captured) = server(false);
        let result = submit(&endpoint, "linked/missing");
        let requests = captured.join().unwrap();
        assert!(
            result.is_err(),
            "unavailable selection must fail before a provider turn"
        );
        let error = result.unwrap_err().to_string();
        assert!(error.contains("linked/missing"));
        assert!(error.contains("linked/available"));
        assert!(requests.iter().all(|r| r["post"] == false));
    }

    #[test]
    fn unqualified_model_uses_unique_connected_catalog_provider() {
        let (endpoint, captured) = server(true);
        submit(&endpoint, "available").unwrap();
        let requests = captured.join().unwrap();
        assert_eq!(
            requests.last().unwrap()["body"]["model"],
            json!({
                "providerID": "linked", "modelID": "available"
            })
        );
    }
}

#[cfg(test)]
mod catalog_tests {
    use super::resolve_model;
    use crate::provider::OpenCodeProviderCatalog;
    use serde_json::json;

    fn catalog(connected: &[&str]) -> OpenCodeProviderCatalog {
        serde_json::from_value(json!({"all": [
            {"id": "first", "name": "First", "models": {"same": {"id": "same", "name": "Same"}}},
            {"id": "second", "name": "Second", "models": {"same": {"id": "same", "name": "Same"},
                "nested/model": {"id": "nested/model", "name": "Nested"}}}
        ], "default": {}, "connected": connected}))
        .unwrap()
    }

    #[test]
    fn explicit_provider_qualifier_is_preserved() {
        assert_eq!(
            resolve_model(&catalog(&["first", "second"]), "second/same").unwrap(),
            "second/same"
        );
        assert_eq!(
            resolve_model(&catalog(&["second"]), "second/nested/model").unwrap(),
            "second/nested/model"
        );
    }

    #[test]
    fn unqualified_ambiguity_requires_user_selection() {
        let error = resolve_model(&catalog(&["first", "second"]), "same").unwrap_err();
        assert!(error.contains("ambiguous"));
        assert!(error.contains("first/same"));
        assert!(error.contains("second/same"));
    }

    #[test]
    fn models_of_disconnected_accounts_are_not_available() {
        let catalog = catalog(&["first"]);
        assert!(resolve_model(&catalog, "second/same").is_err());
        assert_eq!(resolve_model(&catalog, "same").unwrap(), "first/same");
    }
}
