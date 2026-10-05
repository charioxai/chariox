//! Cloud API HTTP transport, URL encoding, and error classification.

use crate::error::DaemonError;

#[path = "bounded_artifact.rs"]
mod bounded_artifact;

const CLOUD_API_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

pub(crate) fn normalize_cloud_api_url(api_url: &str) -> Result<String, DaemonError> {
    let normalized = api_url.trim().trim_end_matches('/').to_string();
    if normalized.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "normalize cloud relay api url",
            message: "api_url must not be empty".to_string(),
        });
    }
    Ok(normalized)
}

pub(crate) async fn post_cloud_json<T>(
    api_url: String,
    path: &'static str,
    body: serde_json::Value,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned + Send + 'static,
{
    tokio::task::spawn_blocking(move || post_cloud_json_blocking(api_url, path, body))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "post cloud relay json",
            message: error.to_string(),
        })?
}

pub(crate) async fn post_cloud_acknowledged(
    api_url: String,
    path: &'static str,
    body: serde_json::Value,
) -> Result<(), DaemonError> {
    tokio::task::spawn_blocking(move || {
        post_cloud_acknowledged_blocking_with_timeout(
            api_url,
            path,
            body,
            CLOUD_API_REQUEST_TIMEOUT,
        )
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "acknowledge cloud relay request",
        message: error.to_string(),
    })?
}

pub(crate) async fn post_cloud_json_dynamic<T>(
    api_url: String,
    path: String,
    body: serde_json::Value,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned + Send + 'static,
{
    tokio::task::spawn_blocking(move || post_cloud_json_blocking(api_url, &path, body))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "post cloud relay json",
            message: error.to_string(),
        })?
}

pub(crate) async fn get_cloud_json<T>(api_url: String, path: String) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned + Send + 'static,
{
    tokio::task::spawn_blocking(move || get_cloud_json_blocking(api_url, &path))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "get cloud relay json",
            message: error.to_string(),
        })?
}

pub(crate) async fn get_cloud_json_authenticated<T>(
    api_url: String,
    path: String,
    bearer_token: String,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        get_cloud_json_authenticated_blocking(api_url, &path, &bearer_token)
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "get authenticated cloud json",
        message: error.to_string(),
    })?
}

pub(crate) async fn get_cloud_kernel_directory(
    profile: &crate::config::PersistedCloudRelayProfile,
) -> Result<serde_json::Value, DaemonError> {
    let profile = profile.clone();
    tokio::task::spawn_blocking(move || {
        let url = format!(
            "{}/relay/targets?accountId={}&realmId={}",
            profile.api_url,
            cloud_url_component(&profile.account_id),
            cloud_url_component(&profile.realm_id)
        );
        let credential =
            profile
                .kernel_credential
                .as_deref()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "read My kernels",
                    message: "independent kernel enrollment is required".into(),
                })?;
        let response = ureq::AgentBuilder::new()
            .timeout(CLOUD_API_REQUEST_TIMEOUT)
            .build()
            .get(&url)
            .set("x-chariox-kernel-credential", credential)
            .call()
            .map_err(cloud_transport_error)?;
        decode_cloud_response(response)
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "read My kernels",
        message: error.to_string(),
    })?
}

pub(crate) async fn delete_cloud_json_authenticated<T>(
    api_url: String,
    path: String,
    bearer_token: String,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let agent = ureq::AgentBuilder::new()
            .timeout(CLOUD_API_REQUEST_TIMEOUT)
            .build();
        let response = agent
            .delete(&format!("{api_url}{path}"))
            .set("authorization", &format!("Bearer {bearer_token}"))
            .call()
            .map_err(cloud_transport_error)?;
        decode_cloud_response(response)
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "delete authenticated cloud json",
        message: error.to_string(),
    })?
}

