use super::*;

// MP-08 / MP-10: 910 adds the optional `images` data-URL list and the
// `images_omitted` count to the ProviderTool payload that clients render.
#[test]
fn mp10_provider_tool_images_910_snapshot_and_hash() {
    let bytes = include_bytes!("provider-tool-images-910.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "30564ce000e4e72eed8e86181e32c84a7db4a7c67f0f2155150065dc1e6d1079"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local_protocol"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(
        fixture["peer_protocol"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    let with_image = &fixture["tool_with_image"];
    assert!(with_image["images"][0]
        .as_str()
        .unwrap()
        .starts_with("data:image/png;base64,"));
    assert_eq!(fixture["tool_with_omitted_image"]["images_omitted"], 1);
}

#[test]
fn mp10_provider_tool_images_survive_the_terminal_caps_whole() {
    let fixture: serde_json::Value =
        serde_json::from_slice(include_bytes!("provider-tool-images-910.json")).unwrap();
    let mut payload = fixture["tool_with_image"].clone();
    let image = format!("data:image/png;base64,{}", "A".repeat(64 * 1024));
    payload["images"] = serde_json::json!([image]);
    let bounded = crate::provider_output_policy::output_bounds::bounded_terminal_output_bytes(
        &crate::terminal::TerminalOutputKind::ProviderTool,
        payload.to_string().as_bytes(),
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bounded).unwrap(),
        payload
    );
}
