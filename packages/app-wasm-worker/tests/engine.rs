use chariox_app_wasm_worker::engine::check;
use chariox_app_wit::APP_WORLD;
mod support;

#[test]
fn r1_forward_vector_reports_engine_incompatibility() {
    let bytes = support::vector(include_str!("../../app-wit/tests/fixtures/v1.wit"), false);
    chariox_app_wit::check_component(&bytes, APP_WORLD).unwrap();
    let error = check(&bytes, APP_WORLD).unwrap_err();
    assert!(format!("{error:#}").contains("component model forward feature"));
}

#[test]
fn r1_async_vector_without_unused_forward_compiles_in_the_real_engine() {
    let bytes = support::vector(include_str!("../../app-wit/tests/fixtures/v1.wit"), true);
    check(&bytes, APP_WORLD).unwrap();
}
