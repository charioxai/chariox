//! MD-DISPLAY-04: transient viewer-specific frame events, never replayed/logged.
use super::kernel_protocol::KernelEvent;
use serde_json::Value;

// MD-DISPLAY-02/04: local diagnostics contain only a fixed stage and times.
pub(crate) fn timing(stage: &'static str, started: std::time::Instant) {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ENABLED
        .get_or_init(|| std::env::var("CHARIOX_BROWSER_DISPLAY_TIMING").as_deref() == Ok("1"))
    {
        let duration = started.elapsed().as_secs_f64() * 1000.0;
        let ended = crate::session::unix_epoch_ms() as f64;
        eprintln!(
            "MD-DISPLAY-TIMING {}",
            serde_json::json!({"stage":stage,"started_ms":ended-duration,"ended_ms":ended,"duration_ms":duration})
        );
    }
}

/// MP-08/MP-10: protocol 466 binary display event. Layout: magic `CXD1`,
/// big-endian u32 header length, UTF-8 JSON header
/// `{"event":"kernel_browser_frame","subscription_id",frame}`, then raw payload
/// bytes. Every `data_base64` of the frame, its tiles and stripes becomes
/// `data:[offset,length]`; segments are contiguous, in header order, and cover
/// the payload exactly. JSON plaintexts never start with this magic.
pub(crate) const DISPLAY_EVENT_MAGIC: &[u8; 4] = b"CXD1";
const DISPLAY_HEADER_LIMIT: usize = 64 * 1024;

pub(crate) fn encode_display_event(event: KernelEvent) -> Result<Vec<u8>, String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let KernelEvent::KernelBrowserFrame {
        subscription_id,
        mut frame,
    } = event
    else {
        return Err("MD-DISPLAY: not a display event".into());
    };
    let mut payload = Vec::new();
    let mut take = |value: &mut Value| -> Result<(), String> {
        let Some(object) = value.as_object_mut() else {
            return Err("MD-DISPLAY: display segment shape".into());
        };
        let Some(data) = object.remove("data_base64") else {
            return Ok(());
        };
        let bytes = STANDARD
            .decode(data.as_str().ok_or("MD-DISPLAY: display segment encoding")?)
            .map_err(|_| "MD-DISPLAY: display segment encoding")?;
        object.insert(
            "data".into(),
            serde_json::json!([payload.len(), bytes.len()]),
        );
        payload.extend_from_slice(&bytes);
        Ok(())
    };
    take(&mut frame)?;
    for key in ["tiles", "stripes"] {
        if let Some(list) = frame.get_mut(key).and_then(Value::as_array_mut) {
            for segment in list {
                take(segment)?;
            }
        }
    }
    let header = serde_json::to_vec(&serde_json::json!({
        "event": "kernel_browser_frame",
        "subscription_id": subscription_id,
        "frame": frame,
    }))
    .map_err(|_| "MD-DISPLAY: display header")?;
    if header.len() > DISPLAY_HEADER_LIMIT {
        return Err("MD-DISPLAY: display header bound".into());
    }
    let mut bytes = Vec::with_capacity(8 + header.len() + payload.len());
    bytes.extend_from_slice(DISPLAY_EVENT_MAGIC);
    bytes.extend_from_slice(&(header.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&header);
    bytes.extend_from_slice(&payload);
    Ok(bytes)
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
    #[test]
    fn mp08_binary_display_event_carries_raw_contiguous_segments() {
        let event = KernelEvent::KernelBrowserFrame {
            subscription_id: "s".into(),
            frame: serde_json::json!({"kind":"tiles","sequence":3,"tiles":[
                {"x":0,"y":0,"width":2,"height":1,"format":"webp","data_base64":"AAEC"},
                {"x":2,"y":0,"width":2,"height":1,"format":"png","data_base64":"/w=="}],
                "moves":[[0,1,4,1,-1]]}),
        };
        let bytes = encode_display_event(event).unwrap();
        assert_eq!(&bytes[..4], b"CXD1");
        let length = u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize;
        let header: Value = serde_json::from_slice(&bytes[8..8 + length]).unwrap();
        assert_eq!(header["event"], "kernel_browser_frame");
        assert_eq!(header["frame"]["tiles"][0]["data"], serde_json::json!([0, 3]));
        assert_eq!(header["frame"]["tiles"][1]["data"], serde_json::json!([3, 1]));
        assert!(header.to_string().find("data_base64").is_none());
        assert_eq!(&bytes[8 + length..], &[0, 1, 2, 255]);
        let bad = KernelEvent::KernelBrowserFrame {
            subscription_id: "s".into(),
            frame: serde_json::json!({"kind":"png","data_base64":"not base64!"}),
        };
        assert!(encode_display_event(bad).is_err());
    }
}
