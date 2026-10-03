use super::super::model::SliceRecord;
use super::LocalDockerSliceOptions;
use crate::config::DEFAULT_LINUX_SLICE_DOCKER_IMAGE;
use sha2::{Digest, Sha256};

pub(super) fn selected_image(record: &SliceRecord, options: &LocalDockerSliceOptions) -> String {
    choose_image(
        &options.docker_image,
        options.extension_dockerfile.is_some(),
        options.saved_home_archive.is_some(),
        [
            &record.owner_kernel_id,
            &record.owner_machine_id,
            &record.id,
            &record.worker_kernel_ref,
        ],
    )
}

// Docker distribution/reference splitDockerDomain preserves domain case and
// canonicalizes only the exact legacy/default Hub spellings.
fn is_default_runtime_base(selected: &str) -> bool {
    let mut name = selected;
    if let Some((registry, remainder)) = name.split_once('/') {
        if registry == "docker.io" || registry == "index.docker.io" {
            name = remainder;
        }
    }
    name.strip_prefix("library/").unwrap_or(name) == DEFAULT_LINUX_SLICE_DOCKER_IMAGE
}

fn choose_image(selected: &str, extension: bool, saved: bool, identity: [&str; 4]) -> String {
    if !extension || saved || !is_default_runtime_base(selected) {
        return selected.to_string();
    }
    // Slice numeric IDs can be reused after deletion. The persisted worker
    // reference includes the fresh creation nonce and survives restarts.
    let mut digest = Sha256::new();
    digest.update(b"chariox-slice-extension-image-v1");
    for part in identity {
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part.as_bytes());
    }
    format!("chariox-slice-extension:{:x}", digest.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    const IDENTITY: [&str; 4] = ["home-a", "machine-a", "slice-1", "opaque-worker-creation-a"];

    #[test]
    fn extension_default_has_stable_unique_output_distinct_from_base() {
        let image = choose_image(DEFAULT_LINUX_SLICE_DOCKER_IMAGE, true, false, IDENTITY);
        assert!(image.starts_with("chariox-slice-extension:"));
        assert_ne!(image, DEFAULT_LINUX_SLICE_DOCKER_IMAGE);
        assert_eq!(
            image,
            choose_image(DEFAULT_LINUX_SLICE_DOCKER_IMAGE, true, false, IDENTITY)
        );
        for changed in [
            ["home-b", "machine-a", "slice-1", "opaque-worker-creation-a"],
            ["home-a", "machine-b", "slice-1", "opaque-worker-creation-a"],
            ["home-a", "machine-a", "slice-1", "opaque-worker-creation-b"],
        ] {
            assert_ne!(
                image,
                choose_image(DEFAULT_LINUX_SLICE_DOCKER_IMAGE, true, false, changed)
            );
        }
    }

    #[test]
    fn docker_hub_default_aliases_share_derived_output_in_both_placements() {
        let derived = choose_image(DEFAULT_LINUX_SLICE_DOCKER_IMAGE, true, false, IDENTITY);
        for prefix in [
            "",
            "library/",
            "docker.io/",
            "docker.io/library/",
            "index.docker.io/",
            "index.docker.io/library/",
        ] {
            let alias = format!("{prefix}{DEFAULT_LINUX_SLICE_DOCKER_IMAGE}");
            assert_eq!(choose_image(&alias, true, false, IDENTITY), derived);
            assert_eq!(choose_image(&alias, true, true, IDENTITY), alias);
        }
        for selected in [
            "chariox-slice-linux:custom",
            "registry.example:5000/team/slice:Tools_1",
            "docker.io/team/chariox-slice-linux:0.1.0",
            "DOCKER.IO/library/chariox-slice-linux:0.1.0",
        ] {
            assert_eq!(choose_image(selected, true, false, IDENTITY), selected);
        }
    }

    #[test]
    fn explicit_image_and_saved_state_keep_their_cache_identity() {
        assert_eq!(
            choose_image("chariox-custom:mine", true, false, IDENTITY),
            "chariox-custom:mine"
        );
        assert_eq!(
            choose_image("chariox-slice-state:mine", true, true, IDENTITY),
            "chariox-slice-state:mine"
        );
        assert_eq!(
            choose_image(DEFAULT_LINUX_SLICE_DOCKER_IMAGE, true, true, IDENTITY),
            DEFAULT_LINUX_SLICE_DOCKER_IMAGE
        );
        assert_eq!(
            choose_image(DEFAULT_LINUX_SLICE_DOCKER_IMAGE, false, false, IDENTITY),
            DEFAULT_LINUX_SLICE_DOCKER_IMAGE
        );
    }
}
