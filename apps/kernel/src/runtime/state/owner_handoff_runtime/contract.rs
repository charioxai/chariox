//! MP-08 / MP-10 / MP-11 A07: safe contract validation and projection.
use super::*;
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HandoffArgs {
    pub(super) kind: HandoffKind,
    pub(super) reason: HandoffReason,
    pub(super) explanation: String,
    pub(super) tab_id: String,
    pub(super) generation: u64,
    pub(super) document_id: String,
    pub(super) node_ref: String,
    #[serde(default)]
    pub(super) change: Vec<HandoffChangeLine>,
    #[serde(default)]
    pub(super) timeout_sec: Option<u64>,
    #[serde(default)]
    pub(super) offer_vault_save: bool,
}

pub(super) fn handoff_spec() -> RuntimeToolSpec {
    RuntimeToolSpec {
        name: HANDOFF.into(),
        description: "MP-08 A07: ask the owner to perform ONE protected browser step you must not or cannot do yourself: click an observed element (kind=click), type a verification code (kind=code) or type a missing secret (kind=secret) into an observed field. Use it after a model refusal, on a site that disallows automation, for a human verification, or when the owner must authorize the step. Never ask for codes or secrets in chat and never retry a refused step yourself. Use tab_id/generation/document_id/node_ref from a fresh snapshot. List the intended before/after change (op keep/add/remove) so the owner can verify it. The result returns handoff_id and registration_id: call chariox.events.yield with that registration and a deadline after expires_at_ms, then end the turn. You wake with only completed/failed/cancelled/expired/uncertain, never the typed value; then re-read the page and check the actual result against the intended change, reporting any missing or extra difference.".into(),
        input_schema: json!({"type":"object","properties":{
            "kind":{"type":"string","enum":["click","code","secret"]},
            "reason":{"type":"string","enum":["model_refusal","automation_disallowed","human_verification","owner_authorization"]},
            "explanation":{"type":"string","minLength":1,"maxLength":crate::session::HANDOFF_EXPLANATION_MAX_CHARS},
            "tab_id":{"type":"string"},"generation":{"type":"integer","minimum":1},
            "document_id":{"type":"string","minLength":1,"maxLength":256},
            "node_ref":{"type":"string","minLength":1,"maxLength":256},
            "change":{"type":"array","maxItems":crate::session::HANDOFF_CHANGE_MAX_LINES,"items":{"type":"object","properties":{"op":{"type":"string","enum":["keep","add","remove"]},"text":{"type":"string","maxLength":crate::session::HANDOFF_CHANGE_LINE_MAX_CHARS}},"required":["op","text"],"additionalProperties":false}},
            "timeout_sec":{"type":"integer","minimum":crate::session::HANDOFF_MIN_TIMEOUT_SEC,"maximum":crate::session::HANDOFF_MAX_TIMEOUT_SEC},
            "offer_vault_save":{"type":"boolean","default":false}},
            "required":["kind","reason","explanation","tab_id","generation","document_id","node_ref"],
            "additionalProperties":false}),
    }
}

pub(super) fn handoff_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "owner hand-off",
        message: message.into(),
    }
}

/// The kind-specific target check, on the snapshot node.
pub(super) fn node_accepts(
    kind: HandoffKind,
    node: &crate::runtime::browser_controller_snapshot::BrowserControllerDomNode,
) -> bool {
    let attr = |name: &str| node.attributes.get(name).map(|v| v.to_ascii_lowercase());
    if node.attributes.contains_key("disabled") {
        return false;
    }
    let editable = !node.attributes.contains_key("readonly");
    match kind {
        HandoffKind::Click => node.node_type == 1,
        HandoffKind::Secret => {
            node.node_name.eq_ignore_ascii_case("input")
                && attr("type").as_deref() == Some("password")
                && editable
        }
        HandoffKind::Code => {
            editable
                && (node.node_name.eq_ignore_ascii_case("textarea")
                    || (node.node_name.eq_ignore_ascii_case("input")
                        && matches!(
                            attr("type").as_deref(),
                            None | Some("text" | "tel" | "number" | "password" | "search")
                        )))
        }
    }
}

/// A safe label: the accessible name, else a naming attribute, else text.
/// Field values are never read.
pub(super) fn node_label(snapshot: &BrowserControllerStructuredSnapshot, node_ref: &str) -> String {
    let named = snapshot
        .accessibility_nodes
        .iter()
        .find(|n| n.node_ref == node_ref)
        .map(|n| n.name.clone())
        .filter(|name| !name.trim().is_empty());
    let node = snapshot.dom_nodes.iter().find(|n| n.node_ref == node_ref);
    let attribute = node.and_then(|n| {
        ["aria-label", "placeholder", "title", "name", "id"]
            .iter()
            .find_map(|key| {
                n.attributes
                    .get(*key)
                    .filter(|v| !v.trim().is_empty())
                    .cloned()
            })
    });
    let text = node
        .filter(|n| !n.node_name.eq_ignore_ascii_case("input"))
        .map(|n| n.text.clone());
    let label = named.or(attribute).or(text).unwrap_or_default();
    let label = bounded_display_text(&label, crate::session::HANDOFF_LABEL_MAX_CHARS, false);
    if label.is_empty() {
        node.map(|n| n.node_name.to_ascii_lowercase())
            .unwrap_or_default()
    } else {
        label
    }
}

pub(super) fn origin_and_path(url: &str) -> Result<(String, String), DaemonError> {
    let url = url::Url::parse(url).map_err(|_| handoff_error("target document URL is invalid"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(handoff_error("hand-off targets must be http(s) pages"));
    }
    Ok((url.origin().ascii_serialization(), url.path().to_owned()))
}

pub(super) fn change_text(change: &[HandoffChangeLine]) -> String {
    change
        .iter()
        .map(|line| {
            let mark = match line.op {
                crate::session::HandoffChangeOp::Keep => ' ',
                crate::session::HandoffChangeOp::Add => '+',
                crate::session::HandoffChangeOp::Remove => '-',
            };
            format!("{mark} {}", line.text)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn outcome(
    handoff_id: &str,
    status: HandoffStatus,
    action: &str,
    reason: Option<&str>,
) -> HandoffOutcome {
    HandoffOutcome {
        handoff_id: handoff_id.to_owned(),
        status,
        action: action.to_owned(),
        reason_code: reason.map(str::to_owned),
        saved_to_vault: false,
    }
}

/// Private fingerprint: projected origin/path never carry query or fragment.
pub(super) fn document_url_binding(url: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(url.as_bytes()))
}
