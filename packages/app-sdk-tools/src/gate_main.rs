//! Internal gate authoring build step: wrap a pure Rust cdylib with its exact
//! WIT metadata, then admit the resulting portable component. No WASI adapter.
use std::{
    env,
    fs::File,
    io::{Read, Write},
    process::ExitCode,
};
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let input = args
        .next()
        .ok_or("usage: chariox-gate-component MODULE OUTPUT")?;
    let output = args
        .next()
        .ok_or("usage: chariox-gate-component MODULE OUTPUT")?;
    if args.next().is_some() {
        return Err("usage: chariox-gate-component MODULE OUTPUT".into());
    }
    let mut bytes = Vec::new();
    File::open(input)?
        .take(chariox_app_wit::gate::MAX_COMPONENT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > chariox_app_wit::gate::MAX_COMPONENT_BYTES {
        return Err("gate module exceeds 2 MiB".into());
    }
    let component = wit_component::ComponentEncoder::default()
        .module(&bytes)?
        .validate(true)
        .encode()?;
    chariox_app_wit::gate::check(&component)?;
    File::create(output)?.write_all(&component)?;
    Ok(())
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
