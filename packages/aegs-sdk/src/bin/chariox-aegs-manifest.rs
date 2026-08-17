//! Small, dependency-light manifest utility for AEGS publishers.
//!
//! Private keys are read from a file or `CHARIOX_AEGS_SIGNING_KEY`; they are
//! never accepted as command-line arguments and are never printed.

use std::{env, fs, path::PathBuf, process::ExitCode};

use chariox_aegs_sdk::{
    parse_signing_key, sign_manifest, unsigned_manifest_digest, validate_manifest_envelope,
};
use serde_json::Value;

fn usage() -> ! {
    eprintln!(
        "usage:\n  chariox-aegs-manifest digest --input FILE\n  chariox-aegs-manifest validate --input FILE\n  chariox-aegs-manifest sign --input FILE --output FILE --key-id ID [--key-file FILE | --key-env NAME]"
    );
    std::process::exit(2)
}

fn option(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
}

fn required_option(args: &[String], name: &str) -> String {
    option(args, name)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            eprintln!("missing required option {name}");
            usage()
        })
}

fn read_json(path: &str) -> Result<Value, String> {
    let bytes = fs::read(path).map_err(|error| format!("cannot read {path}: {error}"))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("invalid JSON in {path}: {error}"))
}

fn run(args: &[String]) -> Result<(), String> {
    let command = args.get(1).map(String::as_str).unwrap_or_else(|| usage());
    let input = required_option(args, "--input");
    let manifest = read_json(&input)?;
    match command {
        "digest" => println!("{}", unsigned_manifest_digest(&manifest)?),
        "validate" => println!("valid {}", validate_manifest_envelope(&manifest)?),
        "sign" => {
            let output = required_option(args, "--output");
            let key_id = required_option(args, "--key-id");
            let key_text = if let Some(path) = option(args, "--key-file") {
                fs::read_to_string(&path)
                    .map_err(|error| format!("cannot read signing key file {path}: {error}"))?
            } else {
                let variable = option(args, "--key-env")
                    .unwrap_or_else(|| "CHARIOX_AEGS_SIGNING_KEY".to_string());
                env::var(&variable).map_err(|_| {
                    format!("signing key environment variable {variable} is not set")
                })?
            };
            let signed = sign_manifest(&manifest, key_id, &parse_signing_key(&key_text)?)?;
            let bytes = serde_json::to_vec_pretty(&signed).map_err(|error| error.to_string())?;
            let output = PathBuf::from(output);
            fs::write(&output, bytes)
                .map_err(|error| format!("cannot write {}: {error}", output.display()))?;
            println!("signed {}", output.display());
        }
        _ => usage(),
    }
    Ok(())
}

fn main() -> ExitCode {
    let args = env::args().collect::<Vec<_>>();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
