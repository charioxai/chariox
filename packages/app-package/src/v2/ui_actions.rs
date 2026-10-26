use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    json::read_json, manifest::valid_function_name, validate_schema, ErrorCode, Limits,
    PackageError, Result,
};

/// What running an action costs the person: `read` and `write` never prompt;
/// `approval` goes through the kernel's shared interaction unless a grant or
/// `/sudo` already covers it. An effect class grants no capability and does
/// not bypass a backend action's protected effect routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiActionEffect {
    Read,
    Write,
    Approval,
}

/// One action an agent may run through the view's own function. The view
/// registers the same name, schemas and effect at runtime; the kernel refuses
/// a registration that differs from this signed declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiActionDeclaration {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    pub effect: UiActionEffect,
    /// Shown in the approval interaction; required for, and only for, `approval`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UiActionsDocument {
    ui_actions: Vec<UiActionDeclaration>,
}

pub(crate) fn read_ui_actions(bytes: &[u8], limits: &Limits) -> Result<Vec<UiActionDeclaration>> {
    let document: UiActionsDocument = read_json(
        bytes,
        limits.max_metadata_bytes,
        limits,
        ErrorCode::InvalidSchema,
        false,
    )?;
    let actions = document.ui_actions;
    let invalid = |message| Err(PackageError::new(ErrorCode::InvalidSchema, message));
    if actions.len() > limits.max_declarations {
        return invalid("too many UI actions");
    }
    for (index, action) in actions.iter().enumerate() {
        if !valid_function_name(&action.name)
            || actions[..index].iter().any(|seen| seen.name == action.name)
        {
            return invalid("UI action names must be distinct lowercase function names");
        }
        if action.description.trim().is_empty() || action.description.len() > 1024 {
            return invalid("UI actions need a description of at most 1024 bytes");
        }
        let reason_ok = match (&action.effect, &action.approval_reason) {
            (UiActionEffect::Approval, Some(reason)) => {
                !reason.trim().is_empty() && reason.len() <= 1024
            }
            (UiActionEffect::Approval, None) => false,
            (_, reason) => reason.is_none(),
        };
        if !reason_ok {
            return invalid("approvalReason is required for, and only for, approval actions");
        }
        validate_schema(&action.input_schema, true, limits)?;
        if let Some(output) = &action.output_schema {
            validate_schema(output, false, limits)?;
        }
    }
    Ok(actions)
}
