//! Offline admission for source-built SDK components, using the install policy.
use chariox_app_wit::{check_component, APP_WORLD};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("usage: check-component <app.wasm>")?;
    let bytes = std::fs::read(path)?;
    check_component(&bytes, APP_WORLD)?;
    println!("accepted {APP_WORLD} ({} bytes)", bytes.len());
    Ok(())
}
