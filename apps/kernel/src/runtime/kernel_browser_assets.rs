//! MD-2: ship the shared controller assets with every native kernel, outside source checkouts.
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
const ASSETS: &[(&str, &[u8])] = &[
    (
        "kernel-browser-mirror-styles.mjs",
        include_bytes!("../../slice-linux-docker/docker/kernel-browser-mirror-styles.mjs"),
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
        "browser-controller-artifacts.mjs",
        include_bytes!("../../slice-linux-docker/docker/browser-controller-artifacts.mjs"),
    ),
    (
        "browser-controller-image.mjs",
        include_bytes!("../../slice-linux-docker/docker/browser-controller-image.mjs"),
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
    /// MP-08/MP-11: the materialized host must load; every relative module
    /// an embedded asset imports is embedded too.
    #[test]
    fn embedded_assets_include_every_relative_import() {
        let names = super::ASSETS
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>();
        for (name, bytes) in super::ASSETS {
            let text = String::from_utf8_lossy(bytes);
            for quote in ['"', '\''] {
                for part in text.split(&format!("{quote}./")).skip(1) {
                    let import = part.split(quote).next().unwrap_or_default();
                    if import.ends_with(".mjs") {
                        assert!(names.contains(&import), "{name} imports missing {import}");
                    }
                }
            }
        }
    }
}
