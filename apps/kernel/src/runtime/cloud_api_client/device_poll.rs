//! Device poll capability negotiation with pre-denial Cloud servers.

use super::http::{
    cloud_status_error, cloud_transport_error, decode_cloud_response, CLOUD_API_REQUEST_TIMEOUT,
};
use super::CloudDevicePollResponse;
use crate::error::DaemonError;

pub(crate) async fn post_cloud_device_poll(
    api_url: String,
    device_code: String,
    supports_access_denied: bool,
) -> Result<CloudDevicePollResponse, DaemonError> {
    tokio::task::spawn_blocking(move || {
        let legacy_body = serde_json::json!({ "deviceCode": device_code });
        let mut body = legacy_body.clone();
        if supports_access_denied {
            body["supportsAccessDenied"] = serde_json::json!(true);
        }
        let agent = ureq::AgentBuilder::new()
            .timeout(CLOUD_API_REQUEST_TIMEOUT)
            .build();
        let request = agent
            .post(&format!("{api_url}/auth/device/poll"))
            .set("content-type", "application/json");
        let response = match request.clone().send_string(&body.to_string()) {
            Err(ureq::Error::Status(400, response)) if supports_access_denied => {
                let payload = response.into_string().unwrap_or_default();
                let error = serde_json::from_str::<serde_json::Value>(&payload).unwrap_or_default();
                // Legacy Cloud does not identify the rejected field. Only its
                // canonical schema rejection permits one retry without it.
                if error["error"]["code"] == "invalid_request"
                    && error["error"]["message"] == "Request validation failed"
                {
                    request
                        .send_string(&legacy_body.to_string())
                        .map_err(cloud_transport_error)?
                } else {
                    return Err(cloud_status_error(400, payload));
                }
            }
            result => result.map_err(cloud_transport_error)?,
        };
        decode_cloud_response(response)
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "poll cloud relay login",
        message: error.to_string(),
    })?
}
