//! The index in kernel-managed state. Every change commits against the version it read and adds
//! one change to the view's feed in the same transaction, so a window that follows the feed never
//! sees a change that did not commit.

use crate::{
    model::{self, Doc, Plan},
    store,
};
use chariox_app_sdk::{app_types::AppError, error, state, view};
use serde_json::Value;

const KEY: &str = "index";
/// The package's data schema version: 0 until it declares migrations.
const SCHEMA: u32 = 0;
const ATTEMPTS: u32 = 5;

pub async fn load() -> Result<(Vec<Doc>, Option<u64>), AppError> {
    Ok(match state::get(KEY.into()).await? {
        Some(record) => (model::parse_index(&record.value)?, Some(record.version)),
        None => (Vec::new(), None),
    })
}

/// Applies `rule` to the index, writing its content file first and committing the index with
/// the version it read. A save that lost the race to another writer reads again and retries.
pub async fn change<R>(mut rule: R) -> Result<Value, AppError>
where
    R: FnMut(&[Doc]) -> Result<Plan, AppError>,
{
    for _ in 0..ATTEMPTS {
        let (docs, version) = load().await?;
        let plan = rule(&docs)?;
        if let Some(file) = &plan.write {
            store::write(file).await?;
        }
        let commit = view::transact_with_change(
            state::Transaction {
                schema_version: SCHEMA,
                checks: vec![state::Check {
                    key: KEY.into(),
                    version,
                }],
                writes: vec![state::Write::Put((
                    KEY.into(),
                    model::index_text(&plan.docs),
                ))],
                occurrences: vec![],
                wakes: vec![],
            },
            plan.delta,
        )
        .await;
        match commit {
            Ok(_) => {
                store::prune(&plan.prune);
                return Ok(plan.summary);
            }
            Err(failure) => {
                // Nothing names the file, and its name is its own: no one else reads it.
                if let Some(file) = &plan.write {
                    store::discard(file);
                }
                if failure.code != "CONFLICT" {
                    return Err(failure);
                }
            }
        }
    }
    Err(error("CONFLICT", "Documents changed too often; try again"))
}
