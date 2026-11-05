//! Signed migration that mutates real structured state before success/failure.
wit_bindgen::generate!({path: "../../../app-wit/wit",world: "chariox:app/migration",generate_all});
use chariox::app::{
    app_types::{AppError, InvocationContext},
    state,
};
use exports::chariox::app::migration_handlers::Guest;
use wit_bindgen::FutureReader;
struct App;
export!(App);
impl Guest for App {
    async fn migrate(
        from: u32,
        to: u32,
        _context: InvocationContext,
        _cancel: FutureReader<String>,
    ) -> Result<(), AppError> {
        let prior = state::get("migrated".into()).await?;
        if from > 0 && prior.as_ref().map(|record| record.value.as_str()) != Some("1") {
            return Err(AppError {
                code: "DRILL_PRIOR_STATE".into(),
                message: "prior migrated state missing".into(),
                retryable: None,
            });
        }
        state::transact(state::Transaction {
            schema_version: to,
            checks: vec![],
            writes: vec![state::Write::Put(("migrated".into(), to.to_string()))],
            occurrences: vec![],
            wakes: vec![],
        })
        .await?;
        if to == 2 {
            return Err(AppError {
                code: "DRILL_MIGRATION_FAILED".into(),
                message: "failed after state mutation".into(),
                retryable: None,
            });
        }
        Ok(())
    }
}
