//! MD-DISPLAY-04: transient viewer-specific frame events, never replayed/logged.
use super::kernel_protocol::KernelEvent;
use serde_json::Value;

// MD-DISPLAY-02/04: local diagnostics contain only a fixed stage and times.
pub(crate) fn timing(stage: &'static str, started: std::time::Instant) {
    if std::env::var("CHARIOX_BROWSER_DISPLAY_TIMING").as_deref() == Ok("1") {
        let duration = started.elapsed().as_secs_f64() * 1000.0;
        let ended = crate::session::unix_epoch_ms() as f64;
        eprintln!(
            "MD-DISPLAY-TIMING {}",
            serde_json::json!({"stage":stage,"started_ms":ended-duration,"ended_ms":ended,"duration_ms":duration})
        );
    }
}

pub(crate) fn take_display_event(response: &mut Value) -> Option<(String, u64, KernelEvent)> {
    let frame = response.pointer_mut("/KernelBrowser/result/display_frame")?;
    if frame.is_null() {
        return None;
    }
    let subscription_id = frame.get("subscription_id")?.as_str()?.to_string();
    let sequence = frame.get("sequence")?.as_u64()?;
    let frame = frame.take();
    Some((
        subscription_id.clone(),
        sequence,
        KernelEvent::KernelBrowserFrame {
            subscription_id,
            frame,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transient_display_event_keeps_bytes_out_of_response_and_trace() {
        let mut reply = serde_json::json!({"KernelBrowser":{"result":{"display_frame":{"subscription_id":"s","sequence":2,"kind":"png","data_base64":"private-pixels"}}}});
        let (id, sequence, event) = take_display_event(&mut reply).unwrap();
        assert_eq!((id.as_str(), sequence), ("s", 2));
        assert!(reply["KernelBrowser"]["result"]["display_frame"].is_null());
        assert!(
            !super::super::kernel_protocol::kernel_event_trace_payload(2, &event)
                .to_string()
                .contains("private-pixels")
        );
        assert!(!super::super::kernel_protocol::event_is_relevant_to_attachment(&event, "any"));
    }
}
