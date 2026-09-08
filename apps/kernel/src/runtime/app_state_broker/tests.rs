use super::*;
use chariox_app_runtime::managed_state::{MAX_CHANGES, MAX_CHECKS, MAX_VALUE_BYTES};

mod integration;

fn transaction() -> Value {
    json!({"schemaVersion":0,"checks":[],"writes":[]})
}
fn rejection(method: &str, params: Value) -> RemoteError {
    decode::operation(method, params).unwrap_err()
}

#[test]
fn sdk_get_and_transaction_fields_are_exact_and_authority_cannot_be_supplied() {
    assert!(
        matches!(decode::operation("state.get",json!({"key":"status"})).unwrap(),AppStateOperation::Get{key} if key=="status")
    );
    assert!(decode::operation("state.transaction", transaction()).is_ok());
    for field in [
        "owner",
        "installation_id",
        "generation",
        "deadline_ms",
        "path",
        "context",
    ] {
        let mut get = json!({"key":"status"});
        get[field] = json!("spoof");
        assert_eq!(rejection("state.get", get).code, "INVALID_ARGUMENT");
        let mut change = transaction();
        change[field] = json!("spoof");
        assert_eq!(
            rejection("state.transaction", change).code,
            "INVALID_ARGUMENT"
        );
    }
    assert_eq!(
        rejection("worker.ready", json!({})).code,
        "METHOD_NOT_FOUND"
    );
    assert_eq!(rejection("state.get", json!({})).code, "INVALID_ARGUMENT");
    for key in [
        json!(null),
        json!(3),
        json!(""),
        json!("has whitespace"),
        json!("\u{feff}key"),
        json!("x".repeat(129)),
    ] {
        assert_eq!(
            rejection("state.get", json!({"key":key})).code,
            "INVALID_ARGUMENT"
        );
    }
}

#[test]
fn version_null_is_required_and_write_union_never_confuses_null_with_delete() {
    let mut good = transaction();
    good["checks"] = json!([{"key":"status","version":null}]);
    good["writes"] = json!([{"key":"status","value":null},{"key":"old","delete":true}]);
    assert!(decode::operation("state.transaction", good).is_ok());
    for check in [
        json!({"key":"status"}),
        json!({"key":"status","version":0}),
        json!({"key":"status","version":false}),
        json!({"key":"status","version":9_007_199_254_740_992_u64}),
        json!({"key":"status","version":1,"extra":true}),
    ] {
        let mut change = transaction();
        change["checks"] = json!([check]);
        assert_eq!(
            rejection("state.transaction", change).code,
            "INVALID_ARGUMENT"
        );
    }
    for write in [
        json!({"key":"status"}),
        json!({"key":"status","delete":false}),
        json!({"key":"status","delete":null}),
        json!({"key":"status","value":null,"delete":true}),
        json!({"key":"status","value":1,"extra":true}),
    ] {
        let mut change = transaction();
        change["writes"] = json!([write]);
        assert_eq!(
            rejection("state.transaction", change).code,
            "INVALID_ARGUMENT"
        );
    }
    for field in ["schemaVersion", "checks", "writes"] {
        let mut missing = transaction();
        missing.as_object_mut().unwrap().remove(field);
        assert_eq!(
            rejection("state.transaction", missing).code,
            "INVALID_ARGUMENT"
        );
    }
}

#[test]
fn occurrence_transactions_fail_explicitly_before_any_state_operation() {
    let mut empty = transaction();
    empty["occurrences"] = json!([]);
    assert!(decode::operation("state.transaction", empty).is_ok());
    let mut nonempty = transaction();
    nonempty["occurrences"] =
        json!([{"automationId":"a","occurrenceId":"o","eventVersion":1,"payload":{}}]);
    assert_eq!(
        rejection("state.transaction", nonempty).code,
        "UNSUPPORTED_OPERATION"
    );
    for invalid in [Value::Null, json!({}), json!("event")] {
        let mut change = transaction();
        change["occurrences"] = invalid;
        assert_eq!(
            rejection("state.transaction", change).code,
            "INVALID_ARGUMENT"
        );
    }
}

#[test]
fn collection_value_and_duplicate_key_bounds_use_managed_state_validation() {
    let mut change = transaction();
    change["checks"] = json!((0..=MAX_CHECKS)
        .map(|n| json!({"key":format!("key-{n}"),"version":null}))
        .collect::<Vec<_>>());
    assert_eq!(
        rejection("state.transaction", change).code,
        "LIMIT_EXCEEDED"
    );
    let mut change = transaction();
    change["writes"] = json!((0..=MAX_CHANGES)
        .map(|n| json!({"key":format!("key-{n}"),"delete":true}))
        .collect::<Vec<_>>());
    assert_eq!(
        rejection("state.transaction", change).code,
        "LIMIT_EXCEEDED"
    );
    let mut change = transaction();
    change["writes"] = json!([{"key":"large","value":"x".repeat(MAX_VALUE_BYTES)}]);
    assert_eq!(
        rejection("state.transaction", change).code,
        "LIMIT_EXCEEDED"
    );
    for field in ["checks", "writes"] {
        let mut change = transaction();
        change[field] = if field == "checks" {
            json!([{"key":"same","version":null},{"key":"same","version":null}])
        } else {
            json!([{"key":"same","delete":true},{"key":"same","value":1}])
        };
        assert_eq!(
            rejection("state.transaction", change).code,
            "INVALID_ARGUMENT"
        );
    }
}

#[test]
fn response_errors_do_not_include_input_values_paths_or_internal_database_details() {
    let secret = "private-secret-/Users/example/credentials";
    let unknown = rejection(secret, json!({"secret":secret}));
    let database = errors::changes(chariox_app_runtime::managed_state::StateError::Database(
        rusqlite::Error::InvalidPath(secret.into()),
    ));
    for error in [
        unknown,
        database,
        errors::stopped(crate::runtime::app_operation_budget::AppOperationStopped::Cancelled),
        errors::stopped(crate::runtime::app_operation_budget::AppOperationStopped::Deadline),
    ] {
        let text = serde_json::to_string(&error).unwrap();
        assert!(!text.contains(secret));
        assert!(error.message.len() < 128);
    }
}
