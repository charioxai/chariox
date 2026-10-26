//! A read-only projection shared by the agent catalog and the DEV inspector.
//! Registrations are observations, never signed authority or callable grants.
use chariox_app_package::v2::{check_registration, UiActionDeclaration};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewControl {
    pub control: String,
    pub action: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewObservation {
    pub registrations: Vec<Value>,
    pub state: Value,
    pub controls: Vec<ViewControl>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewWarning {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewCatalog {
    pub actions: Vec<UiActionDeclaration>,
    pub registrations: Vec<Value>,
    pub state: Value,
    pub warnings: Vec<ViewWarning>,
}

impl ViewCatalog {
    /// `declared` must come from the currently verified signed release.
    pub fn from_observation(
        declared: &[UiActionDeclaration],
        observation: ViewObservation,
    ) -> Result<Self, &'static str> {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "registrations": observation.registrations,
            "state": observation.state,
            "controls": observation.controls,
        }))
        .map_err(|_| "The App view inspection is not JSON")?;
        if bytes.len() > 256 * 1024
            || observation.registrations.len() > 256
            || observation.controls.len() > 512
        {
            return Err("The App view inspection exceeds its bounds");
        }
        let state = &observation.state;
        match state.get("ok").and_then(Value::as_bool) {
            Some(true) if state.get("value").is_some() => {
                if serde_json::to_vec(&state["value"])
                    .map_err(|_| "Invalid App state")?
                    .len()
                    > 64 * 1024
                {
                    return Err("The App state exceeds 64 KiB");
                }
            }
            Some(false)
                if state.get("code").and_then(Value::as_str).is_some()
                    && state.get("message").and_then(Value::as_str).is_some() => {}
            _ => return Err("The App view returned invalid state; reload the view"),
        }
        if observation.controls.iter().any(|control| {
            control.control.len() > 1024
                || control
                    .action
                    .as_ref()
                    .is_some_and(|action| action.len() > 64)
        }) {
            return Err("The App view returned invalid control coverage");
        }
        let mut warnings = Vec::new();
        let mut names = BTreeSet::new();
        let mut matched = BTreeSet::new();
        for value in &observation.registrations {
            match serde_json::from_value::<UiActionDeclaration>(value.clone()) {
                Ok(action) => {
                    let result = check_registration(declared, &action);
                    if !names.insert(action.name.clone()) {
                        warnings.push(ViewWarning {
                            code: "declaration_mismatch".into(),
                            message: format!(
                                "Action '{}' is registered more than once",
                                action.name
                            ),
                        });
                        matched.remove(&action.name);
                    } else if let Err(error) = result {
                        warnings.push(ViewWarning {
                            code: "declaration_mismatch".into(),
                            message: error.to_string(),
                        });
                    } else {
                        matched.insert(action.name);
                    }
                }
                Err(_) => warnings.push(ViewWarning {
                    code: "declaration_mismatch".into(),
                    message: "A view registration is not a valid signed action declaration".into(),
                }),
            }
        }
        for action in declared {
            if !names.contains(&action.name) {
                warnings.push(ViewWarning {
                    code: "declaration_mismatch".into(),
                    message: format!(
                        "Signed action '{}' is not registered by this view",
                        action.name
                    ),
                });
            }
        }
        for control in &observation.controls {
            if !control
                .action
                .as_ref()
                .is_some_and(|action| matched.contains(action))
            {
                warnings.push(ViewWarning {
                    code: "undeclared_control".into(),
                    message: format!(
                        "Visible control '{}' has no matching declared action{}",
                        control.control,
                        control
                            .action
                            .as_ref()
                            .map(|name| format!(" ('{name}')"))
                            .unwrap_or_default()
                    ),
                });
            }
        }
        Ok(Self {
            actions: declared.to_vec(),
            registrations: observation.registrations,
            state: observation.state,
            warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn action() -> UiActionDeclaration {
        serde_json::from_value(json!({"name":"add", "description":"Add", "inputSchema":{"type":"object"}, "effect":"write"})).unwrap()
    }
    fn observation() -> ViewObservation {
        ViewObservation {
            registrations: vec![serde_json::to_value(action()).unwrap()],
            state: json!({"ok":true,"value":{"count":1}}),
            controls: vec![ViewControl {
                control: "button#add".into(),
                action: Some("add".into()),
            }],
        }
    }
    #[test]
    fn view_catalog_warns_on_undeclared_controls_and_manifest_disagreement() {
        let mut observed = observation();
        observed.controls.push(ViewControl {
            control: "button#missing".into(),
            action: None,
        });
        let catalog = ViewCatalog::from_observation(&[action()], observed).unwrap();
        assert_eq!(catalog.warnings.len(), 1);
        assert_eq!(catalog.warnings[0].code, "undeclared_control");
        assert!(catalog.warnings[0].message.contains("button#missing"));
        let mut observed = observation();
        observed.registrations[0]["effect"] = json!("read");
        let catalog = ViewCatalog::from_observation(&[action()], observed).unwrap();
        assert_eq!(catalog.warnings.len(), 2);
        assert_eq!(catalog.warnings[0].code, "declaration_mismatch");
        assert!(catalog.warnings[0].message.contains("effect"));
    }
    #[test]
    fn view_catalog_preserves_catalog_bytes_state_failures_and_reports_missing_duplicate_actions() {
        let catalog = ViewCatalog::from_observation(&[action()], observation()).unwrap();
        assert!(catalog.warnings.is_empty());
        let bytes = serde_json::to_vec(&catalog).unwrap();
        let decoded: ViewCatalog = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(serde_json::to_vec(&decoded).unwrap(), bytes);
        let mut observed = observation();
        observed.registrations.clear();
        observed.state =
            json!({"ok":false,"code":"state_unavailable","message":"No state provider"});
        let catalog = ViewCatalog::from_observation(&[action()], observed).unwrap();
        assert_eq!(catalog.state["code"], "state_unavailable");
        assert_eq!(catalog.warnings.len(), 2);
        let mut observed = observation();
        observed
            .registrations
            .push(observed.registrations[0].clone());
        assert_eq!(
            ViewCatalog::from_observation(&[action()], observed)
                .unwrap()
                .warnings
                .len(),
            2
        );
        let catalog = ViewCatalog::from_observation(&[], observation()).unwrap();
        assert!(catalog.actions.is_empty());
        assert_eq!(catalog.registrations.len(), 1);
        assert_eq!(
            catalog.warnings.len(),
            2,
            "v1 has no signed UI actions; registrations cannot acquire authority"
        );
    }
    #[test]
    fn view_catalog_rejects_unbounded_or_invalid_observations() {
        let mut observed = observation();
        observed.state = json!({"ok":true,"value":"x".repeat(65536)});
        assert!(ViewCatalog::from_observation(&[], observed).is_err());
        let mut observed = observation();
        observed.controls.resize(513, observed.controls[0].clone());
        assert!(ViewCatalog::from_observation(&[], observed).is_err());
        let mut observed = observation();
        observed.state = json!({"ok":true});
        assert!(ViewCatalog::from_observation(&[], observed).is_err());
    }
}
