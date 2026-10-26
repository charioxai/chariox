//! Validation of the selections that become prompt notes. Nothing is stored here.
pub(crate) mod observation;
mod validation;
pub(crate) use validation::{validate_anchor, validate_box, validate_quote, validate_window};
