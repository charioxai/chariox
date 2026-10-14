//! Shared AEGS and kernel notification dotted-field equality / any-of matching.
use serde_json::Value;

pub fn metadata_matches_filter(metadata: &Value, filter: &Value) -> bool {
    let Some(filter) = filter.as_object() else {
        return filter.is_null();
    };
    filter.iter().all(|(key, expected)| {
        dotted_value(metadata, key).is_some_and(|actual| filter_value_matches(actual, expected))
    })
}

/// Match either a scalar filter value or an explicit any-of array. Provider
/// filters use arrays for one binding that intentionally selects several
/// resources (for example, several Slack channels), while preserving the
/// existing scalar and metadata-array behavior.
fn filter_value_matches(actual: &Value, expected: &Value) -> bool {
    if let Some(expected_values) = expected.as_array() {
        return expected_values
            .iter()
            .any(|candidate| filter_value_matches(actual, candidate));
    }
    actual == expected || array_contains(actual, expected)
}

fn dotted_value<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .try_fold(value, |current, key| current.get(key))
}

fn array_contains(actual: &Value, expected: &Value) -> bool {
    actual
        .as_array()
        .is_some_and(|values| values.iter().any(|value| value == expected))
}
