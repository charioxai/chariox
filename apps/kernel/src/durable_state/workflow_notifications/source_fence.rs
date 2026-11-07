//! The source kernel decides a peer's attach or detach by the generation it carries, never by
//! arrival order. A request that was on the wire when the destination's state moved on lands
//! after the newer one, finds a higher generation recorded, and is ignored. The record is kept
//! after a detach too, so an older attach arriving afterwards cannot bring the follow back.
use super::*;

/// Records `generation` for the subscription and answers `None`, unless the source has seen a
/// newer one: then nothing is recorded and the newest one seen is answered. The same generation
/// again is the same state asked twice and is applied again.
pub(super) fn refuse_in(
    tx: &Transaction<'_>,
    owner: &str,
    subscription_id: &str,
    generation: u64,
) -> Result<Option<u64>, DaemonError> {
    let seen: Option<i64> = tx.query_row("SELECT generation FROM workflow_notification_source_fence WHERE owner_id=?1 AND subscription_id=?2",params![owner,subscription_id],|r|r.get(0)).optional().map_err(sql)?;
    let incoming = i64::try_from(generation).map_err(|_| error("notification generation out of range"))?;
    if let Some(seen) = seen.filter(|seen| *seen > incoming) {
        return Ok(Some(seen as u64));
    }
    tx.execute("INSERT INTO workflow_notification_source_fence VALUES (?1,?2,?3) ON CONFLICT(owner_id,subscription_id) DO UPDATE SET generation=excluded.generation",params![owner,subscription_id,incoming]).map_err(sql)?;
    Ok(None)
}
