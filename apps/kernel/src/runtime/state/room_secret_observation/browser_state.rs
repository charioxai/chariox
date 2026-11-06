//! MP-08 / MP-09 / MP-10 / MP-11: page text in fixed browser inventory replies.
use super::*;

impl RoomSecretObservations {
    pub(in crate::runtime::state) fn scrub_kernel_browser_state(
        &self,
        room: &str,
        mut result: serde_json::Value,
    ) -> Result<serde_json::Value, DaemonError> {
        // The controller owns keys, enums and identities. These are the only
        // inventory fields populated from a page or its suggested filename.
        let mut paths = Vec::new();
        let mut page_values = Vec::new();
        for (collection, fields) in [
            ("tabs", &["title", "url"][..]),
            ("dialogs", &["message", "default_prompt", "origin"][..]),
            ("downloads", &["name", "path"][..]),
        ] {
            if let Some(items) = result.get(collection).and_then(serde_json::Value::as_array) {
                for (index, item) in items.iter().enumerate() {
                    for field in fields {
                        if collection == "dialogs"
                            && *field == "message"
                            && item["kind"] == "permission"
                        {
                            continue;
                        }
                        if let Some(value) = item.get(*field) {
                            paths.push(format!("/{collection}/{index}/{field}"));
                            page_values.push(value.clone());
                        }
                    }
                }
            }
        }
        let cleaned: Vec<serde_json::Value> = self.scrub(room, page_values)?;
        for (path, value) in paths.into_iter().zip(cleaned) {
            *result.pointer_mut(&path).ok_or_else(protection_error)? = value;
        }
        Ok(result)
    }
}
