use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SessionConfigState {
    version: u64,
    values: BTreeMap<String, String>,
    updated_by_attachment_id: Option<String>,
}

impl SessionConfigState {
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn values(&self) -> &BTreeMap<String, String> {
        &self.values
    }

    pub fn updated_by_attachment_id(&self) -> Option<&str> {
        self.updated_by_attachment_id.as_deref()
    }

    pub fn apply_changes(
        &mut self,
        values: BTreeMap<String, String>,
        updated_by_attachment_id: impl Into<String>,
    ) {
        for (key, value) in values {
            self.values.insert(key, value);
        }
        self.version += 1;
        self.updated_by_attachment_id = Some(updated_by_attachment_id.into());
    }
}
