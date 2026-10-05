use serde::{Deserialize, Serialize};

// MP-08/MP-10/MP-11: one bounded mutation, using the shared observed locator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceBrowserInteractArgs {
    pub field_id: String,
    #[serde(flatten)]
    pub interaction: SliceBrowserInteraction,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum SliceBrowserInteraction {
    Press {
        key: String,
        #[serde(default)]
        expected_value: Option<String>,
    },
    Drag {
        delta_x: i32,
        delta_y: i32,
        #[serde(default)]
        expected_value: Option<String>,
    },
}

impl std::fmt::Debug for SliceBrowserInteraction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Press { .. } => "Press",
            Self::Drag { .. } => "Drag",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mp08_interaction_is_typed_and_rejects_unrelated_parameters() {
        for wire in [
            serde_json::json!({"field_id":"element-1","action":"press","key":"ArrowRight","expected_value":"13"}),
            serde_json::json!({"field_id":"element-1","action":"drag","delta_x":20,"delta_y":0}),
        ] {
            assert!(serde_json::from_value::<SliceBrowserInteractArgs>(wire).is_ok());
        }
        for wire in [
            serde_json::json!({"field_id":"element-1","action":"press","key":"ArrowRight","delta_x":1}),
            serde_json::json!({"field_id":"element-1","action":"drag","delta_x":1,"delta_y":0,"key":"Home"}),
        ] {
            assert!(serde_json::from_value::<SliceBrowserInteractArgs>(wire).is_err());
        }
    }
}
