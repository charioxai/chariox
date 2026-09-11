use super::*;

#[test]
fn general_kernel_role_is_the_default() {
    let config = DaemonConfig::for_tests();
    assert_eq!(config.kernel_runtime_role, KernelRuntimeRole::General);
    config.validate().expect("general role should remain valid");
}

#[test]
fn lease_worker_requires_positive_bounded_capacity() {
    for capacity in [None, Some(0)] {
        let mut config = DaemonConfig::for_tests();
        config.kernel_runtime_role = KernelRuntimeRole::RemoteLeaseWorker;
        config.remote_lease_capacity = capacity;
        let error = config
            .validate()
            .expect_err("lease worker without positive capacity must fail closed");
        assert!(matches!(
            error,
            DaemonError::InvalidConfig {
                field: "remote_lease_capacity",
                ..
            }
        ));
    }

    let mut config = DaemonConfig::for_tests();
    config.kernel_runtime_role = KernelRuntimeRole::RemoteLeaseWorker;
    config.remote_lease_capacity = Some(1);
    config.validate().expect("bounded lease worker should validate");
}

#[test]
fn kernel_runtime_role_parser_is_strict() {
    assert_eq!(
        parse_kernel_runtime_role(None).expect("missing role should default"),
        KernelRuntimeRole::General
    );
    assert_eq!(
        parse_kernel_runtime_role(Some("remote_lease_worker"))
            .expect("lease worker role should parse"),
        KernelRuntimeRole::RemoteLeaseWorker
    );
    for malformed in ["", "worker", "REMOTE_LEASE_WORKER", "remote-lease-worker"] {
        assert!(
            parse_kernel_runtime_role(Some(malformed)).is_err(),
            "malformed role `{malformed}` must fail closed"
        );
    }
}

#[test]
fn remote_lease_capacity_parser_rejects_malformed_or_zero_values() {
    assert_eq!(parse_remote_lease_capacity(None).unwrap(), None);
    assert_eq!(parse_remote_lease_capacity(Some("1")).unwrap(), Some(1));
    for malformed in ["", "0", "-1", "one"] {
        assert!(parse_remote_lease_capacity(Some(malformed)).is_err());
    }
}
