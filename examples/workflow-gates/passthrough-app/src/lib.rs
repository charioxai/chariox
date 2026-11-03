//! The signed incoming schema validates facts. Policy lives in the workflow gate.
wit_bindgen::generate!({
    path: "../../../packages/app-wit/wit",
    inline: r#"package chariox:upwatch;
        world passthrough {
            import chariox:app/registry@1.0.0;
            import chariox:app/events@1.0.0;
            export chariox:app/handlers@1.0.0;
        }"#,
    world: "passthrough", generate_all,
});
use chariox::app::{
    app_types::{AppError, Invocation, InvocationContext, Occurrence, Wake},
    events, registry,
};
use exports::chariox::app::handlers::Guest;
use serde_json::Value;
use wit_bindgen::FutureReader;
struct App;
export!(App);
fn error(message: &str) -> AppError {
    AppError {
        code: "INVALID_INPUT".into(),
        message: message.into(),
        retryable: Some(false),
    }
}
impl Guest for App {
    fn register() -> Result<(), AppError> {
        registry::event("upstream_fact");
        Ok(())
    }
    async fn call_tool(
        _: String,
        _: String,
        _: InvocationContext,
        _: FutureReader<String>,
    ) -> Result<String, AppError> {
        Err(error("no tools"))
    }
    async fn deliver_event(
        name: String,
        occurrence_id: String,
        payload: String,
        _: InvocationContext,
        _: FutureReader<String>,
    ) -> Result<String, AppError> {
        if name != "upstream_fact" {
            return Err(error("unknown event"));
        }
        let input: Value =
            serde_json::from_str(&payload).map_err(|_| error("invalid source envelope"))?;
        let fact = input
            .get("data")
            .filter(|v| v.is_object())
            .ok_or_else(|| error("typed fact required"))?;
        let occurred_at_ms = chrono::DateTime::parse_from_rfc3339(
            input["occurred_at"]
                .as_str()
                .ok_or_else(|| error("source time required"))?,
        )
        .map_err(|_| error("source time invalid"))?
        .timestamp_millis()
        .try_into()
        .map_err(|_| error("source time precedes epoch"))?;
        let outgoing_id =
            events::occurrence_id(&format!("upwatch:{occurrence_id}"), occurred_at_ms)?;
        let mut receipts = Vec::new();
        for automation in events::automations().await? {
            if automation.event != "public_advisory" || automation.state != "active" {
                continue;
            }
            let receipt = events::emit(Occurrence {
                automation_id: automation.automation_id,
                occurrence_id: outgoing_id.clone(),
                event_version: automation.event_version,
                occurred_at_ms,
                payload: fact.to_string(),
                invocation: Invocation {
                    prompt: "Process this public advisory.".into(),
                    artifacts: vec![],
                },
                schedule_revision: None,
            })
            .await?;
            receipts.push(receipt.receipt_id);
        }
        Ok(serde_json::to_string(&receipts).unwrap())
    }
    async fn lifecycle(
        _: String,
        data: Option<String>,
        _: InvocationContext,
        _: FutureReader<String>,
    ) -> Result<Option<String>, AppError> {
        Ok(data)
    }
    async fn deliver_wake(
        _: Wake,
        _: bool,
        _: InvocationContext,
        _: FutureReader<String>,
    ) -> Result<(), AppError> {
        Ok(())
    }
}
