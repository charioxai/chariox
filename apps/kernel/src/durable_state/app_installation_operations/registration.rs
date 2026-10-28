//! Local registration preserves the publisher already bound to the App ID.
use super::*;
use chariox_app_package::v2::{registration_outcome, SignerPrincipal};
use chariox_app_runtime::installation::ReleaseMetadata;
use rusqlite::{params, Transaction};

pub(super) fn check_in(
    tx: &Transaction<'_>,
    owner: &str,
    release: &ReleaseMetadata,
    signer: &SignerPrincipal,
    first: bool,
) -> Result<()> {
    let mut statement = store::sql(tx.prepare("SELECT coalesce(json_extract(active_json,'$.release.publisher_id'), json_extract(retained_json,'$.release.publisher_id'),
        (SELECT json_extract(record_json,'$.release.publisher_id') FROM app_installation_updates u WHERE u.installation_id=i.installation_id AND u.generation=i.pending_generation))
        FROM app_installations i WHERE owner_id=?1 AND app_id=?2 AND channel='release' AND (active_json IS NOT NULL OR retained_json IS NOT NULL OR pending_generation IS NOT NULL)"))?;
    let publishers = store::sql(statement.query_map(params![owner, release.app_id], |r| {
        r.get::<_, Option<String>>(0)
    }))?;
    for publisher in publishers {
        let publisher = store::sql(publisher)?.ok_or(InstallOperationError::Storage)?;
        registration_outcome(Some(&publisher), signer)
            .map_err(|_| InstallOperationError::PublisherConflict)?;
        if first {
            return Err(InstallOperationError::Conflict);
        }
    }
    Ok(())
}
