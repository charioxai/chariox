//! MP-08/MP-10/MP-11: bind a server behind a spawn launcher before any turn.
//! An endpoint holder must be unique and belong to the recorded launch tree.
use crate::runtime::kernel_access::process::{ancestry, inspect, ProcessIdentity};
use std::net::IpAddr;
use std::net::SocketAddr;

pub(super) fn capture(launcher: &ProcessIdentity, endpoint: &str) -> Option<ProcessIdentity> {
    let url = url::Url::parse(endpoint).ok()?;
    let ip = match url.host()? {
        url::Host::Ipv4(ip) => IpAddr::V4(ip),
        url::Host::Ipv6(ip) => IpAddr::V6(ip),
        url::Host::Domain(_) => return None,
    };
    let address = SocketAddr::new(ip, url.port()?);
    if !address.ip().is_loopback() {
        return None;
    }
    let candidates = listener_identities(address)?;
    let [owner] = candidates.as_slice() else {
        return None;
    };
    if owner.uid != launcher.uid
        || !ancestry(owner).ok()?.iter().any(|ancestor| {
            // exec may change the executable, but never the birth identity.
            ancestor.pid == launcher.pid
                && ancestor.uid == launcher.uid
                && ancestor.start == launcher.start
        })
        || !owner.alive()
    {
        return None;
    }
    Some(owner.clone())
}

#[cfg(target_os = "linux")]
fn listener_identities(address: SocketAddr) -> Option<Vec<ProcessIdentity>> {
    let inodes = listener_inodes(address)?;
    if inodes.len() != 1 {
        return None;
    }
    let target = format!("socket:[{}]", inodes[0]);
    let mut pids = Vec::new();
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse().ok())
        else {
            continue;
        };
        let Ok((identity, _)) = inspect(pid) else {
            continue;
        };
        let Ok(fds) = std::fs::read_dir(entry.path().join("fd")) else {
            continue;
        };
        if fds.flatten().any(|fd| {
            std::fs::read_link(fd.path()).is_ok_and(|path| path.as_os_str() == target.as_str())
        }) && identity.alive()
        {
            pids.push(identity);
        }
    }
    // A port replacement during inspection must not become a different root.
    (listener_inodes(address)? == inodes).then_some(pids)
}

#[cfg(target_os = "linux")]
fn listener_inodes(address: SocketAddr) -> Option<Vec<String>> {
    let mut inodes = Vec::new();
    for file in ["/proc/net/tcp", "/proc/net/tcp6"] {
        let table = match std::fs::read_to_string(file) {
            Ok(table) => table,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return None,
        };
        for line in table.lines().skip(1) {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.get(3) != Some(&"0A") {
                continue;
            }
            let (host, port) = fields.get(1)?.split_once(':')?;
            let port = u16::from_str_radix(port, 16).ok()?;
            let words = (0..host.len())
                .step_by(8)
                .map(|offset| {
                    u32::from_str_radix(host.get(offset..offset + 8)?, 16)
                        .ok()
                        .map(u32::to_ne_bytes)
                })
                .collect::<Option<Vec<_>>>()?;
            let ip = match words.as_slice() {
                [word] => IpAddr::from(*word),
                [a, b, c, d] => {
                    let bytes: [u8; 16] = [*a, *b, *c, *d].concat().try_into().ok()?;
                    IpAddr::from(bytes)
                }
                _ => return None,
            };
            if port == address.port() && (ip == address.ip() || ip.is_unspecified()) {
                inodes.push(fields.get(9)?.to_string());
            }
        }
    }
    Some(inodes)
}

#[cfg(target_os = "macos")]
fn listener_identities(address: SocketAddr) -> Option<Vec<ProcessIdentity>> {
    let pids = macos_listener_pids(address)?;
    let identities = pids
        .iter()
        .map(|pid| inspect(*pid).ok().map(|p| p.0))
        .collect::<Option<Vec<_>>>()?;
    (macos_listener_pids(address)? == pids && identities.iter().all(ProcessIdentity::alive))
        .then_some(identities)
}

#[cfg(target_os = "macos")]
fn macos_listener_pids(address: SocketAddr) -> Option<Vec<u32>> {
    let output = std::process::Command::new("/usr/sbin/lsof")
        .args([
            "-nP",
            "-a",
            &format!("-iTCP:{}", address.port()),
            "-sTCP:LISTEN",
            "-Fp",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8(output.stdout)
            .ok()?
            .lines()
            .filter_map(|line| line.strip_prefix('p')?.parse().ok())
            .collect(),
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn listener_identities(_: SocketAddr) -> Option<Vec<ProcessIdentity>> {
    None
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn endpoint_identity_requires_local_listener_and_same_launch_birth() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let endpoint = format!("http://{address}");
        let launched = inspect(std::process::id()).unwrap().0;
        assert_eq!(capture(&launched, &endpoint), Some(launched.clone()));
        let mut reused = launched.clone();
        reused.start += 1;
        assert!(capture(&reused, &endpoint).is_none());
        let mut other_user = launched.clone();
        other_user.uid = other_user.uid.wrapping_add(1);
        assert!(capture(&other_user, &endpoint).is_none());
        let mut sibling = std::process::Command::new("/bin/sh")
            .args(["-c", "exec sleep 1"])
            .spawn()
            .unwrap();
        let sibling_identity = inspect(sibling.id()).unwrap().0;
        let outside_tree = capture(&sibling_identity, &endpoint);
        sibling.wait().unwrap();
        assert!(
            outside_tree.is_none(),
            "an unrelated listener is never a launch root"
        );
        assert!(capture(&launched, "https://example.com:443").is_none());
        drop(listener);
        assert!(capture(&launched, &endpoint).is_none());
    }
}
