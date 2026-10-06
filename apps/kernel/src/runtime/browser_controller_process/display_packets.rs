//! MP-08/MP-10/MP-11: private native stripe bytes bypass the Node control loop.
//! This descriptor never crosses a client/relay contract or enters replay.
use serde_json::Value;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::PathBuf;

pub(super) struct DisplayPackets {
    root: PathBuf,
    #[cfg(unix)]
    directory: File,
}

impl DisplayPackets {
    pub(super) fn create() -> Result<Self, String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
            let root = std::env::temp_dir().join(format!(
                "chariox-display-{}",
                format!("{:032x}", rand::random::<u128>())
            ));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&root)
                .map_err(|_| "MP-11: cannot create native packet directory")?;
            let directory = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
                .open(&root)
                .map_err(|_| "MP-11: cannot pin native packet directory")?;
            Ok(Self { root, directory })
        }
        #[cfg(not(unix))]
        Err("MP-10: native packet path requires Unix".into())
    }

    pub(super) fn root(&self) -> &std::path::Path {
        &self.root
    }

    pub(super) fn hydrate(&self, result: &mut Value) -> Result<(), String> {
        let Some(frame) = result.get_mut("display_frame") else {
            return Ok(());
        };
        let Some(descriptor) = frame.get("native_packet") else {
            return Ok(());
        };
        let name = descriptor
            .get("name")
            .and_then(Value::as_str)
            .ok_or("MP-11: packet name")?;
        let length = descriptor
            .get("length")
            .and_then(Value::as_u64)
            .ok_or("MP-11: packet length")?;
        if name.len() != 37
            || !name.ends_with(".json")
            || !name.bytes().take(32).all(|b| b.is_ascii_hexdigit())
            || length == 0
            || length > 1024 * 1024
            || frame["kind"] != "stripes"
        {
            return Err("MP-11: native packet bounds".into());
        }
        #[cfg(unix)]
        {
            use std::os::fd::{AsRawFd, FromRawFd};
            use std::os::unix::fs::MetadataExt;
            let name = std::ffi::CString::new(name).map_err(|_| "MP-11: packet name")?;
            // The retained directory FD prevents parent/symlink substitution.
            let fd = unsafe {
                libc::openat(
                    self.directory.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err("MP-11: native packet unavailable".into());
            }
            let mut file = unsafe { File::from_raw_fd(fd) };
            let metadata = file
                .metadata()
                .map_err(|_| "MP-11: native packet metadata")?;
            if !metadata.is_file()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.mode() & 0o077 != 0
                || metadata.nlink() != 1
                || metadata.len() != length
            {
                return Err("MP-11: native packet owner/size".into());
            }
            // Unlink before parsing: every admitted descriptor is consumed once,
            // including malformed JSON. The open descriptor pins these bytes.
            if unsafe { libc::unlinkat(self.directory.as_raw_fd(), name.as_ptr(), 0) } != 0 {
                return Err("MP-11: native packet retirement".into());
            }
            let mut bytes = Vec::with_capacity(length as usize);
            (&mut file)
                .take(length + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "MP-11: native packet read")?;
            if bytes.len() as u64 != length {
                return Err("MP-11: native packet changed".into());
            }
            let stripes: Value =
                serde_json::from_slice(&bytes).map_err(|_| "MP-11: native stripe JSON")?;
            let rows = stripes.as_array().ok_or("MP-11: native stripe array")?;
            let headers = frame["stripes"]
                .as_array()
                .ok_or("MP-11: native stripe headers")?;
            if rows.is_empty() || rows.len() > 8 || rows.len() != headers.len() {
                return Err("MP-11: native stripe count".into());
            }
            for (row, header) in rows.iter().zip(headers) {
                let mut record = row.clone();
                let data = record
                    .as_object_mut()
                    .ok_or("MP-11: native stripe record")?
                    .remove("data_base64")
                    .ok_or("MP-11: native stripe bytes")?;
                let data = data.as_str().ok_or("MP-11: native stripe encoding")?;
                if record != *header
                    || data.is_empty()
                    || data.len() > 1024 * 1024
                    || !data
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b))
                {
                    return Err("MP-11: native stripe binding".into());
                }
            }
            frame
                .as_object_mut()
                .ok_or("MP-11: native frame")?
                .remove("native_packet");
            frame["stripes"] = stripes;
            Ok(())
        }
        #[cfg(not(unix))]
        Err("MP-11: native packet unsupported".into())
    }
}

