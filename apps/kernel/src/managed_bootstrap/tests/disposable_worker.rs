use super::*;

#[test]
fn ordinary_bootstrap_rejects_worker_shaped_records_without_mutation() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    for with_receipt in [false, true] {
        let fixture = Fixture::new("legacy-worker-rejection");
        let previous_home = std::env::var_os("CHARIOX_HOME");
        std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
        let binding = serde_json::json!({
            "allocationId": "allocation-1",
            "expectedHomeKernelId": "home-kernel-1",
            "userId": "owner-1",
            "realmId": "realm-1",
            "workerMachineId": "worker-machine",
            "workerKernelId": "worker-kernel",
            "imageDigest": format!("sha256:{}", "c".repeat(64)),
            "runtimeReleaseDigest": fixture.release_digest.clone(),
            "managerOperationId": "operation-1",
            "managerOperationFence": 7,
            "managerRequestDigest": format!("sha256:{}", "d".repeat(64)),
            "senderKeyThumbprint": format!("sha256:{}", "e".repeat(64)),
        });
        let binding_digest = format!("sha256:{}", "f".repeat(64));
        let envelope = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1, "cloudApiUrl": "https://cloud.example.test",
            "token": format!("dwboot_{}", "a".repeat(43)),
            "expiresAt": (fixture.now + chrono::Duration::minutes(5)).to_rfc3339(),
            "bindingDigest": binding_digest, "binding": binding,
        }))
        .unwrap();
        fs::write(&fixture.config.envelope_path, &envelope).unwrap();
        let receipt = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1, "kind": "disposable_worker", "status": "exchange_pending",
            "cloudApiUrl": "https://cloud.example.test", "relayPublicKey": "worker-key",
            "bindingDigest": binding_digest, "binding": binding,
            "enrollmentReceipt": null, "cloudRelay": null,
        }))
        .unwrap();
        if with_receipt {
            crate::config::write_private_file(&fixture.config.receipt_path, &receipt).unwrap();
        }
        let cloud = FakeCloud::new(fixture.exchange_response());
        let result = prepare_managed_kernel(&fixture.config, &cloud, fixture.now);
        let after_envelope = fs::read(&fixture.config.envelope_path).ok();
        let after_receipt = fs::read(&fixture.config.receipt_path).ok();
        match previous_home {
            Some(value) => std::env::set_var("CHARIOX_HOME", value),
            None => std::env::remove_var("CHARIOX_HOME"),
        }
        fixture.cleanup();
        let error = result.expect_err("legacy worker must not become an ordinary registration");
        assert!(
            error.to_string().contains("managed bootstrap"),
            "a worker-shaped record is not a managed bootstrap document: {error}"
        );
        assert_eq!(after_envelope, Some(envelope));
        assert_eq!(
            after_receipt,
            if with_receipt { Some(receipt) } else { None }
        );
        assert!(cloud.exchange_calls.lock().unwrap().is_empty());
        assert!(cloud.confirm_calls.lock().unwrap().is_empty());
    }
}
