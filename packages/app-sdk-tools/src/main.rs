use std::{env, fs, process::ExitCode};

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let path = args
        .next()
        .ok_or("usage: chariox-app-component <component.wasm> [prepared.wasm]")?;
    let output = args.next();
    if args.next().is_some() {
        return Err("usage: chariox-app-component <component.wasm> [prepared.wasm]".into());
    }
    let bytes = fs::read(path)?;
    if let Some(output) = output {
        let component = chariox_app_sdk_tools::prepare_component(&bytes)?;
        fs::write(output, component)?;
    } else {
        chariox_app_wit::check_component(&bytes, chariox_app_wit::APP_WORLD)?;
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
