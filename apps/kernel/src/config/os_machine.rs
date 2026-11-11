//! The computer's own identity and official name.
//!
//! Every kernel on one computer reports the same machine ID, and the name is
//! whatever the operating system calls the computer, read again each time it is
//! asked for so renaming the computer in the OS renames it in Chariox.

use std::fs;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

const NAME_CACHE_TTL: Duration = Duration::from_secs(2);

/// The ID is a one-way digest of the OS machine ID: systemd documents the raw
/// value as confidential, and Cloud and relays only need it to be stable.
pub(crate) fn machine_id() -> Result<String, String> {
    let raw = read_os_machine_id()?;
    machine_id_from_os_value(&raw)
}

fn machine_id_from_os_value(raw: &str) -> Result<String, String> {
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.is_empty() || normalized.chars().all(|c| c == '0' || c == '-') {
        return Err("the OS machine ID is empty".to_string());
    }
    let digest = Sha256::digest(format!("chariox-machine-id/v1\n{normalized}").as_bytes());
    let hex = digest
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("machine-{hex}"))
}

#[cfg(target_os = "macos")]
fn read_os_machine_id() -> Result<String, String> {
    let output = command_output("ioreg", &["-rd1", "-c", "IOPlatformExpertDevice"])
        .ok_or_else(|| "ioreg did not report IOPlatformExpertDevice".to_string())?;
    parse_ioplatform_uuid(&output)
        .ok_or_else(|| "ioreg did not report an IOPlatformUUID".to_string())
}

#[cfg(target_os = "windows")]
fn read_os_machine_id() -> Result<String, String> {
    let output = command_output(
        "reg",
        &[
            "query",
            r"HKLM\SOFTWARE\Microsoft\Cryptography",
            "/v",
            "MachineGuid",
        ],
    )
    .ok_or_else(|| "the registry did not report MachineGuid".to_string())?;
    parse_machine_guid(&output).ok_or_else(|| "the registry MachineGuid is empty".to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn read_os_machine_id() -> Result<String, String> {
    for path in ["/etc/machine-id", "/var/lib/dbus/machine-id"] {
        if let Ok(contents) = fs::read_to_string(path) {
            let value = contents.trim();
            if !value.is_empty() {
                return Ok(value.to_string());
            }
        }
    }
    Err("/etc/machine-id is missing or empty; run systemd-machine-id-setup".to_string())
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_ioplatform_uuid(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        if key.trim().trim_matches('"') != "IOPlatformUUID" {
            return None;
        }
        let uuid = value.trim().trim_matches('"');
        (!uuid.is_empty()).then(|| uuid.to_string())
    })
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn parse_machine_guid(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        if words.next()? != "MachineGuid" {
            return None;
        }
        words.next()?;
        words.next().map(str::to_string)
    })
}

/// The OS's official name for this computer, read at most every two seconds.
#[cfg_attr(test, allow(dead_code))]
pub(crate) fn machine_name() -> Option<String> {
    static CACHE: Mutex<Option<(Instant, Option<String>)>> = Mutex::new(None);
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((read_at, name)) = cache.as_ref() {
        if read_at.elapsed() < NAME_CACHE_TTL {
            return name.clone();
        }
    }
    let name = read_os_machine_name();
    *cache = Some((Instant::now(), name.clone()));
    name
}

#[cfg(target_os = "macos")]
#[cfg_attr(test, allow(dead_code))]
fn read_os_machine_name() -> Option<String> {
    command_output("scutil", &["--get", "ComputerName"])
}

#[cfg(target_os = "windows")]
fn read_os_machine_name() -> Option<String> {
    std::env::var("COMPUTERNAME")
        .ok()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn read_os_machine_name() -> Option<String> {
    fs::read_to_string("/etc/machine-info")
        .ok()
        .and_then(|contents| parse_pretty_hostname(&contents))
        .or_else(|| {
            fs::read_to_string("/proc/sys/kernel/hostname")
                .ok()
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
        })
        .or_else(|| command_output("hostname", &[]))
}

#[cfg_attr(any(target_os = "macos", target_os = "windows"), allow(dead_code))]
fn parse_pretty_hostname(machine_info: &str) -> Option<String> {
    machine_info.lines().find_map(|line| {
        let value = line.trim().strip_prefix("PRETTY_HOSTNAME=")?;
        let value = value.trim().trim_matches(|c| c == '"' || c == '\'').trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

#[cfg_attr(
    any(
        test,
        not(any(target_os = "macos", target_os = "windows", target_os = "linux"))
    ),
    allow(dead_code)
)]
fn command_output(command: &str, args: &[&str]) -> Option<String> {
    use crate::runtime::bounded_process::BoundedCommandExt as _;
    let output = crate::process_spawn::command(command)
        .args(args)
        .bounded_output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_id_is_a_stable_digest_of_the_os_value_in_any_case() {
        let upper = machine_id_from_os_value("A1B2C3D4-0000-1111-2222-333344445555\n").unwrap();
        let lower = machine_id_from_os_value("a1b2c3d4-0000-1111-2222-333344445555").unwrap();
        assert_eq!(upper, lower);
        assert!(upper.starts_with("machine-"));
        assert_eq!(upper.len(), "machine-".len() + 32);
        assert!(upper["machine-".len()..]
            .bytes()
            .all(|b| b.is_ascii_hexdigit()));
        assert!(!upper.contains("a1b2c3d4"));
        assert_ne!(upper, machine_id_from_os_value("another-computer").unwrap());
    }

    #[test]
    fn an_empty_os_machine_id_is_refused() {
        assert!(machine_id_from_os_value("  \n").is_err());
        assert!(machine_id_from_os_value("00000000-0000-0000-0000-000000000000").is_err());
    }

    #[test]
    fn macos_platform_uuid_is_read_from_ioreg_output() {
        let output = "+-o Root  <class IORegistryEntry>\n  +-o MacBookPro18,3  <class IOPlatformExpertDevice>\n    {\n      \"IOPlatformSerialNumber\" = \"ABC123\"\n      \"IOPlatformUUID\" = \"8E2F5F7A-1111-2222-3333-444455556666\"\n    }\n";
        assert_eq!(
            parse_ioplatform_uuid(output).as_deref(),
            Some("8E2F5F7A-1111-2222-3333-444455556666")
        );
        assert_eq!(parse_ioplatform_uuid("\"IOPlatformUUID\" = \"\""), None);
    }

    #[test]
    fn windows_machine_guid_is_read_from_registry_output() {
        let output = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Cryptography\r\n    MachineGuid    REG_SZ    5b1c-guid\r\n";
        assert_eq!(parse_machine_guid(output).as_deref(), Some("5b1c-guid"));
        assert_eq!(parse_machine_guid("nothing here"), None);
    }

    #[test]
    fn linux_pretty_hostname_wins_when_set() {
        let info = "CHASSIS=laptop\nPRETTY_HOSTNAME=\"Miguel's Laptop\"\n";
        assert_eq!(
            parse_pretty_hostname(info).as_deref(),
            Some("Miguel's Laptop")
        );
        assert_eq!(parse_pretty_hostname("PRETTY_HOSTNAME=\n"), None);
        assert_eq!(parse_pretty_hostname("CHASSIS=server\n"), None);
    }
}
