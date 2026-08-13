use std::sync::Arc;

use chariox_aegs_dummy::DummyProvider;
use chariox_aegs_sdk::run_from_environment;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_from_environment(|_| Ok(Arc::new(DummyProvider))).await
}
