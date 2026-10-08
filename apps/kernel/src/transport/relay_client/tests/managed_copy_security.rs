use super::support::*;
use crate::account_profile::*;
use base64::Engine;

#[test]
fn secrev_f2_encrypted_slice_import_replay_cannot_restore_removed_login() {
    crate::test_support::isolated_env_test!();
    run_async_with_large_test_stack("slice-copy-replay", || async {
        let _guard = relay_client_test_guard().await;
        let _home = RelayTestHome::new();
        let root = crate::test_support::TestWorktree::new("slice-copy-replay");
        let source = DaemonConfig::for_tests();
        let mut target = DaemonConfig::for_tests();
        target.daemon_id = "replay-slice-kernel".into();
        target.host_machine_id = "replay-slice-machine".into();
        target.managed_slice_relay_owner_public_key = Some(source.relay_public_key.clone());
        std::env::set_var("CHARIOX_SLICE_ID", "replay-slice");
        std::env::set_var("CHARIOX_SLICE_OWNER_KERNEL_ID", &source.daemon_id);
        std::env::set_var("CHARIOX_SLICE_OWNER_MACHINE_ID", &source.host_machine_id);
        let binary = root.path().join("opencode");
        std::fs::write(&binary, "#!/bin/sh\nprintf 'fixture-opencode\\n'\n").unwrap();
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::env::set_var("CHARIOX_OPENCODE_BIN", binary);
        let app = Arc::new(Mutex::new(DaemonApp::bootstrap(target.clone()).unwrap()));
        let router = Arc::new(crate::runtime::router::CommandRouter::with_interactive_capacity(app, 2));
        let registry = router.runtime_state().provider_account_profile_registry().clone();
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (outgoing, _priority, _events) = RelayOutgoingSender::channel(32);
        let mut materialization = ProviderAccountMaterialization {
            copy_source: Some(ProviderAccountCopySource { machine_id: source.host_machine_id.clone(), kernel_id: source.daemon_id.clone() }),
            profile: ProviderAccountReplicaMetadata { owner_user_id: "local".into(), provider: "opencode".into(), profile_id: "replay-account".into(), label: "Synthetic copy".into(), origin: ProviderAccountProfileOrigin::CharioxCreated, is_default: false },
            files: vec![ProviderAccountMaterializationFile { relative_path: "data/opencode/auth.json".into(), contents_base64: base64::engine::general_purpose::STANDARD.encode(br#"{"openai":{"type":"api","key":"synthetic"}}"#) }],
            generated_at_ms: 1,
        };
        let frame = relay_crypto::encrypt_payload_for_peer(&source.relay_private_key, &target.relay_public_key,
            &serde_json::to_vec(&RelayPeerRequest::ImportManagedSliceProviderAccountCopy { slice_id: "replay-slice".into(), materialization: materialization.clone() }).unwrap()).unwrap();
        let imported = super::super::peer_requests::handle_daemon_peer_request(&router, &state, &outgoing, &source.daemon_id, None, frame.clone()).await;
        assert!(imported.error.is_none(), "initial import: {:?}", imported.error);
        materialization.generated_at_ms = 2;
        let no_op_frame = relay_crypto::encrypt_payload_for_peer(&source.relay_private_key, &target.relay_public_key,
            &serde_json::to_vec(&RelayPeerRequest::ImportManagedSliceProviderAccountCopy {slice_id: "replay-slice".into(), materialization}).unwrap()).unwrap();
        let no_op = super::super::peer_requests::handle_daemon_peer_request(&router, &state, &outgoing, &source.daemon_id, None, no_op_frame.clone()).await;
        assert!(no_op.error.is_none(), "a fresh owner import may preserve the receiving login");
        let environment = registry.resolve_environment("local", "opencode", "replay-account").unwrap();
        let auth = std::path::Path::new(&environment["XDG_DATA_HOME"]).join("opencode/auth.json");
        assert!(auth.is_file());
        // The Docker removal helper removes this official artifact. Keep persisted
        // copy metadata and replay the exact encrypted frame.
        std::fs::remove_file(&auth).unwrap();
        let replay = super::super::peer_requests::handle_daemon_peer_request(&router, &state, &outgoing, &source.daemon_id, None, frame).await;
        assert!(!auth.exists(), "relay replay restored the removed credential");
        assert!(replay.error.is_some(), "stale import must be rejected");
        let replay = super::super::peer_requests::handle_daemon_peer_request(&router, &state, &outgoing, &source.daemon_id, None, no_op_frame).await;
        assert!(replay.error.is_some(), "a no-op import must also be consumed");
        assert!(!auth.exists(), "replaying a no-op request restored the credential");
    });
}