pub(crate) async fn post_cloud_json_authenticated<T>(
    api_url: String,
    path: String,
    bearer_token: String,
    body: serde_json::Value,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        post_cloud_json_authenticated_blocking(api_url, &path, &bearer_token, body)
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "post authenticated cloud json",
        message: error.to_string(),
    })?
}

/// Streams a Cloud response body into `destination` (via a `.partial` file),
/// refusing bodies larger than `max_bytes`.
pub(crate) async fn post_cloud_to_file(
    api_url: String,
    path: &'static str,
    body: serde_json::Value,
    destination: std::path::PathBuf,
    max_bytes: u64,
) -> Result<(), DaemonError> {
    tokio::task::spawn_blocking(move || {
        let io_error = |message: String| DaemonError::LocalTransport {
            operation: "download cloud artifact",
            message,
        };
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(30 * 60))
            .build();
        let response = agent
            .post(&format!("{api_url}{path}"))
            .set("content-type", "application/json")
            .send_string(&body.to_string())
            .map_err(cloud_transport_error)?;
        let partial = destination.with_extension("partial");
        let written = (|| {
            let mut file =
                std::fs::File::create(&partial).map_err(|error| io_error(error.to_string()))?;
            let parent = destination
                .parent()
                .ok_or_else(|| io_error("artifact has no parent directory".into()))?;
            bounded_artifact::copy(response.into_reader(), &mut file, max_bytes, || {
                fs2::available_space(parent)
            })
            .map_err(|error| io_error(error.to_string()))?;
            file.sync_all()
                .map_err(|error| io_error(error.to_string()))?;
            std::fs::rename(&partial, &destination).map_err(|error| io_error(error.to_string()))
        })();
        if written.is_err() {
            let _ = std::fs::remove_file(&partial);
        }
        written
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "download cloud artifact",
        message: error.to_string(),
    })?
}

fn post_cloud_acknowledged_blocking_with_timeout(
    api_url: String,
    path: &str,
    body: serde_json::Value,
    timeout: std::time::Duration,
) -> Result<(), DaemonError> {
    let agent = ureq::AgentBuilder::new()
        .timeout(timeout)
        .redirects(0)
        .build();
    let response = agent
        .post(&format!("{api_url}{path}"))
        .set("content-type", "application/json")
        .send_string(&body.to_string())
        .map_err(cloud_transport_error)?;
    match response.status() {
        200 | 204 => Ok(()),
        status => Err(DaemonError::LocalTransport {
            operation: "acknowledge cloud relay request",
            message: format!("Cloud did not acknowledge completed logout (HTTP {status})"),
        }),
    }
}

fn post_cloud_json_blocking<T>(
    api_url: String,
    path: &str,
    body: serde_json::Value,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned,
{
    post_cloud_json_blocking_with_timeout(api_url, path, body, CLOUD_API_REQUEST_TIMEOUT)
}

fn post_cloud_json_blocking_with_timeout<T>(
    api_url: String,
    path: &str,
    body: serde_json::Value,
    timeout: std::time::Duration,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned,
{
    let url = format!("{api_url}{path}");
    let agent = ureq::AgentBuilder::new().timeout(timeout).build();
    let response = agent
        .post(&url)
        .set("content-type", "application/json")
        .send_string(&body.to_string())
        .map_err(cloud_transport_error)?;
    let payload = response
        .into_string()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "read cloud relay response",
            message: error.to_string(),
        })?;
    serde_json::from_str::<T>(&payload).map_err(|error| DaemonError::LocalTransport {
        operation: "decode cloud relay response",
        message: error.to_string(),
    })
}

fn get_cloud_json_blocking<T>(api_url: String, path: &str) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned,
{
    let url = format!("{api_url}{path}");
    let agent = ureq::AgentBuilder::new()
        .timeout(CLOUD_API_REQUEST_TIMEOUT)
        .build();
    let response = agent.get(&url).call().map_err(cloud_transport_error)?;
    let payload = response
        .into_string()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "read cloud relay response",
            message: error.to_string(),
        })?;
    serde_json::from_str::<T>(&payload).map_err(|error| DaemonError::LocalTransport {
        operation: "decode cloud relay response",
        message: error.to_string(),
    })
}

