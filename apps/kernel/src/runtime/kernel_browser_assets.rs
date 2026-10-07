//! MD-2: ship the shared controller assets with every native kernel, outside source checkouts.
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
const ASSETS: &[(&str, &[u8])] = &[
    ("kernel-browser-codec-protection.py", include_bytes!("../../slice-linux-docker/docker/kernel-browser-codec-protection.py")),
    ("kernel-browser-native-worker.mjs", include_bytes!("../../slice-linux-docker/docker/kernel-browser-native-worker.mjs")),
    ("kernel-browser-native-credit.mjs", include_bytes!("../../slice-linux-docker/docker/kernel-browser-native-credit.mjs")),
    ("kernel-browser-openh264.py", include_bytes!("../../slice-linux-docker/docker/kernel-browser-openh264.py")),

    ("kernel-browser-stripes.py", include_bytes!("../../slice-linux-docker/docker/kernel-browser-stripes.py")),
    ("kernel-browser-shared-raster.mjs", include_bytes!("../../slice-linux-docker/docker/kernel-browser-shared-raster.mjs")),
    ("kernel-browser-geometry.mjs", include_bytes!("../../slice-linux-docker/docker/kernel-browser-geometry.mjs")),
    (
        "kernel-browser-native-pipe.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-native-pipe.mjs"),
    ),
    (
        "kernel-browser-owned-display.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-owned-display.mjs"),
    ),
    (
        "kernel-browser-native.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-native.mjs"),
    ),
    (
        "kernel-browser-xshm.py",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-xshm.py"),
    ),
    (
        "kernel-browser-raster-damage.py",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-raster-damage.py"),
    ),
    (
        "kernel-browser-motion.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-motion.mjs"),
    ),
    (
        "kernel-browser-tiles.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-tiles.mjs"),
    ),
    (
        "kernel-browser-refiner.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-refiner.mjs"),
    ),
    (
        "kernel-browser-pixel-worker.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-pixel-worker.mjs"),
    ),
    (
        "kernel-browser-webcodecs.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-webcodecs.mjs"),
    ),
    (
        "kernel-browser-compositor.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-compositor.mjs"),
    ),
    (
        "kernel-browser-sample-lane.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-sample-lane.mjs"),
    ),
    (
        "kernel-browser-mirror-local-fonts.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-mirror-local-fonts.mjs"),
    ),
    (
        "kernel-browser-mirror-protected-text.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-mirror-protected-text.mjs"),
    ),
    (
        "kernel-browser-mirror-css.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-mirror-css.mjs"),
    ),
    (
        "kernel-browser-mirror-custom-elements.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-mirror-custom-elements.mjs"),
    ),
    (
        "browser-controller-mirror-fonts.mjs",
        include_bytes!("../../slice-linux-docker/docker/browser-controller-mirror-fonts.mjs"),
    ),
    (
        "kernel-browser-mirror-styles.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-mirror-styles.mjs"),
    ),
    (
        "kernel-browser-mirror-wire.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-mirror-wire.mjs"),
    ),
    (
        "kernel-browser-mirror.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-mirror.mjs"),
    ),
    (
        "kernel-browser-mirror-observer.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-mirror-observer.mjs"),
    ),
    (
        "kernel-browser-mirror-resources.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-mirror-resources.mjs"),
    ),
    (
        "kernel-browser-cdp-pipe.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-cdp-pipe.mjs"),
    ),
    (
        "browser-controller-notes.mjs",
        include_bytes!("../../slice-linux-docker/docker/browser-controller-notes.mjs"),
    ),
    (
        "kernel-browser-display-capture.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-display-capture.mjs"),
    ),
    (
        "kernel-browser-timing.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-timing.mjs"),
    ),
    (
        "kernel-browser-display.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-display.mjs"),
    ),
    (
        "kernel-browser-encoder.py",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-encoder.py"),
    ),
    (
        "kernel-browser-region-protection.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-region-protection.mjs"),
    ),
    (
        "browser-observation-regions.mjs",
        include_bytes!("../../slice-linux-docker/docker/browser-observation-regions.mjs"),
    ),
    (
        "kernel-browser-pixels.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-pixels.mjs"),
    ),
    (
        "kernel-browser-linux.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-linux.mjs"),
    ),
    (
        "kernel-browser-macos.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-macos.mjs"),
    ),
    (
        "browser-app-restore.mjs",
        include_bytes!("../../slice-linux-docker/docker/browser-app-restore.mjs"),
    ),
    (
        "browser-controller-actions.mjs",
        include_bytes!("../../slice-linux-docker/docker/browser-controller-actions.mjs"),
    ),
    (
        "browser-controller-apps.mjs",
        include_bytes!("../../slice-linux-docker/docker/browser-controller-apps.mjs"),
    ),
    (
        "browser-controller-bar.mjs",
        include_bytes!("../../slice-linux-docker/docker/browser-controller-bar.mjs"),
    ),
    (
        "browser-controller-cdp.mjs",
        include_bytes!("../../slice-linux-docker/docker/browser-controller-cdp.mjs"),
    ),
    (
        "browser-controller-compatibility.mjs",
        include_bytes!("../../slice-linux-docker/docker/browser-controller-compatibility.mjs"),
    ),
    (
        "browser-controller-cookie-fence.mjs",
        include_bytes!("../../slice-linux-docker/docker/browser-controller-cookie-fence.mjs"),
    ),
    (
        "browser-controller-dialogs.mjs",
        include_bytes!("../../slice-linux-docker/docker/browser-controller-dialogs.mjs"),
    ),
    (
        "browser-controller-display.mjs",
        include_bytes!("../../slice-linux-docker/docker/browser-controller-display.mjs"),
    ),
    (
        "browser-controller-events.mjs",
        include_bytes!("../../slice-linux-docker/docker/browser-controller-events.mjs"),
    ),
    (
        "browser-controller-files.mjs",
        include_bytes!("../../slice-linux-docker/docker/browser-controller-files.mjs"),
    ),
    (
        "browser-controller-frames.mjs",
        include_bytes!("../../slice-linux-docker/docker/browser-controller-frames.mjs"),
    ),
    (
        "browser-controller-history.mjs",
        include_bytes!("../../slice-linux-docker/docker/browser-controller-history.mjs"),
    ),
    (
        "browser-controller-input.mjs",
        include_bytes!("../../slice-linux-docker/docker/browser-controller-input.mjs"),
    ),
    (
        "browser-controller-permissions.mjs",
        include_bytes!("../../slice-linux-docker/docker/browser-controller-permissions.mjs"),
    ),
    (
        "browser-controller-resources.mjs",
        include_bytes!("../../slice-linux-docker/docker/browser-controller-resources.mjs"),
    ),
    (
        "browser-controller-snapshot.mjs",
        include_bytes!("../../slice-linux-docker/docker/browser-controller-snapshot.mjs"),
    ),
    (
        "browser-controller-upload-staging.mjs",
        include_bytes!("../../slice-linux-docker/docker/browser-controller-upload-staging.mjs"),
    ),
    (
        "browser-controller.mjs",
        include_bytes!("../../slice-linux-docker/docker/browser-controller.mjs"),
    ),
    (
        "kernel-browser-input.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-input.mjs"),
    ),
    (
        "kernel-browser-refusal.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-refusal.mjs"),
    ),
    (
        "kernel-browser-host.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-host.mjs"),
    ),
    (
        "kernel-browser-process.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-process.mjs"),
    ),
];
pub(super) fn materialize(root: &Path) -> Result<PathBuf, String> {
    let mut digest = Sha256::new();
    for (name, bytes) in ASSETS {
        digest.update(name.as_bytes());
        digest.update(bytes);
    }
    let directory = root
        .join("controller-assets")
        .join(format!("{:x}", digest.finalize()));
    std::fs::create_dir_all(&directory)
        .map_err(|_| "MD-2: cannot create private browser controller assets")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "MD-2: cannot protect browser controller assets")?;
    }
    for (name, bytes) in ASSETS {
        let file = directory.join(name);
        if std::fs::read(&file).is_ok_and(|stored| stored == *bytes) {
            continue;
        }
        let temporary = directory.join(format!("{name}.new"));
        std::fs::write(&temporary, bytes)
            .map_err(|_| "MD-2: cannot write browser controller asset")?;
        std::fs::rename(temporary, file)
            .map_err(|_| "MD-2: cannot publish browser controller asset")?;
    }
    Ok(directory.join("kernel-browser-host.mjs"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::{Command, Stdio};

    #[test]
    fn md900_default_materialized_controller_loads_without_source_override() {
        let root =
            std::env::temp_dir().join(format!("chariox-assets-{:032x}", rand::random::<u128>()));
        let entry = materialize(&root).unwrap();
        let mut child = Command::new("node")
            .args([
                entry.as_os_str(),
                std::ffi::OsStr::new("stdio"),
                root.as_os_str(),
            ])
            .env_clear()
            .current_dir(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Node is required to verify the default embedded controller");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"{\"id\":1,\"method\":\"health\",\"params\":{}}\n")
            .unwrap();
        let result = child.wait_with_output().unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let health: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(health["ok"], true);
        assert_eq!(health["result"]["state"], "ready");
    }
}
