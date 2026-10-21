use chariox_event_protocol::{PublishEventRequest, PublishEventResponse};

#[derive(Clone)]
pub struct AedsPublisher {
    producer_id: String,
    producer_token: Option<String>,
    events_url: String,
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
        }
    }

    pub async fn publish(
        &self,
        mut event: PublishEventRequest,
    ) -> Result<PublishEventResponse, String> {
        event.producer_id = self.producer_id.clone();
        let url = self.events_url.clone();
        let token = self.producer_token.clone();
        tokio::task::spawn_blocking(move || {
            let mut request = ureq::post(&url).set("content-type", "application/json");
            if let Some(token) = token {
                request = request.set("authorization", &format!("Bearer {token}"));
            }
            let body = serde_json::to_string(&event).map_err(|error| error.to_string())?;
            let response = request
                .send_string(&body)
                .map_err(|error| error.to_string())?;
            let response_body = response.into_string().map_err(|error| error.to_string())?;
            serde_json::from_str(&response_body).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())?
    }
}
