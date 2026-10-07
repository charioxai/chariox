//! Argument admission runs before logging, identity, broker or runtime setup.
use std::ffi::OsString;

pub enum Command {
    Run,
    Help,
    Version,
    ProtocolVersion,
    PrepareProtectedSliceIdentity(u16),
    OwnerManagedEnroll,
    OwnerManagedReady,
}

pub const USAGE: &str = "usage: chariox-kernel [--help | --version | --print-local-daemon-protocol-version | --prepare-protected-slice-identity PORT]

With no arguments, run the kernel. Configure it through CHARIOX_HOME,
CHARIOX_KERNEL_HOST, CHARIOX_KERNEL_PORT and the Chariox configuration.
  -h, --help                            Print usage and exit
  --version                             Print the package version and exit
  --print-local-daemon-protocol-version  Print the shared protocol version and exit
  --prepare-protected-slice-identity PORT
                                        Prepare a protected slice identity";

pub fn parse(args: Vec<OsString>) -> Result<Command, String> {
    let Some(first) = args.first().and_then(|arg| arg.to_str()) else {
        return if args.is_empty() {
            Ok(Command::Run)
        } else {
            Err("arguments must be valid UTF-8".into())
        };
    };
    match (first, args.len()) {
        ("--help" | "-h", 1) => Ok(Command::Help),
        ("--version", 1) => Ok(Command::Version),
        ("--print-local-daemon-protocol-version", 1) => Ok(Command::ProtocolVersion),
        ("--owner-managed-enroll-stdin", 1) => Ok(Command::OwnerManagedEnroll),
        ("--owner-managed-ready", 1) => Ok(Command::OwnerManagedReady),
        ("--prepare-protected-slice-identity", 2) => {
            let port = args[1]
                .to_str()
                .and_then(|value| value.parse::<u16>().ok())
                .filter(|port| *port > 0)
                .ok_or("--prepare-protected-slice-identity requires a port from 1 to 65535")?;
            Ok(Command::PrepareProtectedSliceIdentity(port))
        }
        ("--prepare-protected-slice-identity", _) => {
            Err("--prepare-protected-slice-identity requires exactly one PORT argument".into())
        }
        ("--help" | "-h" | "--version" | "--print-local-daemon-protocol-version", _) => {
            Err(format!("{first} does not accept additional arguments"))
        }
        _ => Err(format!("unknown argument {first}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byom_mp07_mp08_mp11_bootstrap_flags_are_admitted() {
        assert!(matches!(
            parse(vec!["--owner-managed-enroll-stdin".into()]),
            Ok(Command::OwnerManagedEnroll)
        ));
        assert!(matches!(
            parse(vec!["--owner-managed-ready".into()]),
            Ok(Command::OwnerManagedReady)
        ));
    }

    #[test]
    fn byom_mp11_bootstrap_flags_reject_extra_arguments() {
        for flag in ["--owner-managed-enroll-stdin", "--owner-managed-ready"] {
            assert!(parse(vec![flag.into(), "unexpected".into()]).is_err());
        }
    }
}
