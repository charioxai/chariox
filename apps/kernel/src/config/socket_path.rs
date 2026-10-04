use sha2::{Digest, Sha256};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

/// Match the Node client's lexical absolute-path normalization; do not depend
/// on the home already existing or resolve symlinks differently at discovery.
fn socket_scope(root: &Path) -> PathBuf {
    let absolute = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(root)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component),
        }
    }
    normalized
}

/// Full SHA-256 keeps long slice IDs and separate kernel homes distinct, while
/// the fixed per-user parent bounds the address even for a deeply nested home.
pub(super) fn default_socket_path(daemon_id: &str, config_root: &Path, uid: u32) -> PathBuf {
    let mut hash = Sha256::new();
    hash.update(b"chariox-unix-socket-v1\0");
    hash.update(socket_scope(config_root).as_os_str().as_bytes());
    hash.update(b"\0");
    hash.update(daemon_id.as_bytes());
    PathBuf::from(format!("/tmp/chariox-{uid}")).join(format!("{:x}.sock", hash.finalize()))
}

pub(crate) fn validate_local_socket_path(path: &Path) -> io::Result<()> {
    let bytes = path.as_os_str().as_bytes();
    // Reserve the terminator: Linux has 108 bytes, Darwin has 104.
    let address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    let maximum = address.sun_path.len() - 1;
    if bytes.contains(&0) || bytes.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Unix socket path must be nonempty and contain no NUL bytes",
        ));
    }
    if bytes.len() > maximum {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!(
            "Unix socket path is {} bytes; this platform allows at most {maximum} bytes. Set CHARIOX_DAEMON_SOCKET to a shorter absolute path.", bytes.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn maximal_slice_socket_binds_and_accepts_from_a_deep_home() {
        let id = format!("slice:{}:{}", "a".repeat(64), "b".repeat(64));
        let home = PathBuf::from(format!(
            "/synthetic/{}/{}/kernel",
            "deep/".repeat(40),
            rand::random::<u64>()
        ));
        let path = default_socket_path(&id, &home, unsafe { libc::geteuid() });
        let listener = crate::local::ipc::LocalIpcListener::bind(path.clone()).unwrap();
        let peer = tokio::net::UnixStream::connect(&path).await.unwrap();
        let (accepted, _) = listener.listener.accept().await.unwrap();
        drop((peer, accepted, listener));
        assert!(!path.exists());
    }

    #[test]
    fn oversized_override_is_rejected_before_creating_directories() {
        let root = std::env::temp_dir().join(format!("chx-too-long-{}", rand::random::<u64>()));
        let path = root.join("é".repeat(108)).join("kernel.sock");
        let error = crate::local::ipc::LocalIpcListener::bind(path.clone())
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error
            .to_string()
            .contains(&format!("{} bytes", path.as_os_str().len())));
        assert!(error.to_string().contains("CHARIOX_DAEMON_SOCKET"));
        assert!(!root.exists());
        let mut config = crate::config::DaemonConfig::for_tests();
        config.local_socket_path = path;
        assert!(config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("this platform allows at most"));
    }

    #[test]
    fn maximal_slice_identity_and_deep_home_have_bounded_distinct_addresses() {
        let id = format!("slice:{}:{}", "a".repeat(64), "b".repeat(64));
        let home = PathBuf::from(format!(
            "/var/lib/chariox/{}/slice-private/kernel",
            "deep/".repeat(40)
        ));
        assert!(
            home.join("run")
                .join(format!("{id}.sock"))
                .as_os_str()
                .len()
                > 182
        );
        let path = default_socket_path(&id, &home, u32::MAX);
        validate_local_socket_path(&path).unwrap();
        assert!(path.as_os_str().len() < 104);
        assert_eq!(path, default_socket_path(&id, &home.join("."), u32::MAX));
        assert_ne!(
            path,
            default_socket_path(&id, &home.join("other"), u32::MAX)
        );
        assert_ne!(path, default_socket_path(&(id + "c"), &home, u32::MAX));
    }

    #[test]
    fn socket_path_agrees_with_node_discovery_fixture() {
        let id = format!("slice:{}:{}", "a".repeat(64), "b".repeat(64));
        let path = default_socket_path(&id, Path::new("/var/lib/chariox/slice-private/kernel"), 42);
        assert_eq!(path, PathBuf::from("/tmp/chariox-42/4445472abcb8084eeec1def5239514b53fa12c0919b849070a897b750ba5f609.sock"));
        assert_eq!(
            path,
            default_socket_path(
                &id,
                Path::new("/var/lib/chariox/other/../slice-private//kernel/."),
                42
            )
        );
    }
}
