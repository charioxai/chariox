use std::time::Duration;

use chariox_event_protocol::{PublishEventRequest, PublishEventResponse};

/// One publication may not hold the retry worker longer than this; a hung AEDS connection
/// otherwise stalls every stored event behind it.
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone)]
pub struct AedsPublisher {
    producer_id: String,
    producer_token: Option<String>,
    events_url: String,
    request_timeout: Duration,
}

impl AedsPublisher {
    pub fn new(
        producer_id: impl Into<String>,
        producer_token: Option<String>,
        events_url: impl Into<String>,
    ) -> Self {
        Self {
            producer_id: producer_id.into(),
            producer_token,
            events_url: events_url.into(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }

    pub fn with_request_timeout(mut self, request_timeout: Duration) -> Self {
        self.request_timeout = request_timeout;
        self
    }

    pub async fn publish(
        &self,
        mut event: PublishEventRequest,
    ) -> Result<PublishEventResponse, String> {
        event.producer_id = self.producer_id.clone();
        let url = self.events_url.clone();
        let token = self.producer_token.clone();
        let timeout = self.request_timeout;
        tokio::task::spawn_blocking(move || {
            let mut request = ureq::AgentBuilder::new()
                .timeout(timeout)
                .build()
                .post(&url)
                .set("content-type", "application/json");
            if let Some(token) = token {
                request = request.set("authorization", &format!("Bearer {token}"));
            }
            let body = serde_json::to_string(&event).map_err(|error| error.to_string())?;
            // The error text of ureq carries the service URL; owners see this message.
            let response = request.send_string(&body).map_err(|error| match error {
                ureq::Error::Status(status, _) => {
                    format!("the event service returned HTTP {status}")
                }
                ureq::Error::Transport(_) => {
                    "the event service did not answer or could not be reached".to_string()
                }
            })?;
            let response_body = response.into_string().map_err(|error| error.to_string())?;
            serde_json::from_str(&response_body).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())?
    }
}
