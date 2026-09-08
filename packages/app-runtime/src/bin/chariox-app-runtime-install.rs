//! Privileged installer entry point. Trust key/digest come from the root
//! operator or signed OS installer, never from a bundle-provided key file.
#[cfg(target_os = "linux")]
fn main() {
    use chariox_app_runtime::runtime_enrollment::installer::RuntimeInstaller;
    let run = || -> Result<(), String> {
        if unsafe { libc::getuid() } != 0 || unsafe { libc::geteuid() } != 0 {
            return Err("app_runtime_installer_requires_root".into());
        }
        let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
        match command::parse(&arguments).map_err(str::to_owned)? {
            command::Command::Install {
                source,
                key,
                digest,
            } => {
                let receipt = RuntimeInstaller::install(&source, key, &digest)
                    .map_err(|error| error.to_string())?;
                println!(
                    "{}",
                    serde_json::to_string(&receipt).map_err(|_| "app_runtime_installer_output")?
                );
            }
            command::Command::Cleanup { digest } => {
                RuntimeInstaller::cleanup(&digest).map_err(|error| error.to_string())?;
                println!("{{\"cleanupComplete\":true}}");
            }
        }
        Ok(())
    };
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("app_runtime_installer_platform_unsupported");
    std::process::exit(1);
}

#[cfg(any(target_os = "linux", test))]
mod command {
    use std::{ffi::OsString, path::PathBuf};
    pub enum Command {
        Install {
            source: PathBuf,
            key: [u8; 32],
            digest: String,
        },
        Cleanup {
            digest: String,
        },
    }
    pub fn parse(arguments: &[OsString]) -> Result<Command, &'static str> {
        const INVALID: &str = "app_runtime_installer_arguments";
        let Some(verb) = arguments.first().and_then(|value| value.to_str()) else {
            return Err(INVALID);
        };
        if arguments.len()
            != if verb == "install" {
                7
            } else if verb == "cleanup" {
                3
            } else {
                return Err(INVALID);
            }
        {
            return Err(INVALID);
        }
        let mut source = None;
        let mut key = None;
        let mut digest = None;
        for pair in arguments[1..].chunks_exact(2) {
            match pair[0].to_str() {
                Some("--source") if verb == "install" && source.is_none() => {
                    let path = PathBuf::from(&pair[1]);
                    if !path.is_absolute() || pair[1].len() > 1024 {
                        return Err(INVALID);
                    }
                    source = Some(path);
                }
                Some("--trusted-public-key-hex") if verb == "install" && key.is_none() => {
                    key = Some(hex32(pair[1].to_str().ok_or(INVALID)?)?);
                }
                Some("--inventory-sha256") if digest.is_none() => {
                    let value = pair[1].to_str().ok_or(INVALID)?;
                    hex32(value)?;
                    digest = Some(value.into());
                }
                _ => return Err(INVALID),
            }
        }
        let digest = digest.ok_or(INVALID)?;
        if verb == "cleanup" {
            Ok(Command::Cleanup { digest })
        } else {
            Ok(Command::Install {
                source: source.ok_or(INVALID)?,
                key: key.ok_or(INVALID)?,
                digest,
            })
        }
    }
    fn hex32(value: &str) -> Result<[u8; 32], &'static str> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("app_runtime_installer_arguments");
        }
        let mut bytes = [0; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
                .map_err(|_| "app_runtime_installer_arguments")?;
        }
        Ok(bytes)
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn only_explicit_external_key_and_digest_with_fixed_output_paths_are_accepted() {
            let args: Vec<OsString> = [
                "install",
                "--source",
                "/tmp/signed bundle",
                "--trusted-public-key-hex",
                &"a".repeat(64),
                "--inventory-sha256",
                &"b".repeat(64),
            ]
            .into_iter()
            .map(Into::into)
            .collect();
            assert!(matches!(parse(&args), Ok(Command::Install { .. })));
            let mut relative = args.clone();
            relative[2] = "bundle".into();
            assert!(parse(&relative).is_err());
            let mut bundle_key = args.clone();
            bundle_key[3] = "--key-file".into();
            assert!(parse(&bundle_key).is_err());
            let mut output = args.clone();
            output[3] = "--output".into();
            assert!(parse(&output).is_err());
            let mut duplicate = args;
            duplicate[3] = "--inventory-sha256".into();
            assert!(parse(&duplicate).is_err());
        }
    }
}
