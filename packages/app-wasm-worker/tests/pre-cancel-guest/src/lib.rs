//! A real caller component writes the cancellation future before entering the
//! frozen SDK component. Non-numeric intra-component futures are unsupported
//! by the pinned engine, so this uses the actual cross-component ABI.
wit_bindgen::generate!({
    path:"../../../app-wit/wit",
    inline:r#"
        package chariox:pre-cancel;
        world proxy {
            import chariox:app/handlers@1.0.0;
            export chariox:app/handlers@1.0.0;
        }
    "#,
    world:"proxy",generate_all,
});
use chariox::app::{
    app_types::{AppError, InvocationContext, Wake},
    handlers,
};
use exports::chariox::app::handlers::Guest;
use wit_bindgen::FutureReader;
struct App;
export!(App);
impl Guest for App {
    fn register() -> Result<(), AppError> {
        handlers::register()
    }
    async fn call_tool(
        name: String,
        input: String,
        context: InvocationContext,
        cancel: FutureReader<String>,
    ) -> Result<String, AppError> {
        drop(cancel);
        let (mut writer, reader) = wit_future::new(|| "CANCELLED".to_owned());
        let write = writer.write("CANCELLED".to_owned());
        let call = handlers::call_tool(name, input, context, reader);
        let (_, result) = futures::join!(write, call);
        result
    }
    async fn deliver_event(
        _: String,
        _: String,
        _: String,
        _: InvocationContext,
        _: FutureReader<String>,
    ) -> Result<String, AppError> {
        unreachable!()
    }
    async fn lifecycle(
        _: String,
        _: Option<String>,
        _: InvocationContext,
        _: FutureReader<String>,
    ) -> Result<Option<String>, AppError> {
        unreachable!()
    }
    async fn deliver_wake(
        _: Wake,
        _: bool,
        _: InvocationContext,
        _: FutureReader<String>,
    ) -> Result<(), AppError> {
        unreachable!()
    }
}
