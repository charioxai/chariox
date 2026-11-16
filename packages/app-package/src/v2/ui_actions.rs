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
/// `/sudo` already covers it. `person_confirmed` always asks the person for
/// this invocation. An effect class grants no capability and does not bypass
/// a backend action's protected effect routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiActionEffect {
    Read,
    Write,
    Approval,
    /// Every invocation needs the person's decision, including under sudo.
    PersonConfirmed,
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
    /// What shows the person the exact input, required only for `person_confirmed`: the App's
    /// registered compact view (a page), or, for an App whose backend serves its view, the
    /// backend tool that answers with the preview rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation_view: Option<String>,
    /// What the owner may stand-grant for this action, so a repository they trust skips its
    /// card. Only an action that asks (`approval`, `person_confirmed`) can carry it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standing: Option<StandingDeclaration>,
}

/// The classes of action an owner may stand-grant. Closed on purpose: force pushes, deleting
/// a default branch and every grant itself are not in it, so no standing permission names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StandingClass {
    Merge,
    Close,
    DeleteBranch,
    Review,
}

impl StandingClass {
    pub fn word(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Close => "close",
            Self::DeleteBranch => "delete_branch",
            Self::Review => "review",
        }
    }

    pub fn from_word(word: &str) -> Option<Self> {
        [Self::Merge, Self::Close, Self::DeleteBranch, Self::Review]
            .into_iter()
            .find(|class| class.word() == word)
    }
}

/// Which standing permission covers an action: its class, the input property that names the
/// repository (`owner/name`) and the one that may name the account the call runs as. The kernel
/// reads both from the validated input, never from anything the App says afterwards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StandingDeclaration {
    pub class: StandingClass,
    pub repo: String,
    pub account: String,
}

/// What a stand-grantable call is about: its class and repository, and the account it names
/// (`None` when the call leaves the account to the App's own choice).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandingUse {
    pub class: StandingClass,
    pub repo: String,
    pub account: Option<String>,
}

impl UiActionDeclaration {
    /// The class, repository and account this call is about, when the action is stand-grantable.
    pub fn standing_use(&self, input: &Value) -> Option<StandingUse> {
        let standing = self.standing.as_ref()?;
        let repo = input.get(&standing.repo)?.as_str()?;
        Some(StandingUse {
            class: standing.class,
            repo: repo.to_ascii_lowercase(),
            account: input
                .get(&standing.account)
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
    }
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
        let confirmation_ok = match (&action.effect, &action.confirmation_view) {
            (UiActionEffect::PersonConfirmed, Some(view)) => valid_function_name(view),
            (UiActionEffect::PersonConfirmed, None) => false,
            (_, view) => view.is_none(),
        };
        if !confirmation_ok {
            return invalid(
                "confirmationView must name a view for, and only for, person-confirmed actions",
            );
        }
        validate_schema(&action.input_schema, true, limits)?;
        if let Some(standing) = &action.standing {
            let names_a_text_property = action.input_schema["properties"][&standing.repo]["type"]
                == "string"
                && action.input_schema["required"]
                    .as_array()
                    .is_some_and(|required| required.iter().any(|name| *name == standing.repo));
            let names_a_text_account =
                action.input_schema["properties"][&standing.account]["type"] == "string";
            if !matches!(
                action.effect,
                UiActionEffect::Approval | UiActionEffect::PersonConfirmed
            ) || !names_a_text_property
                || !names_a_text_account
            {
                return invalid(
                    "standing is for approval or person-confirmed actions whose required text input property names the repository and whose text input property names the account",
                );
            }
        }
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
    ConfirmationView,
    Standing,
    InputSchema,
    OutputSchema,
    Description,
}

impl fmt::Display for UiActionField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Effect => "effect",
            Self::ApprovalReason => "approval reason",
            Self::ConfirmationView => "confirmation view",
            Self::Standing => "standing permission",
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
    if signed.confirmation_view != registered.confirmation_view {
        return differs(UiActionField::ConfirmationView);
    }
    if signed.standing != registered.standing {
        return differs(UiActionField::Standing);
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
