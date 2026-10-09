use super::*;

#[test]
fn home_bound_disposable_control_shapes_are_versioned_and_hashed() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 488);
    let selection = serde_json::json!({"allocationId":"worker-1","homeKernelId":"home-1","homeRelayRealmId":"realm-1"});
    let mut shapes = vec![serde_json::json!({"CreateDisposableWorker":{
        "clientRequestId":"request-1","homeKernelId":"home-1","homeRelayRealmId":"realm-1",
        "region":"fsn1","computeClass":"worker","architecture":"x86_64","maximumLifetimeSeconds":14400,
        "autoStopPolicy":{"minimumRuntimeSeconds":12600,"idleDelaySeconds":null}
    }})];
    for variant in [
        "GetDisposableWorker",
        "ReleaseDisposableWorker",
        "KeepDisposableWorkerRunning",
        "PrepareDisposableWorkerContextTransfer",
    ] {
        let mut value = serde_json::Map::new();
        value.insert(variant.to_string(), selection.clone());
        shapes.push(serde_json::Value::Object(value));
    }
    shapes.push(
        serde_json::json!({"KeepManagedEnvironmentRunning":{"environmentId":"environment-1"}}),
    );
    let actual: Vec<serde_json::Value> = shapes
        .iter()
        .map(|value| {
            let request: LocalDaemonRequest = serde_json::from_value(value.clone()).unwrap();
            serde_json::to_value(request).unwrap()
        })
        .collect();
    assert_eq!(actual, shapes);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_string(&actual).unwrap().as_bytes())
        ),
        "e1cc93b0e4b21c3958eee2cca956381780bd6b0b97c7982e76a6e70706091a8d"
    );
    let mut unscoped = selection;
    unscoped.as_object_mut().unwrap().remove("homeRelayRealmId");
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"ReleaseDisposableWorker":unscoped})
    )
    .is_err());
}
