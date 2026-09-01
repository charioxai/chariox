use super::{
    SliceDisplayBackend, SliceDisplayEndpoint, SliceDisplayEndpointAccess,
    SliceDisplayEndpointKind, SliceDisplayMode,
};

pub(super) fn display_endpoint_for_slice(
    slice_id: &str,
    mode: &SliceDisplayMode,
    backend: SliceDisplayBackend,
    port: u16,
    custom_url: Option<String>,
) -> Option<SliceDisplayEndpoint> {
    if *mode != SliceDisplayMode::Headed {
        return None;
    }
    let (kind, default_url, capabilities) = match backend {
        SliceDisplayBackend::Novnc => (
            SliceDisplayEndpointKind::Novnc,
            format!("http://127.0.0.1:{port}/vnc.html?host=127.0.0.1&port={port}&autoconnect=true&resize=scale"),
            vec!["view", "keyboard", "mouse"],
        ),
        SliceDisplayBackend::Selkies => (
            SliceDisplayEndpointKind::Selkies,
            format!("http://127.0.0.1:{port}/"),
            // Direct input stays disabled; kernel-owned input is added separately.
            vec!["view", "websocket", "h264", "software_encoding"],
        ),
    };
    Some(SliceDisplayEndpoint {
        slice_id: slice_id.to_string(),
        kind,
        url: custom_url.unwrap_or(default_url),
        access: SliceDisplayEndpointAccess::Local,
        expires_at_ms: None,
        capabilities: capabilities.into_iter().map(str::to_string).collect(),
    })
}
