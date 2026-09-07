use std::{collections::BTreeMap, path::Path};

use serde::Serialize;
use serde_json::{json, Value};

use super::{
    generate_manifest, inspect_archive, keygen, pack_directory, read_manifest, read_publisher,
    validate_archive, write_manifest, ManifestOptions,
};
use crate::{ErrorCode, Limits, NetworkDestination, PackageError, Result};

pub fn usage() -> &'static str {
    "chariox-app-package keygen --publisher-id ID --publisher-name NAME --key-out PRIVATE --trust-out PUBLIC\nchariox-app-package manifest --app-id ID --version SEMVER --publisher PUBLIC --kernel-protocol N --output FILE [--runtime-entry PATH --ui-entry PATH --tools PATH --events PATH --actions PATH --information-sets PATH --network GET,POST=https://api.example.com]\nchariox-app-package pack --bundle DIR --manifest FILE --key PRIVATE --output FILE --kernel-protocol N\nchariox-app-package validate ARCHIVE --trust PUBLIC --kernel-protocol N\nchariox-app-package inspect ARCHIVE"
}

fn arguments_error() -> PackageError {
    PackageError::new(
        ErrorCode::InvalidArguments,
        "invalid command arguments; use --help for the command contract",
    )
}

struct Arguments<'a> {
    flags: BTreeMap<&'a str, &'a str>,
    positional: Vec<&'a str>,
    networks: Vec<&'a str>,
}

impl<'a> Arguments<'a> {
    fn parse(args: &'a [String], allowed: &[&str], positionals: usize) -> Result<Self> {
        if args.len() > 64 || args.iter().any(|value| value.len() > 4096) {
            return Err(arguments_error());
        }
        let mut parsed = Self {
            flags: BTreeMap::new(),
            positional: Vec::new(),
            networks: Vec::new(),
        };
        let mut index = 0;
        while index < args.len() {
            let argument = args[index].as_str();
            if argument.starts_with('-') {
                if !allowed.contains(&argument)
                    || index + 1 >= args.len()
                    || args[index + 1].starts_with("--")
                {
                    return Err(arguments_error());
                }
                let value = args[index + 1].as_str();
                if argument == "--network" {
                    parsed.networks.push(value);
                } else if parsed.flags.insert(argument, value).is_some() {
                    return Err(arguments_error());
                }
                index += 2;
            } else {
                parsed.positional.push(argument);
                index += 1;
            }
        }
        if parsed.positional.len() != positionals {
            return Err(arguments_error());
        }
        Ok(parsed)
    }

    fn required(&self, name: &str) -> Result<&str> {
        self.flags
            .get(name)
            .copied()
            .filter(|value| !value.is_empty())
            .ok_or_else(arguments_error)
    }
    fn optional(&self, name: &str) -> Option<String> {
        self.flags.get(name).map(|value| (*value).to_owned())
    }
    fn protocol(&self) -> Result<u32> {
        self.required("--kernel-protocol")?
            .parse::<u32>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(arguments_error)
    }
}

fn output(value: impl Serialize) -> Result<Value> {
    serde_json::to_value(value)
        .map_err(|_| PackageError::new(ErrorCode::Io, "CLI result encoding failed"))
}

/// Returns the JSON result value. The executable wraps it as `{ok:true,result}`;
/// shared clients can consume the same result without spawning another process.
pub fn run_cli(args: &[String]) -> Result<Value> {
    if args == ["--help"] || args == ["help"] {
        return Ok(json!({"usage":usage()}));
    }
    let Some(command) = args.first() else {
        return Err(arguments_error());
    };
    let limits = Limits::default();
    let rest = &args[1..];
    match command.as_str() {
        "keygen" => {
            let args = Arguments::parse(
                rest,
                &[
                    "--publisher-id",
                    "--publisher-name",
                    "--key-out",
                    "--trust-out",
                ],
                0,
            )?;
            output(keygen(
                args.required("--publisher-id")?,
                args.required("--publisher-name")?,
                Path::new(args.required("--key-out")?),
                Path::new(args.required("--trust-out")?),
            )?)
        }
        "manifest" => {
            let args = Arguments::parse(
                rest,
                &[
                    "--app-id",
                    "--version",
                    "--publisher",
                    "--kernel-protocol",
                    "--output",
                    "--runtime-entry",
                    "--ui-entry",
                    "--tools",
                    "--events",
                    "--actions",
                    "--information-sets",
                    "--network",
                ],
                0,
            )?;
            let publisher = read_publisher(Path::new(args.required("--publisher")?), &limits)?;
            let mut options = ManifestOptions::new(
                args.required("--app-id")?.to_owned(),
                args.required("--version")?.to_owned(),
                publisher.publisher,
                args.protocol()?,
            );
            if let Some(value) = args.optional("--runtime-entry") {
                options.runtime_entry = value;
            }
            if let Some(value) = args.optional("--ui-entry") {
                options.ui_entry = value;
            }
            options.tools = args.optional("--tools");
            options.events = args.optional("--events");
            options.actions = args.optional("--actions");
            options.information_sets = args.optional("--information-sets");
            for network in &args.networks {
                let (methods, origin) = network.split_once('=').ok_or_else(arguments_error)?;
                let methods = methods
                    .split(',')
                    .map(|method| {
                        serde_json::from_value(Value::String(method.to_owned()))
                            .map_err(|_| arguments_error())
                    })
                    .collect::<Result<Vec<_>>>()?;
                options.capabilities.network.push(NetworkDestination {
                    origin: origin.to_owned(),
                    methods,
                });
            }
            let manifest = generate_manifest(options, &limits)?;
            write_manifest(Path::new(args.required("--output")?), &manifest, &limits)?;
            Ok(
                json!({"status":"manifest-created","manifest":manifest,"output":args.required("--output")?}),
            )
        }
        "pack" => {
            let args = Arguments::parse(
                rest,
                &[
                    "--bundle",
                    "--manifest",
                    "--key",
                    "--output",
                    "--kernel-protocol",
                ],
                0,
            )?;
            let manifest = read_manifest(Path::new(args.required("--manifest")?), &limits)?;
            output(pack_directory(
                Path::new(args.required("--bundle")?),
                &manifest,
                Path::new(args.required("--key")?),
                Path::new(args.required("--output")?),
                args.protocol()?,
                &limits,
            )?)
        }
        "validate" => {
            let args = Arguments::parse(rest, &["--trust", "--kernel-protocol"], 1)?;
            let publisher = read_publisher(Path::new(args.required("--trust")?), &limits)?;
            output(validate_archive(
                Path::new(args.positional[0]),
                &publisher,
                args.protocol()?,
                &limits,
            )?)
        }
        "inspect" => {
            let args = Arguments::parse(rest, &[], 1)?;
            output(inspect_archive(Path::new(args.positional[0]), &limits)?)
        }
        _ => Err(arguments_error()),
    }
}