fn get_cloud_json_authenticated_blocking<T>(
    api_url: String,
    path: &str,
    bearer_token: &str,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned,
{
    let url = format!("{api_url}{path}");
    let agent = ureq::AgentBuilder::new()
        .timeout(CLOUD_API_REQUEST_TIMEOUT)
        .build();
    let response = agent
        .get(&url)
        .set("authorization", &format!("Bearer {bearer_token}"))
        .call()
        .map_err(cloud_transport_error)?;
    decode_cloud_response(response)
}

fn post_cloud_json_authenticated_blocking<T>(
    api_url: String,
    path: &str,
    bearer_token: &str,
    body: serde_json::Value,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned,
{
    let url = format!("{api_url}{path}");
    let agent = ureq::AgentBuilder::new()
        .timeout(CLOUD_API_REQUEST_TIMEOUT)
        .build();
    let response = agent
        .post(&url)
        .set("authorization", &format!("Bearer {bearer_token}"))
        .set("content-type", "application/json")
        .send_string(&body.to_string())
        .map_err(cloud_transport_error)?;
    decode_cloud_response(response)
}

fn decode_cloud_response<T>(response: ureq::Response) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned,
{
    let payload = response
        .into_string()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "read cloud relay response",
            message: error.to_string(),
        })?;
    serde_json::from_str::<T>(&payload).map_err(|error| DaemonError::LocalTransport {
        operation: "decode cloud relay response",
        message: error.to_string(),
    })
}

pub(crate) fn cloud_url_component(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            _ => format!("%{byte:02X}").chars().collect(),
        })
        .collect()
}

fn cloud_transport_error(error: ureq::Error) -> DaemonError {
    let message = match error {
        ureq::Error::Status(status, response) => {
            let body = response.into_string().unwrap_or_default();
            if body.is_empty() {
                format!("cloud relay request failed with {status}")
            } else if let Some(code) = cloud_api_error_code(&body) {
                format!("cloud relay request failed with {status}: cloud_api_code={code}")
            } else {
                format!("cloud relay request failed with {status}")
            }
        }
        ureq::Error::Transport(error) => error.to_string(),
    };
    DaemonError::LocalTransport {
        operation: "cloud relay request",
        message,
    }
}

fn cloud_api_error_code(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|payload| {
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(|code| code.as_str())
                .map(str::to_string)
        })
}

pub(crate) fn cloud_error_code(error: &DaemonError) -> Option<&str> {
    let message = match error {
        DaemonError::LocalTransport { message, .. } => message,
        _ => return None,
    };
    let code = message.split_once("cloud_api_code=")?.1;
    Some(code.split(':').next().unwrap_or(code))
}

