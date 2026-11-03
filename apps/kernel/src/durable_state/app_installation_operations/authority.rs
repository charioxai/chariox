//! Recheck the normalized capability decision on the existing kernel writer.
use super::*;
use chariox_app_runtime::installation::activation::{
    approval_requirement, ApprovalRequirement, AuthoritySet,
};
use rusqlite::{params, OptionalExtension, Transaction};

pub(super) fn reuses_in(tx: &Transaction<'_>, token: &StageToken) -> Result<bool> {
    fn read(tx: &Transaction<'_>, id: &str, generation: u64) -> Result<Option<AuthoritySet>> {
        let json: Option<String> = store::sql(tx.query_row("SELECT authority_json FROM app_installation_authority WHERE installation_id=?1 AND generation=?2", params![id,generation as i64], |r| r.get(0)).optional())?;
        if let Some(value) = json {
            return serde_json::from_str(&value)
                .map(Some)
                .map_err(|_| InstallOperationError::Storage);
        }
        // Older generations predate the normalized table. Their committed
        // review is derived from the verified package, with the same protected
        // routes; input schemas never grant authority.
        let review: Option<String> = store::sql(tx.query_row("SELECT review_json FROM app_installation_operations WHERE installation_id=?1 AND generation=?2 AND phase='committed' AND review_json IS NOT NULL LIMIT 1", params![id,generation as i64], |r| r.get(0)).optional())?;
        let Some(review) = review else {
            return Ok(None);
        };
        let review: serde_json::Value =
            serde_json::from_str(&review).map_err(|_| InstallOperationError::Storage)?;
        let capabilities = serde_json::from_value(review["capabilities"].clone())
            .map_err(|_| InstallOperationError::Storage)?;
        let mut actions = review["actions"]
            .as_array()
            .cloned()
            .ok_or(InstallOperationError::Storage)?;
        for action in &mut actions {
            action["inputSchema"] = serde_json::json!({});
        }
        let actions: Vec<chariox_app_package::ActionDeclaration> =
            serde_json::from_value(actions.into()).map_err(|_| InstallOperationError::Storage)?;
        Ok(Some(AuthoritySet::of(&capabilities, &actions)))
    }
    let approved = read(tx, &token.installation_id, token.base_generation)?;
    let candidate = read(tx, &token.installation_id, token.generation)?;
    Ok(candidate.is_some_and(|candidate| {
        approval_requirement(approved.as_ref(), &candidate) == ApprovalRequirement::Reuse
    }))
}
