//! MP-08/MP-10/MP-11: source integration through real kernel/relay/stdio seams.
//! Chromium pixels are synthetic here; actual Chromium is a separate drill.
use super::*;
use futures_util::FutureExt;
use sha2::{Digest, Sha256};

#[test]
fn mp08_mp10_mp11_browser_artifact_client_and_worker_routing() {
    crate::test_support::isolated_env_test!();
    run_test(check_artifacts);
}
async fn check_artifacts() {
    let mut fixture = LiveWorker::start_configured(false, true).await;
    let check = std::panic::AssertUnwindSafe(async {
        let (room, attachment_id, _) = prepare_cross_worker_room_display(&fixture).await;
        let environment = dispatch_json(&fixture.home, json!({"StartRoomEnvironment": {"session_id":room,
            "viewport":{"css_width":1280,"css_height":800,"device_scale_factor":1,
            "desktop_pixel_width":1280,"desktop_pixel_height":800}}})).await.unwrap();
        let tab = environment["RoomEnvironmentUpdated"]["environment"]["focused_tab_id"].as_str().unwrap().to_string();
        let capture = dispatch_json(&fixture.home, json!({"RoomBrowserArtifact": {
            "session_id":room,"attachment_id":attachment_id,"tab_id":tab,
            "operation":{"action":"capture","kind":"image","browser_generation":1,"return_image_base64":true}
        }})).await.unwrap();
        let payload = &capture["RoomBrowserArtifact"]["result"]["payload"];
        let bytes = base64::engine::general_purpose::STANDARD.decode(payload["image_base64"].as_str().unwrap()).unwrap();
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(payload["sha256"], format!("{:x}", Sha256::digest(&bytes)));
        assert_eq!(payload["identity"]["tab_id"], tab);
        assert_eq!(payload["identity"]["document_id"], "worker-document");
        assert_eq!(payload["identity"]["viewport"]["css_width"], 1280);
        assert!(!payload.to_string().contains("operational_path"));
        let artifact_id = payload["artifact_id"].as_str().unwrap();
        let read = dispatch_json(&fixture.home, json!({"RoomBrowserArtifact": {
            "session_id":room,"attachment_id":attachment_id,"tab_id":tab,
            "operation":{"action":"read","artifact_id":artifact_id,"offset":0,"max_bytes":131072}
        }})).await.unwrap();
        assert_eq!(read["RoomBrowserArtifact"]["result"]["payload"]["data_base64"], payload["image_base64"]);
        // MP-08/MP-10/MP-11: the provider tool and attachment client share
        // the real home/worker service. This stub proves routing, not an
        // official-provider/model-visible acceptance run.
        let spawned = dispatch_json(&fixture.home, json!({"SpawnAgent": {
            "session_id":room,"provider":"managed-dev-stub","model":"runtime-mcp-idle"
        }})).await.unwrap();
        let agent_id = spawned["AgentSpawned"]["agent"]["id"].as_str().unwrap();
        dispatch_json(&fixture.home, json!({"LaunchProviderRun": {
            "session_id":room,"agent_id":agent_id,"adapter_key":"managed-dev-stub",
            "provider":"managed-dev-stub","account_profile":"default","model":"runtime-mcp-idle",
            "variant":null,"structured_endpoint":null,"provider_session_id":null,"native_tui":false
        }})).await.unwrap();
        let run_id = fixture.home.app.lock().await.providers()
            .get_latest_run_for_agent(&room, agent_id).unwrap().id().to_string();
        let tool_auth = fixture.home.runtime_state.runtime_mcp_auth_token_for_provider_run(&run_id).unwrap();
        assert!(fixture.home.runtime_state.runtime_tool_specs_for_auth_token(&tool_auth)
            .iter().any(|spec| spec.name == "slice_browser_artifact"));
        let provider_capture = fixture.home.runtime_state.dispatch_authenticated_runtime_tool_call(
            &tool_auth, "slice_browser_artifact", json!({"action":"capture","kind":"image",
                "browser_generation":1,"return_image_base64":true})).await.unwrap();
        assert!(provider_capture.ok);
        assert_eq!(provider_capture.payload["image_base64"], payload["image_base64"]);
        assert_eq!(provider_capture.payload["identity"], payload["identity"]);
        let client_read = dispatch_json(&fixture.home, json!({"RoomBrowserArtifact": {
            "session_id":room,"attachment_id":attachment_id,"tab_id":tab,
            "operation":{"action":"read","artifact_id":provider_capture.payload["artifact_id"],"max_bytes":131072}
        }})).await.unwrap();
        assert_eq!(client_read["RoomBrowserArtifact"]["result"]["payload"]["data_base64"], payload["image_base64"]);
        for operation in [
            json!({"action":"read","artifact_id":artifact_id,"offset":0,"max_bytes":131073}),
            json!({"action":"read","artifact_id":"missing","offset":0,"max_bytes":1}),
            json!({"action":"capture","kind":"image","browser_generation":2}),
        ] {
            assert!(dispatch_json(&fixture.home, json!({"RoomBrowserArtifact":{
                "session_id":room,"attachment_id":attachment_id,"tab_id":tab,"operation":operation}})).await.is_err());
        }
        assert!(dispatch_json(&fixture.home, json!({"RoomBrowserArtifact":{
            "session_id":room,"attachment_id":"unattached","tab_id":tab,
            "operation":{"action":"read","artifact_id":artifact_id,"max_bytes":1}}})).await.is_err());
        let other_room = &fixture.rooms[1];
        assert!(dispatch_json(&fixture.home, json!({"RoomBrowserArtifact":{
            "session_id":other_room,"attachment_id":attachment_id,"tab_id":tab,
            "operation":{"action":"read","artifact_id":artifact_id,"max_bytes":1}}})).await.is_err());
        // MP-08/MP-10/MP-11: a permitted transfer record crosses the real
        // encrypted worker route and private stager without filesystem paths.
        let config = fixture.home.runtime_state.owned.config_projection.snapshot();
        let store = crate::artifacts::OperationalArtifactStore::open(
            config.operational_artifact_root(), config.operational_artifact_index_path()).unwrap();
        let source = fixture.home_state.root.join("transfer-fixture.txt");
        let transferred_bytes = "Transferred Grüße 世界\n".as_bytes();
        std::fs::write(&source, transferred_bytes).unwrap();
        let transfer = store.store_existing_file(crate::artifacts::StoreArtifactRequest {
            source_path: source.clone(), display_name:"transferred-世界.txt".into(),
            media_type:Some("text/plain".into()), source_kind:"transfer".into(), enqueue_archive:false,
            session_id:Some(room.clone()), attachment_id:Some(attachment_id.clone()),
            workspace_id:None, worktree_path:None, metadata:Default::default()
        }).unwrap();
        let snapshot = fixture.home.runtime_state.capture_browser_environment_snapshot(&room, &tab).await.unwrap();
        let field = &snapshot.dom_nodes.iter().find(|node| node.node_name == "INPUT" &&
            node.attributes.get("type").is_some_and(|kind| kind == "file")).unwrap().element_ref;
        let uploaded = fixture.home.runtime_state.dispatch_authenticated_runtime_tool_call(
            &tool_auth, "slice_browser_upload", json!({"field_id":field,"artifact_ids":[transfer.artifact_id]})).await.unwrap();
        assert!(uploaded.ok, "opaque transfer upload failed");
        let receipts: Value = serde_json::from_slice(&std::fs::read(fixture._worker_state.root.join("chromium-state.json")).unwrap()).unwrap();
        assert_eq!(receipts["upload"]["receipts"], json!([{"filename":"transferred-世界.txt",
            "size_bytes":transferred_bytes.len(),"sha256":format!("{:x}", Sha256::digest(transferred_bytes))}]));
        for artifact_ids in [json!(["missing"]), json!([transfer.artifact_id, "missing"])] {
            assert!(fixture.home.runtime_state.dispatch_authenticated_runtime_tool_call(
                &tool_auth, "slice_browser_upload", json!({"field_id":field,"artifact_ids":artifact_ids})).await.is_err());
        }
        let after: Value = serde_json::from_slice(&std::fs::read(fixture._worker_state.root.join("chromium-state.json")).unwrap()).unwrap();
        assert_eq!(after["uploadCount"], receipts["uploadCount"], "missing artifacts cannot partially reuse the prior selection");
        std::fs::remove_file(source).unwrap();
        // MP-08/MP-10/MP-11: a same-tab navigation retires old image reads.
        std::fs::write(fixture._worker_state.root.join("external-browser-navigation"), "navigate").unwrap();
        fixture.home.runtime_state.reconcile_browser_controller_environment(&room).await.unwrap();
        assert!(dispatch_json(&fixture.home, json!({"RoomBrowserArtifact":{
            "session_id":room,"attachment_id":attachment_id,"tab_id":tab,
            "operation":{"action":"read","artifact_id":artifact_id,"max_bytes":1}}})).await.is_err());
    }).catch_unwind().await;
    fixture.stop().await;
    if let Err(panic) = check {
        std::panic::resume_unwind(panic);
    }
}