pub(crate) fn cloud_error_is_retryable(error: &DaemonError) -> bool {
    if let Some(code) = cloud_error_code(error) {
        match code {
            "capacity_exceeded" | "dependency_unavailable" | "internal_error" | "rate_limited" => {
                return true
            }
            "account_deleted"
            | "authorization_denied"
            | "identity_conflict"
            | "identity_revoked"
            | "invalid_request"
            | "not_found"
            | "realm_not_found"
            | "session_invalid"
            | "subscription_required"
            | "user_deleted" => return false,
            _ => {}
        }
    }
    let (operation, message) = match error {
        DaemonError::LocalTransport {
            operation, message, ..
        } => (*operation, message.as_str()),
        _ => return true,
    };
    if operation == "decode cloud relay response" {
        return true;
    }
    ![
        "cloud relay request failed with 400",
        "cloud relay request failed with 401",
        "cloud relay request failed with 402",
        "cloud relay request failed with 403",
        "cloud relay request failed with 404",
        "cloud relay request failed with 409",
        "cloud relay request failed with 410",
        "cloud relay request failed with 422",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

pub(crate) fn is_stale_cloud_link_error(error: &DaemonError) -> bool {
    let message = match error {
        DaemonError::LocalTransport { message, .. } => message.as_str(),
        _ => return false,
    };
    [
        "cloud_api_code=session_invalid",
        "cloud_api_code=realm_not_found",
        "cloud_api_code=account_deleted",
        "cloud_api_code=user_deleted",
        "\"code\":\"session_invalid\"",
        "\"code\":\"realm_not_found\"",
        "\"code\":\"account_deleted\"",
        "\"code\":\"user_deleted\"",
        "invalid_session",
        "cloud relay request failed with 401",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_url_component_percent_encodes_query_values() {
        assert_eq!(
            cloud_url_component("token/a+b?x=1"),
            "token%2Fa%2Bb%3Fx%3D1"
        );
        assert_eq!(cloud_url_component("abc-_.~XYZ"), "abc-_.~XYZ");
    }

    #[test]
    fn cloud_api_error_code_reads_cloud_error_payloads() {
        assert_eq!(
            cloud_api_error_code(r#"{"error":{"code":"session_invalid"}}"#),
            Some("session_invalid".to_string())
        );
        assert_eq!(cloud_api_error_code(r#"{"error":{}}"#), None);
    }

    #[test]
    fn stale_cloud_link_errors_only_include_invalid_cloud_sessions() {
        assert!(!is_stale_cloud_link_error(&DaemonError::LocalTransport {
            operation: "cloud relay request",
            message: "cloud_api_code=identity_revoked".to_string(),
        }));
        assert!(is_stale_cloud_link_error(&DaemonError::LocalTransport {
            operation: "cloud relay request",
            message: "cloud relay request failed with 401".to_string(),
        }));
        assert!(!is_stale_cloud_link_error(&DaemonError::LocalTransport {
            operation: "cloud relay request",
            message: "network timeout".to_string(),
        }));
        assert!(!is_stale_cloud_link_error(&DaemonError::LocalTransport {
            operation: "cloud relay request",
            message: "cloud relay request failed with 403".to_string(),
        }));
    }

    #[test]
    fn cloud_error_code_preserves_structured_api_failures() {
        let error = DaemonError::LocalTransport {
            operation: "cloud relay request",
            message: "cloud relay request failed with 409: cloud_api_code=identity_conflict: body"
                .to_string(),
        };
        assert_eq!(cloud_error_code(&error), Some("identity_conflict"));
        assert_eq!(
            cloud_error_code(&DaemonError::LocalTransport {
                operation: "cloud relay request",
                message: "network timeout".to_string(),
            }),
            None
        );
    }

    #[test]
    fn cloud_error_retryability_preserves_terminal_and_transient_classes() {
        assert!(!cloud_error_is_retryable(&DaemonError::LocalTransport {
            operation: "cloud relay request",
            message: "cloud relay request failed with 409: cloud_api_code=identity_conflict: body"
                .to_string(),
        }));
        assert!(cloud_error_is_retryable(&DaemonError::LocalTransport {
            operation: "cloud relay request",
            message:
                "cloud relay request failed with 503: cloud_api_code=dependency_unavailable: body"
                    .to_string(),
        }));
        assert!(cloud_error_is_retryable(&DaemonError::LocalTransport {
            operation: "cloud relay request",
            message:
                "cloud relay request failed with 503: cloud_api_code=service_unavailable: body"
                    .to_string(),
        }));
        assert!(cloud_error_is_retryable(&DaemonError::LocalTransport {
            operation: "cloud relay request",
            message: "network timeout".to_string(),
        }));
        assert!(cloud_error_is_retryable(&DaemonError::LocalTransport {
            operation: "decode cloud relay response",
            message: "unexpected response shape".to_string(),
        }));
    }

    fn read_fixture_request(stream: &mut std::net::TcpStream) {
        use std::io::{BufRead, BufReader, Read};

        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .expect("bound fixture request read");
        let mut reader = BufReader::new(stream);
        let mut content_length = 0;
        let mut line = String::new();
        loop {
            line.clear();
            assert!(reader.read_line(&mut line).expect("read request headers") > 0);
            if line == "\r\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    content_length = value.trim().parse().expect("request body length");
                }
            }
        }
        let mut body = vec![0; content_length];
        reader
            .read_exact(&mut body)
            .expect("read complete request body");
    }

    fn acknowledgement_fixture(status: u16, body: &str) -> Result<(), DaemonError> {
        use std::io::Write;
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("bind Cloud acknowledgement fixture");
        let address = listener.local_addr().expect("Cloud fixture address");
        let response = format!(
            "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let fixture = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept Cloud request");
            read_fixture_request(&mut stream);
            stream
                .write_all(response.as_bytes())
                .expect("send Cloud acknowledgement fixture");
        });
        let result = post_cloud_acknowledged_blocking_with_timeout(
            format!("http://{address}"),
            "/auth/logout",
            serde_json::json!({}),
            std::time::Duration::from_secs(1),
        );
        fixture.join().expect("Cloud acknowledgement fixture");
        result
    }

    #[test]
    fn cloud_acknowledgement_accepts_an_empty_completed_204_response() {
        let result = acknowledgement_fixture(204, "");
        assert!(
            result.is_ok(),
            "HTTP 204 must acknowledge completed revocation: {result:?}"
        );
    }

    #[test]
    fn cloud_acknowledgement_accepts_completed_200_without_requiring_json() {
        assert!(acknowledgement_fixture(200, "completed").is_ok());
    }

    #[test]
    fn cloud_acknowledgement_rejects_pending_redirect_and_failed_responses() {
        for status in [201, 202, 301, 302, 307, 308, 400, 401, 403, 409, 500] {
            assert!(
                acknowledgement_fixture(status, "").is_err(),
                "HTTP {status} is not completed acknowledgement"
            );
        }
        let error = acknowledgement_fixture(401, r#"{"error":{"code":"session_invalid"}}"#)
            .expect_err("Cloud failure must remain an error");
        assert_eq!(cloud_error_code(&error), Some("session_invalid"));
    }

    #[test]
    fn cloud_acknowledgement_does_not_follow_a_redirect_to_success() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind redirect fixture");
        let address = listener.local_addr().expect("redirect fixture address");
        let fixture = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept original logout");
            read_fixture_request(&mut stream);
            let response = format!("HTTP/1.1 302 Found\r\nLocation: http://{address}/redirected\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            stream
                .write_all(response.as_bytes())
                .expect("send redirect");
            drop(stream);
            listener
                .set_nonblocking(true)
                .expect("poll redirect listener");
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
            while std::time::Instant::now() < deadline {
                if let Ok((mut followed, _)) = listener.accept() {
                    read_fixture_request(&mut followed);
                    followed
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .expect("send redirected success");
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            false
        });
        let result = post_cloud_acknowledged_blocking_with_timeout(
            format!("http://{address}"),
            "/auth/logout",
            serde_json::json!({}),
            std::time::Duration::from_secs(1),
        );
        assert!(result.is_err());
        assert!(
            !fixture.join().expect("redirect fixture"),
            "logout must not follow redirects"
        );
    }

    #[test]
    fn cloud_post_has_a_total_response_deadline() {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stalled Cloud fixture");
        let address = listener
            .local_addr()
            .expect("stalled Cloud fixture address");
        let fixture = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().expect("accept stalled Cloud request");
            std::thread::sleep(std::time::Duration::from_millis(250));
        });
        let started = std::time::Instant::now();
        let result = post_cloud_json_blocking_with_timeout::<serde_json::Value>(
            format!("http://{address}"),
            "/stalled",
            serde_json::json!({}),
            std::time::Duration::from_millis(50),
        );
        assert!(result.is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        fixture.join().expect("stalled Cloud fixture");
    }
}
