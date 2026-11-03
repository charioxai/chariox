use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    json::{canonical, read_json},
    manifest::valid_function_name,
    validate_schema, ErrorCode, Limits, PackageError, Result,
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

/// Which part of a registration differs from the signed declaration. Listed
/// in the order they are reported: authority-relevant parts first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UiActionField {
    Effect,
    ApprovalReason,
    InputSchema,
    OutputSchema,
    Description,
}

impl fmt::Display for UiActionField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Effect => "effect",
            Self::ApprovalReason => "approval reason",
            Self::InputSchema => "input schema",
            Self::OutputSchema => "output schema",
            Self::Description => "description",
        })
    }
}

/// Why a view's runtime actions are refused against the signed declarations.
/// The kernel is the authority: the view registers, the signed package decides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "mismatch", rename_all = "snake_case")]
pub enum UiActionMismatch {
    /// Registered by the view, absent from the signed declarations.
    Undeclared { action: String },
    /// Declared and signed, but the view never registered it.
    Missing { action: String },
    /// Registered twice (two functions, or a function and a form).
    Duplicate { action: String },
    Differs {
        action: String,
        field: UiActionField,
    },
}

impl fmt::Display for UiActionMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Undeclared { action } => write!(
                f,
                "the view registers action {action:?}, which the signed manifest does not declare; declare it in ui-actions and rebuild"
            ),
            Self::Missing { action } => write!(
                f,
                "the signed manifest declares action {action:?}, but the view never registers it"
            ),
            Self::Duplicate { action } => {
                write!(f, "the view registers action {action:?} more than once")
            }
            Self::Differs { action, field } => write!(
                f,
                "the view's {field} for action {action:?} differs from the signed manifest; rebuild so both come from one declaration"
            ),
        }
    }
}

impl std::error::Error for UiActionMismatch {}

/// Schemas agree when their RFC 8785 forms do, so key order and number
/// spelling (`1` or `1.0`) are not differences.
fn same_json(a: &Value, b: &Value) -> bool {
    matches!((canonical(a), canonical(b)), (Ok(a), Ok(b)) if a == b)
}

/// One registration against the declarations: it must be declared and equal.
/// A2 runs this when the view registers and again before every invocation.
pub fn check_registration(
    declared: &[UiActionDeclaration],
    registered: &UiActionDeclaration,
) -> std::result::Result<(), UiActionMismatch> {
    let action = registered.name.clone();
    let Some(signed) = declared.iter().find(|d| d.name == registered.name) else {
        return Err(UiActionMismatch::Undeclared { action });
    };
    let differs = |field| {
        Err(UiActionMismatch::Differs {
            action: action.clone(),
            field,
        })
    };
    if signed.effect != registered.effect {
        return differs(UiActionField::Effect);
    }
    if signed.approval_reason != registered.approval_reason {
        return differs(UiActionField::ApprovalReason);
    }
    if !same_json(&signed.input_schema, &registered.input_schema) {
        return differs(UiActionField::InputSchema);
    }
    let outputs_agree = match (&signed.output_schema, &registered.output_schema) {
        (None, None) => true,
        (Some(a), Some(b)) => same_json(a, b),
        _ => false,
    };
    if !outputs_agree {
        return differs(UiActionField::OutputSchema);
    }
    if signed.description != registered.description {
        return differs(UiActionField::Description);
    }
    Ok(())
}

/// The whole set the view registered against the signed declarations: each
/// registration agrees, none repeats, and none declared is missing. Order is
/// free. A4 runs this for `/app actions`; the first mismatch is reported.
pub fn check_agreement(
    declared: &[UiActionDeclaration],
    registered: &[UiActionDeclaration],
) -> std::result::Result<(), UiActionMismatch> {
    for (index, action) in registered.iter().enumerate() {
        if registered[..index]
            .iter()
            .any(|seen| seen.name == action.name)
        {
            return Err(UiActionMismatch::Duplicate {
                action: action.name.clone(),
            });
        }
        check_registration(declared, action)?;
    }
    match declared
        .iter()
        .find(|d| registered.iter().all(|r| r.name != d.name))
    {
        Some(missing) => Err(UiActionMismatch::Missing {
            action: missing.name.clone(),
        }),
        None => Ok(()),
    }
}
