use jsonschema::JSONSchema;
use serde_json::Value;

pub fn validate_json_output_schema(
    schema_label: &str,
    schema_value: &Value,
    handoff_json: &str,
) -> Result<(), String> {
    let output_value = serde_json::from_str::<Value>(handoff_json)
        .map_err(|error| format!("output is not valid JSON: {error}"))?;
    let compiled = JSONSchema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .compile(schema_value)
        .map_err(|error| format!("schema `{schema_label}` failed to compile: {error}"))?;
    if let Err(errors) = compiled.validate(&output_value) {
        let message = errors
            .into_iter()
            .next()
            .map(|error| error.to_string())
            .unwrap_or_else(|| "schema validation failed".to_string());
        return Err(message);
    }
    Ok(())
}

pub fn validate_workflow_handoff_schema(
    schema_ref: &str,
    handoff_json: &str,
) -> Result<(), String> {
    let schema_source = std::fs::read_to_string(schema_ref)
        .map_err(|error| format!("schema ref `{schema_ref}` could not be read: {error}"))?;
    let schema_value = serde_json::from_str::<Value>(&schema_source)
        .map_err(|error| format!("schema ref `{schema_ref}` is not valid JSON: {error}"))?;
    validate_json_output_schema(schema_ref, &schema_value, handoff_json)
}
