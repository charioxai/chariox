//! Poll subscription boundary and durable delivery through real management HTTP.
use super::*;

struct PollFilterProvider;

impl AegsProvider for PollFilterProvider {
    fn generator_id(&self) -> &'static str {
        "dev.chariox.github"
    }
    fn provider_slug(&self) -> &'static str {
        "github"
    }
    fn validate_subscription(&self, claim: &crate::SubscriptionClaim) -> Result<(), String> {
        if claim.filter["packages"].is_array() {
            Ok(())
        } else {
            Err("packages must be an array".into())
        }
    }
    fn normalize_webhook(
        &self,
        _: WebhookInput<'_>,
        _: &crate::WebhookRoute,
    ) -> Result<crate::NormalizedEvent, String> {
        Err("this source polls; webhooks are unsupported".into())
    }
}

#[tokio::test]
async fn polling_filter_is_validated_before_reconciliation_and_matches_durable_delivery() {
    let publisher = PublisherFixture::new();
    let (mut server, store) = subscribed_server(&publisher);
    let inner = Arc::get_mut(&mut server).unwrap();
    inner.management_token = Some("management-token".into());
    inner.provider = Arc::new(PollFilterProvider);
    let mut claim = subscription_claim();
    claim.revision = 2;
    claim.filter = serde_json::json!({"packages": "invalid"});
    let body = |claim: &crate::SubscriptionClaim| {
        serde_json::json!({
            "owner_id":"owner", "generator_id":"dev.chariox.github", "subscriptions":[claim]
        })
    };
    let (status, _) = management_request(
        &server,
        "PUT",
        "/v1/subscriptions/reconcile",
        "owner",
        body(&claim),
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(store.all("dev.chariox.github").unwrap()[0].revision, 1);
    claim.filter = serde_json::json!({"packages": ["npm:package"]});
    let (status, _) = management_request(
        &server,
        "PUT",
        "/v1/subscriptions/reconcile",
        "owner",
        body(&claim),
    )
    .await;
    assert_eq!(status, 200);
    let mut event = crate::NormalizedEvent {
        data: None,
        occurrence_id: "upstream:1".into(),
        event_type: "event".into(),
        occurred_at: "2026-01-01T00:00:00.000Z".into(),
        connection_scope: "scope".into(),
        prompt: "An upstream fact changed.".into(),
        metadata: serde_json::json!({"packages":["npm:different"]}),
        reply_context: None,
    };
    store
        .record_inbox_event("wrong-watch", None, &event, now_ms())
        .unwrap();
    server
        .attempt_inbox_delivery("wrong-watch", None, now_ms())
        .await
        .unwrap();
    assert_eq!(publisher.calls.load(Ordering::SeqCst), 0);
    event.metadata["packages"] = claim.filter["packages"].clone();
    store
        .record_inbox_event("right-watch", None, &event, now_ms())
        .unwrap();
    server
        .attempt_inbox_delivery("right-watch", None, now_ms())
        .await
        .unwrap();
    server
        .attempt_inbox_delivery("right-watch", None, now_ms())
        .await
        .unwrap();
    assert_eq!(publisher.calls.load(Ordering::SeqCst), 1);
}
