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
        cancel: FutureReader<String>,
    ) -> Result<(), AppError> {
        let work = Box::pin(async move {
            if from == 98 {
                core::arch::wasm32::unreachable();
            }
            if from == 99 {
                loop {
                    std::hint::spin_loop();
                }
            }
            state::get(if from == 3 { "hold" } else { "migration" }.into()).await?;
            let tx = state::Transaction {
                schema_version: to,
                checks: vec![],
                writes: vec![state::Write::Put(("migrated".into(), "true".into()))],
                occurrences: vec![],
                wakes: vec![],
            };
            state::transact(tx).await?;
            Ok(())
        });
        match futures::future::select(
            work,
            std::pin::pin!(std::future::IntoFuture::into_future(cancel)),
        )
        .await
        {
            futures::future::Either::Left((result, _)) => result,
            futures::future::Either::Right((_, _)) => Err(AppError {
                code: "CANCELLED".into(),
                message: "migration cancelled".into(),
                retryable: None,
            }),
        }
    }
}
