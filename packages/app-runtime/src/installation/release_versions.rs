//! Preserve every known release version when upgrading installations that predate the ledger.
use super::{ReleaseMetadata, Result};
use rusqlite::{params, Connection, TransactionBehavior};

pub(super) fn initialize(connection: &mut Connection) -> Result<()> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_release_versions (
            owner_id TEXT NOT NULL, publisher_id TEXT NOT NULL, app_id TEXT NOT NULL,
            version TEXT NOT NULL, package_digest TEXT NOT NULL,
            PRIMARY KEY(owner_id,publisher_id,app_id,version)
        );",
    )?;
    {
        // Journals are bounded; active and kept-data releases also survive pruning.
        // Seed on every initialization so a database opened by the earlier D1
        // implementation (which created an empty ledger) is repaired as well.
        // Before immutability, a version could have been reused. Pin current
        // bytes first, then kept data, then the newest available committed bytes;
        // history fills missing versions without replacing an existing binding.
        let mut statement = tx.prepare(
            "SELECT owner_id, json_extract(active_json, '$.release'), 0 AS priority, generation, installation_id
             FROM app_installations WHERE channel='release' AND active_json IS NOT NULL
             UNION ALL
             SELECT owner_id, json_extract(retained_json, '$.release'), 1, generation, installation_id
             FROM app_installations WHERE channel='release' AND retained_json IS NOT NULL
             UNION ALL
             SELECT i.owner_id, json_extract(u.record_json, '$.release'), 2, u.generation, i.installation_id
             FROM app_installation_updates u JOIN app_installations i
               ON i.installation_id=u.installation_id
             WHERE i.channel='release' AND u.phase='committed'
             ORDER BY priority, generation DESC, installation_id",
        )?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let owner: String = row.get(0)?;
            let release: ReleaseMetadata = serde_json::from_str(&row.get::<_, String>(1)?)?;
            if release.publisher_id == chariox_app_package::v2::DEV_PUBLISHER_ID {
                continue;
            }
            tx.execute(
                "INSERT INTO app_release_versions(owner_id,publisher_id,app_id,version,package_digest)
                 VALUES(?1,?2,?3,?4,?5) ON CONFLICT DO NOTHING",
                params![owner, release.publisher_id, release.app_id, release.version, release.package_digest],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}
