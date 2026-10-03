use super::*;

#[test]
fn process_preparation_identity_must_match_before_any_sdk_broker_is_created() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let catalog = fixture_event_catalog(&store);
    let runtime = runtime();
    let fixture = NativeFixture::compile().unwrap();
    let (expected_bytes, publisher) = fixture_event_package();
    let expected = verify(
        &expected_bytes,
        &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
    )
    .unwrap();
    let (other_bytes, publisher) = crate::durable_state::app_state::fixture_tool_package();
    let other = verify(
        &other_bytes,
        &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
    )
    .unwrap();
    for (mode, prepared_package) in [(Mode::OtherInstallation, &expected), (Mode::Ready, &other)] {
        let (process, observed) = fixture.spawn_blocking(mode, prepared_package).unwrap();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let called = calls.clone();
        let result = AppWorkerOwner::start_blocking(
            process,
            &expected,
            catalog.clone(),
            broker(move |_| {
                called.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Ok(Value::Null) })
            }),
            PeerLimits::default(),
            runtime.handle().clone(),
        );
        assert!(matches!(result, Err(AppWorkerError::Identity)));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(!observed.ready_was_acknowledged());
        assert!(observed.was_reaped());
        assert!(observed.lease_was_dropped());
    }
}
