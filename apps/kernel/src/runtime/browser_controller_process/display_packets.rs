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
    #[cfg(unix)]
    fn reclaim_private_files(directory: &File, names: &[&str], limit: u64) -> bool {
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::fs::MetadataExt;
        let mut files = Vec::new();
        for name in names {
            let name = std::ffi::CString::new(*name).unwrap();
            let fd = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
                    continue;
                }
                return false;
            }
            let file = unsafe { File::from_raw_fd(fd) };
            let Ok(info) = file.metadata() else {
                return false;
            };
            if !info.is_file()
                || info.uid() != unsafe { libc::geteuid() }
                || info.mode() & 0o077 != 0
                || info.nlink() != 1
                || info.len() > limit
            {
                return false;
            }
            files.push((name, file));
        }
        // Validate every fixed slot before removing any; never read authority bytes.
        for (name, _file) in files {
            unsafe {
                libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0);
            }
        }
        true
    }

    #[cfg(unix)]
    fn reclaim_rasters(&self, name: &str) {
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::fs::MetadataExt;
        let (prefix, files): (&str, &[&str]) = if name.starts_with("encoder-") {
            ("encoder-", &["raster"])
        } else if name.starts_with("raster-") {
            ("raster-", &super::RASTER_SLOT_NAMES)
        } else {
            return;
        };
        if name.len() != prefix.len() + 6
            || !name
                .bytes()
                .skip(prefix.len())
                .all(|b| b.is_ascii_alphanumeric())
        {
            return;
        }
        let Ok(name) = std::ffi::CString::new(name) else {
            return;
        };
        let fd = unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return;
        }
        let directory = unsafe { File::from_raw_fd(fd) };
        let Ok(info) = directory.metadata() else {
            return;
        };
        if info.uid() != unsafe { libc::geteuid() } || info.mode() & 0o077 != 0 {
            return;
        }
        if !Self::reclaim_private_files(&directory, files, 2560 * 1600 * 4) {
            return;
        }
        // Unknown contents prevent removal. Never recurse or follow links.
        unsafe {
            libc::unlinkat(
                self.directory.as_raw_fd(),
                name.as_ptr(),
                libc::AT_REMOVEDIR,
            );
        }
    }

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

    /// MP-08/MP-10/MP-11: bind the packet's segment headers to the frame,
    /// then project each raw segment. The descriptor never reaches a client.
    fn bind(frame: &mut Value, bytes: &[u8]) -> Result<(), String> {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let size = bytes
            .get(..4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
            .filter(|size| *size <= 64 * 1024 && 4 + size <= bytes.len())
            .ok_or("MP-11: native packet header")?;
        let headers: Vec<Value> =
            serde_json::from_slice(&bytes[4..4 + size]).map_err(|_| "MP-11: native packet JSON")?;
        let mut raw = &bytes[4 + size..];
        let mut segments = Vec::with_capacity(headers.len());
        for header in &headers {
            let length = header["length"]
                .as_u64()
                .filter(|n| *n > 0 && *n as usize <= raw.len())
                .ok_or("MP-11: native segment length")? as usize;
            segments.push(STANDARD.encode(&raw[..length]));
            raw = &raw[length..];
        }
        if !raw.is_empty() {
            return Err("MP-11: native packet trailing bytes".into());
        }
        let object = frame.as_object_mut().ok_or("MP-11: native frame")?;
        object.remove("native_packet");
        if object["kind"] == "video" {
            // MP-08/MP-10/MP-11: one native whole-frame access unit.
            let row = match headers.as_slice() {
                [row] => row,
                _ => return Err("MP-11: native video count".into()),
            };
            if row["row"] != 0
                || row["y"] != 0
                || row["height"] != object["height"]
                || row["codec"] != "avc1.420033"
                || row["codec"] != object["codec"]
                || !row["key"].is_boolean()
                || row["key"] != object["key"]
            {
                return Err("MP-11: native video binding".into());
            }
            object.insert("data_base64".into(), segments.pop().unwrap().into());
            return Ok(());
        }
        let key = if object["kind"] == "tiles" {
            "tiles"
        } else {
            "stripes"
        };
        let list = object
            .get_mut(key)
            .and_then(Value::as_array_mut)
            .ok_or("MP-11: native segment headers")?;
        if list.is_empty()
            || list.len() > 256
            || list.len() != headers.len()
            || (key == "stripes" && list.len() > 8)
        {
            return Err("MP-11: native segment count".into());
        }
        for ((record, header), data) in list.iter_mut().zip(&headers).zip(segments) {
            if record != header {
                return Err("MP-11: native segment binding".into());
            }
            let record = record.as_object_mut().ok_or("MP-11: native segment")?;
            record.remove("length");
            record.insert("data_base64".into(), data.into());
        }
        Ok(())
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
            || !matches!(frame["kind"].as_str(), Some("stripes" | "video" | "tiles"))
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
            Self::bind(frame, &bytes)
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
                    self.reclaim_rasters(&name);
                    if name == "display.xauth" {
                        Self::reclaim_private_files(&self.directory, &["display.xauth"], 4096);
                    }
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
    #[test]
    fn mp11_native_packets_reclaim_raster_after_supervisor_death() {
        use std::io::BufRead;
        use std::process::{Command, Stdio};
        let spool = DisplayPackets::create().unwrap();
        let root = spool.root.clone();
        // The supervisor uses Node's real mkdtemp naming and private modes.
        // Kill only this child after its raster is durable, without close().
        let mut child = Command::new("node")
            .args(["--input-type=module", "-e", r#"
                import fs from 'node:fs';
                import path from 'node:path';
                const directory = fs.mkdtempSync(path.join(process.argv[1], 'encoder-'));
                fs.writeFileSync(path.join(directory, 'raster'), Buffer.alloc(16384000), {mode:0o600});
                console.log('ready');
                setInterval(() => {}, 1000);
            "#])
            .arg(&root)
            .stdout(Stdio::piped())
            .spawn().unwrap();
        let mut ready = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut ready)
            .unwrap();
        assert_eq!(ready.trim(), "ready");
        assert!(child.id() > 1, "MP-11: unsafe owned supervisor PID");
        child.kill().unwrap();
        child.wait().unwrap();
        drop(spool);
        let reclaimed = !root.exists();
        // Fail-first must not itself leave the reproduced leak behind.
        if !reclaimed {
            std::fs::remove_dir_all(&root).unwrap();
        }
        assert!(
            reclaimed,
            "MP-11: supervisor crash leaked reusable encoder raster"
        );
    }
    #[test]
    fn mp11_native_packets_reclaim_capture_pool_and_xauth_after_death() {
        let spool = DisplayPackets::create().unwrap();
        let root = spool.root.clone();
        let directory = root.join("raster-Ab1234");
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        // Every slot the worker allocates (review #893 P2: slots 3-5 leaked).
        for name in ["0", "1", "2", "3", "4", "5"] {
            let file = directory.join(name);
            std::fs::write(&file, b"MP-11 owned capture").unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let auth = root.join("display.xauth");
        std::fs::write(&auth, b"MP-11 synthetic authority").unwrap();
        std::fs::set_permissions(&auth, std::fs::Permissions::from_mode(0o600)).unwrap();
        drop(spool);
        let reclaimed = !root.exists();
        if !reclaimed {
            std::fs::remove_dir_all(&root).unwrap();
        }
        assert!(
            reclaimed,
            "MP-11: supervisor death leaked native pool/xauth"
        );
    }
    #[test]
    fn mp11_native_packets_cleanup_rejects_unvalidated_raster_entries() {
        for prefix in ["encoder-", "raster-"] {
            for case in [
                "directory_link",
                "file_link",
                "hardlink",
                "permissions",
                "oversize",
                "unknown",
            ] {
                let spool = DisplayPackets::create().unwrap();
                let root = spool.root.clone();
                let outside = DisplayPackets::create().unwrap();
                let external = outside.root.join("retained");
                std::fs::write(&external, b"MP-11 retained public sentinel").unwrap();
                let directory = root.join(format!("{prefix}Ab1234"));
                if case == "directory_link" {
                    std::os::unix::fs::symlink(&outside.root, &directory).unwrap();
                } else {
                    std::fs::create_dir(&directory).unwrap();
                    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
                        .unwrap();
                    let raster = directory.join(if prefix == "raster-" { "0" } else { "raster" });
                    match case {
                        "file_link" => std::os::unix::fs::symlink(&external, &raster).unwrap(),
                        "hardlink" => std::fs::hard_link(&external, &raster).unwrap(),
                        "unknown" => {
                            std::fs::write(directory.join("other"), b"retain").unwrap();
                        }
                        _ => {
                            let file = OpenOptions::new()
                                .create_new(true)
                                .write(true)
                                .open(&raster)
                                .unwrap();
                            file.set_len(if case == "oversize" {
                                2560 * 1600 * 4 + 1
                            } else {
                                16
                            })
                            .unwrap();
                            std::fs::set_permissions(
                                &raster,
                                std::fs::Permissions::from_mode(if case == "permissions" {
                                    0o644
                                } else {
                                    0o600
                                }),
                            )
                            .unwrap();
                        }
                    }
                }
                drop(spool);
                assert!(directory.symlink_metadata().is_ok(), "{case}");
                assert_eq!(
                    std::fs::read(&external).unwrap(),
                    b"MP-11 retained public sentinel"
                );
                std::fs::remove_dir_all(root).unwrap();
                std::fs::remove_file(external).unwrap();
            }
        }
        let spool = DisplayPackets::create().unwrap();
        let root = spool.root.clone();
        std::fs::create_dir(root.join("encoder-Ab1234")).unwrap();
        std::fs::set_permissions(
            root.join("encoder-Ab1234"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        drop(spool);
        assert!(!root.exists(), "empty partial handoff must be reclaimed");
    }
    #[test]
    fn mp11_native_packets_preserve_unsafe_authority_and_partial_pool() {
        for case in ["symlink", "permissions", "oversize", "hardlink"] {
            let spool = DisplayPackets::create().unwrap();
            let root = spool.root.clone();
            let auth = root.join("display.xauth");
            if case == "symlink" {
                std::os::unix::fs::symlink("/dev/null", &auth).unwrap();
            } else {
                let file = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&auth)
                    .unwrap();
                file.set_len(if case == "oversize" { 4097 } else { 16 })
                    .unwrap();
                std::fs::set_permissions(
                    &auth,
                    std::fs::Permissions::from_mode(if case == "permissions" {
                        0o644
                    } else {
                        0o600
                    }),
                )
                .unwrap();
                if case == "hardlink" {
                    std::fs::hard_link(&auth, root.join("retained")).unwrap();
                }
            }
            drop(spool);
            assert!(auth.symlink_metadata().is_ok(), "{case}");
            std::fs::remove_dir_all(root).unwrap();
        }
        let spool = DisplayPackets::create().unwrap();
        let root = spool.root.clone();
        let directory = root.join("raster-Ab1234");
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(directory.join("0"), b"partial").unwrap();
        std::fs::set_permissions(directory.join("0"), std::fs::Permissions::from_mode(0o600))
            .unwrap();
        drop(spool);
        assert!(
            !root.exists(),
            "MP-11 partial native pool must be reclaimed"
        );
    }
    fn encode(headers: &[Value], segments: &[&[u8]]) -> Vec<u8> {
        let header = serde_json::to_vec(headers).unwrap();
        let mut bytes = (header.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(&header);
        for segment in segments {
            bytes.extend_from_slice(segment);
        }
        bytes
    }
    fn packet(spool: &DisplayPackets) -> (PathBuf, Value) {
        let name = format!("{}.json", format!("{:032x}", rand::random::<u128>()));
        let row = json!({"row":0,"sequence":1,"key":true,"length":1});
        let bytes = encode(&[row.clone()], &[&[0]]);
        let path = spool.root.join(&name);
        std::fs::write(&path, &bytes).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let reply = json!({"display_frame":{"kind":"stripes","stripes":[row],"native_packet":{"name":name,"length":bytes.len()}}});
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
    fn mp08_native_tile_packets_split_raw_segments_by_bound_lengths() {
        for case in ["ok", "trailing", "short", "header"] {
            let spool = DisplayPackets::create().unwrap();
            let (path, mut reply) = packet(&spool);
            let tiles = [
                json!({"x":0,"y":0,"width":4,"height":2,"format":"webp","length":3}),
                json!({"x":4,"y":0,"width":4,"height":2,"format":"webp","length":2}),
            ];
            let mut bytes = encode(&tiles, &[&[1, 2, 3], &[4, 5]]);
            match case {
                "trailing" => bytes.push(9),
                "short" => {
                    bytes.pop();
                }
                _ => {}
            }
            std::fs::write(&path, &bytes).unwrap();
            let mut frame_tiles = tiles.to_vec();
            if case == "header" {
                frame_tiles[1]["x"] = json!(8);
            }
            let descriptor =
                json!({"name":path.file_name().unwrap().to_str().unwrap(),"length":bytes.len()});
            reply["display_frame"] = json!({"kind":"tiles","tiles":frame_tiles,"moves":[[0,2,8,6,2]],"native_packet":descriptor});
            let result = spool.hydrate(&mut reply);
            assert!(!path.exists(), "{case}");
            assert_eq!(result.is_ok(), case == "ok", "{case}");
            if case == "ok" {
                assert_eq!(reply["display_frame"]["tiles"][0]["data_base64"], "AQID");
                assert_eq!(reply["display_frame"]["tiles"][1]["data_base64"], "BAU=");
                assert_eq!(reply["display_frame"]["moves"], json!([[0, 2, 8, 6, 2]]));
            }
        }
    }
    #[test]
    fn mp11_native_whole_video_packets_bind_geometry_codec_and_key_before_consumption() {
        for mismatch in [false, true] {
            let spool = DisplayPackets::create().unwrap();
            let (path, mut reply) = packet(&spool);
            let row =
                json!({"row":0,"y":0,"height":800,"codec":"avc1.420033","key":true,"length":1});
            let bytes = encode(&[row], &[&[0]]);
            std::fs::write(&path, &bytes).unwrap();
            let descriptor =
                json!({"name":path.file_name().unwrap().to_str().unwrap(),"length":bytes.len()});
            reply["display_frame"] = json!({"kind":"video","width":1280,"height":800,"codec":"avc1.420033","key":!mismatch,"native_packet":descriptor});
            let result = spool.hydrate(&mut reply);
            assert_eq!(result.is_err(), mismatch);
            assert!(!path.exists());
            if !mismatch {
                assert!(reply["display_frame"].get("native_packet").is_none());
                assert_eq!(reply["display_frame"]["data_base64"], "AA==");
            }
        }
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
