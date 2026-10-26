//! MP-08/MP-10/MP-11: public command decode reasons, without caller values.

pub(super) fn public_reason(error: &serde_json::Error) -> String {
    // These names belong to the shipped Browser contract. Unknown names and
    // serde's value descriptions may contain private text and are never sent.
    const FIELDS: &[&str] = &[
        "command",
        "document_id",
        "op",
        "url",
        "tab_id",
        "generation",
        "input",
        "kind",
        "x",
        "y",
        "text",
        "key",
        "modifiers",
        "delta_x",
        "delta_y",
        "subscription_id",
        "after",
        "after_sequence",
        "wait_ms",
        "device_scale_factor",
    ];
    let message = error.to_string();
    for field in FIELDS {
        let reason = if message == format!("missing field `{field}`") {
            Some("missing required field")
        } else if message.starts_with(&format!("unknown field `{field}`,")) {
            Some("unexpected field")
        } else {
            None
        };
        if let Some(reason) = reason {
            return format!("MD-3: invalid browser command ({reason} {field})");
        }
    }
    let reason = if message.starts_with("missing field ") {
        "missing required parameter"
    } else if message.starts_with("unknown field ") {
        "unexpected parameter"
    } else if message.starts_with("unknown variant ") {
        "unsupported operation or input kind"
    } else if message.starts_with("invalid type:") {
        "invalid parameter type"
    } else if message.starts_with("invalid value:") {
        "invalid parameter value"
    } else {
        "invalid command structure"
    };
    format!("MD-3: invalid browser command ({reason})")
}