impl Drop for DisplayPackets {
    fn drop(&mut self) {
        // Only this unique directory and our fixed packet names are disposable.
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            use std::os::unix::fs::MetadataExt;
            let same = std::fs::symlink_metadata(&self.root)
                .ok()
                .zip(self.directory.metadata().ok())
                .is_some_and(|(a, b)| a.is_dir() && a.dev() == b.dev() && a.ino() == b.ino());
            if !same {
                return;
            }
            if let Ok(entries) = std::fs::read_dir(&self.root) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.len() == 37
                        && name.ends_with(".json")
                        && name.bytes().take(32).all(|b| b.is_ascii_hexdigit())
                    {
                        if let Ok(name) = std::ffi::CString::new(name) {
                            unsafe {
                                libc::unlinkat(self.directory.as_raw_fd(), name.as_ptr(), 0);
                            }
                        }
                    }
                }
            }
            let _ = std::fs::remove_dir(&self.root);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;
    fn packet(spool: &DisplayPackets) -> (PathBuf, Value) {
        let name = format!("{}.json", format!("{:032x}", rand::random::<u128>()));
        let row = json!({"row":0,"sequence":1,"key":true,"data_base64":"AA=="});
        let bytes = serde_json::to_vec(&json!([row])).unwrap();
        let path = spool.root.join(&name);
        std::fs::write(&path, &bytes).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let reply = json!({"display_frame":{"kind":"stripes","stripes":[{"row":0,"sequence":1,"key":true}],"native_packet":{"name":name,"length":bytes.len()}}});
        (path, reply)
    }
    #[test]
    fn mp11_native_packets_bind_headers_and_consume_once() {
        let spool = DisplayPackets::create().unwrap();
        let (path, mut reply) = packet(&spool);
        spool.hydrate(&mut reply).unwrap();
        assert!(!path.exists());
        assert_eq!(reply["display_frame"]["stripes"][0]["data_base64"], "AA==");
        assert!(reply["display_frame"].get("native_packet").is_none());
        let (path, mut bad) = packet(&spool);
        bad["display_frame"]["stripes"][0]["sequence"] = json!(2);
        assert!(spool.hydrate(&mut bad).is_err());
        assert!(!path.exists());
    }
    #[test]
    fn mp11_native_packets_reject_traversal_symlinks_permissions_and_size() {
        let spool = DisplayPackets::create().unwrap();
        for mode in ["traversal", "symlink", "permissions", "size", "hardlink"] {
            let (path, mut reply) = packet(&spool);
            match mode {
                "traversal" => {
                    reply["display_frame"]["native_packet"]["name"] = json!("../other.json")
                }
                "symlink" => {
                    std::fs::remove_file(&path).unwrap();
                    std::os::unix::fs::symlink("/dev/null", &path).unwrap()
                }
                "permissions" => {
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap()
                }
                "hardlink" => std::fs::hard_link(&path, spool.root.join("link")).unwrap(),
                _ => reply["display_frame"]["native_packet"]["length"] = json!(1024 * 1024 + 1),
            }
            assert!(spool.hydrate(&mut reply).is_err(), "{mode}");
            std::fs::remove_file(&path).unwrap();
            if mode == "hardlink" {
                std::fs::remove_file(spool.root.join("link")).unwrap()
            }
        }
    }
}
