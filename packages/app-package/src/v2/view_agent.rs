//! Apps v2 A2: what the kernel decides, from the signed declarations alone,
//! before an agent touches an open App view. The page registers and answers;
//! these checks are the kernel's, so a page cannot widen what it declared.

use std::fmt;

use serde_json::Value;

use super::{check_registration, UiActionDeclaration, UiActionEffect, UiActionMismatch};
use crate::{compile_schema, Limits};

/// A refusal with a stable code the agent can act on and a message that says
/// what to do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewActionError {
    pub code: &'static str,
    pub message: String,
}

impl fmt::Display for ViewActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ViewActionError {}

fn refuse(code: &'static str, message: impl Into<String>) -> ViewActionError {
    ViewActionError {
        code,
        message: message.into(),
    }
}

/// The signed declaration an agent names.
pub fn find_declared<'a>(
    declared: &'a [UiActionDeclaration],
    name: &str,
) -> Result<&'a UiActionDeclaration, ViewActionError> {
    declared.iter().find(|action| action.name == name).ok_or_else(|| {
        let names: Vec<_> = declared.iter().map(|action| action.name.as_str()).collect();
        refuse(
            "unknown_action",
            format!(
                "the App declares no action {name:?}; its actions are: {}",
                if names.is_empty() { "none".into() } else { names.join(", ") }
            ),
        )
    })
}

/// The agent's input against the action's signed input schema.
pub fn check_input(action: &UiActionDeclaration, input: &Value) -> Result<(), ViewActionError> {
    let schema = compile_schema(&action.input_schema, true, &Limits::default())
        .map_err(|error| refuse("invalid_declaration", error.to_string()))?;
    if let Err(mut errors) = schema.validate(input) {
        let first = errors.next().map_or_else(String::new, |error| error.to_string());
        return Err(refuse(
            "invalid_input",
            format!("the input does not fit action {:?}: {first}", action.name),
        ));
    }
    Ok(())
}

/// The page's registration of `name`, as listed by `registrations()`, must be
/// the signed declaration. Run before every invocation: the page may have
/// reloaded onto another generation since the last one.
pub fn check_registered(
    declared: &[UiActionDeclaration],
    registrations: &Value,
    name: &str,
) -> Result<(), ViewActionError> {
    let listed: Vec<UiActionDeclaration> = serde_json::from_value(registrations.clone())
        .map_err(|error| refuse("view_mismatch", format!("the view lists its actions in an unknown shape: {error}")))?;
    let mut named = listed.iter().filter(|action| action.name == name);
    let Some(registered) = named.next() else {
        return Err(refuse(
            "action_not_registered",
            format!("the view has not registered action {name:?}; wait for it to finish loading or reopen it"),
        ));
    };
    if named.next().is_some() {
        return Err(refuse("view_mismatch", UiActionMismatch::Duplicate { action: name.into() }.to_string()));
    }
    check_registration(declared, registered).map_err(|mismatch| refuse("view_mismatch", mismatch.to_string()))
}

/// How far a batch may run before the kernel must stop it.
#[derive(Debug, PartialEq)]
pub struct BatchPlan<'a> {
    /// Steps that run now, in order.
    pub run: usize,
    /// The step at index `run`, when it needs an approval: a batch never
    /// decides one; the agent runs that step on its own.
    pub approval: Option<&'a UiActionDeclaration>,
}

