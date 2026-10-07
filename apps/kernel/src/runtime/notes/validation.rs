//! MD-N1–N2 / MP-11: bounded quotes and trusted surface identities.
use crate::local::{NoteAnchor, NoteBox, NoteTextQuote, NoteWindow};
fn identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}
pub(crate) fn validate_window(window: &NoteWindow) -> Result<(), String> {
    let valid = match window {
        NoteWindow::KernelBrowser { tab_id, generation } => identity(tab_id) && *generation > 0,
        NoteWindow::RoomBrowser { session_id, tab_id } => identity(session_id) && identity(tab_id),
        NoteWindow::Panel { window_id } => identity(window_id),
        NoteWindow::Terminal {
            session_id,
            window_id,
        } => identity(session_id) && identity(window_id),
    };
    valid
        .then_some(())
        .ok_or("MD-N1: invalid note window".into())
}
pub(crate) fn validate_quote(quote: &NoteTextQuote) -> Result<(), String> {
    (!quote.exact.trim().is_empty()
        && quote.exact.len() <= 16384
        && quote.prefix.len() <= 512
        && quote.suffix.len() <= 512
        && !quote.exact.contains('\0'))
    .then_some(())
    .ok_or("MD-N1: invalid text quote".into())
}
pub(crate) fn validate_anchor(anchor: &NoteAnchor) -> Result<(), String> {
    validate_window(&anchor.window)?;
    validate_quote(&anchor.quote)?;
    if anchor
        .url
        .as_ref()
        .is_some_and(|s| s.len() > 8192 || s.contains('\0'))
        || anchor.document_id.as_ref().is_some_and(|s| !identity(s))
        || anchor.hint.as_ref().is_some_and(|s| s.len() > 2048)
    {
        return Err("MD-N1: invalid anchor metadata".into());
    }
    Ok(())
}
pub(crate) fn validate_box(rect: Option<&NoteBox>) -> Result<(), String> {
    if let Some(r) = rect {
        if ![r.x, r.y, r.width, r.height]
            .into_iter()
            .all(|n| n.is_finite() && n.abs() <= 32768.)
            || r.width <= 0.
            || r.height <= 0.
        {
            return Err("MD-N2: invalid selection box".into());
        }
    }
    Ok(())
}
pub(super) fn validate_comment(comment: &str) -> Result<(), String> {
    (!comment.trim().is_empty() && comment.len() <= 16384 && !comment.contains('\0'))
        .then_some(())
        .ok_or("MD-N1: comment must contain 1–16384 bytes".into())
}

// MD-N1: generations fence new selections; durable tab identity survives restarts.
pub(crate) fn same_window(a: &NoteWindow, b: &NoteWindow) -> bool {
    match (a, b) {
        (
            NoteWindow::KernelBrowser { tab_id: a, .. },
            NoteWindow::KernelBrowser { tab_id: b, .. },
        ) => a == b,
        _ => a == b,
    }
}