/// Every step is checked against the declarations before the first one runs,
/// so a typo in step five costs no side effect in steps one to four. The error
/// carries the failing step's index.
pub fn plan_batch<'a>(
    declared: &'a [UiActionDeclaration],
    steps: &[(&str, &Value)],
) -> Result<BatchPlan<'a>, (usize, ViewActionError)> {
    let mut run = steps.len();
    let mut approval = None;
    for (index, (name, input)) in steps.iter().enumerate() {
        let action = find_declared(declared, name).map_err(|error| (index, error))?;
        check_input(action, input).map_err(|error| (index, error))?;
        if action.effect == UiActionEffect::Approval && approval.is_none() {
            run = index;
            approval = Some(action);
        }
    }
    Ok(BatchPlan { run, approval })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn action(name: &str, effect: UiActionEffect) -> UiActionDeclaration {
        UiActionDeclaration {
            name: name.into(),
            description: format!("{name} the list"),
            input_schema: json!({"type": "object", "additionalProperties": false,
                "properties": {"text": {"type": "string"}}, "required": ["text"]}),
            output_schema: None,
            effect,
            approval_reason: (effect == UiActionEffect::Approval).then(|| "Deletes the list".into()),
        }
    }

    fn declared() -> Vec<UiActionDeclaration> {
        vec![
            action("add_todo", UiActionEffect::Write),
            action("wipe", UiActionEffect::Approval),
            action("rename", UiActionEffect::Write),
        ]
    }

    #[test]
    fn an_action_the_app_did_not_declare_names_the_ones_it_did() {
        let declared = declared();
        assert_eq!(find_declared(&declared, "wipe").unwrap().name, "wipe");
        let error = find_declared(&declared, "drop_table").unwrap_err();
        assert_eq!(error.code, "unknown_action");
        assert!(error.message.contains("add_todo, wipe, rename"), "{}", error.message);
        assert!(find_declared(&[], "x").unwrap_err().message.contains("none"));
    }

    #[test]
    fn input_must_fit_the_signed_schema() {
        let add = action("add_todo", UiActionEffect::Write);
        check_input(&add, &json!({"text": "milk"})).unwrap();
        for bad in [json!({}), json!({"text": 5}), json!({"text": "x", "extra": true}), json!([])] {
            let error = check_input(&add, &bad).unwrap_err();
            assert_eq!(error.code, "invalid_input", "{bad}");
            assert!(error.message.contains("add_todo"), "{}", error.message);
        }
    }

    #[test]
    fn the_page_registration_must_be_the_signed_declaration() {
        let declared = declared();
        let registrations = serde_json::to_value(&declared).unwrap();
        check_registered(&declared, &registrations, "add_todo").unwrap();

        let mut widened = declared.clone();
        widened[0].effect = UiActionEffect::Read;
        let error = check_registered(&declared, &serde_json::to_value(&widened).unwrap(), "add_todo").unwrap_err();
        assert_eq!(error.code, "view_mismatch");
        assert!(error.message.contains("effect"), "{}", error.message);

        let mut extra = declared.clone();
        extra.push(action("sneak", UiActionEffect::Write));
        assert_eq!(
            check_registered(&declared, &serde_json::to_value(&extra).unwrap(), "sneak").unwrap_err().code,
            "view_mismatch"
        );
        assert_eq!(check_registered(&declared, &json!([]), "add_todo").unwrap_err().code, "action_not_registered");
        let mut twice = declared.clone();
        twice.push(twice[0].clone());
        assert_eq!(
            check_registered(&declared, &serde_json::to_value(&twice).unwrap(), "add_todo").unwrap_err().code,
            "view_mismatch"
        );
        assert_eq!(check_registered(&declared, &json!({"not": "a list"}), "add_todo").unwrap_err().code, "view_mismatch");
    }

    #[test]
    fn a_batch_runs_up_to_the_first_approval_and_checks_every_step_first() {
        let declared = declared();
        let (milk, eggs, none) = (json!({"text": "milk"}), json!({"text": "eggs"}), json!({}));
        let plan = plan_batch(&declared, &[("add_todo", &milk), ("rename", &eggs)]).unwrap();
        assert_eq!(plan, BatchPlan { run: 2, approval: None });

        let plan = plan_batch(&declared, &[("add_todo", &milk), ("wipe", &eggs), ("rename", &eggs)]).unwrap();
        assert_eq!(plan.run, 1);
        assert_eq!(plan.approval.unwrap().name, "wipe");

        // A bad step after the approval still refuses the whole batch: nothing ran.
        let (at, error) = plan_batch(&declared, &[("add_todo", &milk), ("wipe", &eggs), ("rename", &none)]).unwrap_err();
        assert_eq!((at, error.code), (2, "invalid_input"));
        let (at, error) = plan_batch(&declared, &[("add_todo", &milk), ("nope", &milk)]).unwrap_err();
        assert_eq!((at, error.code), (1, "unknown_action"));
        assert_eq!(plan_batch(&declared, &[]).unwrap(), BatchPlan { run: 0, approval: None });
    }
}
