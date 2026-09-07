//! Local developer packaging only: no installation, kernel enrollment, package
//! execution, build scripts, dependency resolution, or network access.

mod cli;
mod fs;
mod keys;
#[cfg(test)]
mod tests;

use std::{collections::BTreeMap, path::Path};

use serde::Serialize;

use crate::{
    archive, json, Capabilities, ErrorCode, Limits, Manifest, PackageError, Publisher, Result,
    Runtime, RuntimeEngine, Ui, VerificationPolicy, APP_CONTRACT_VERSION, APP_SCHEMA,
    RESOURCE_POLICY, SUPPORTED_SDK_VERSION,
};

pub use cli::{run_cli, usage};
pub use keys::{keygen, read_publisher, KeygenReport, PublisherFile};

/// The shared Chariox CLI supplies its real kernel protocol constant. The
/// standalone command requires it explicitly; this crate never guesses a
/// kernel version or depends on the kernel implementation.
#[derive(Debug, Clone)]
pub struct ManifestOptions {
    pub app_id: String,
    pub version: String,
    pub publisher: Publisher,
    pub kernel_protocol: u32,
    pub runtime_entry: String,
    pub ui_entry: String,
    pub tools: Option<String>,
    pub events: Option<String>,
    pub actions: Option<String>,
    pub information_sets: Option<String>,
    pub capabilities: Capabilities,
}

impl ManifestOptions {
    pub fn new(
        app_id: String,
        version: String,
        publisher: Publisher,
        kernel_protocol: u32,
    ) -> Self {
        Self {
            app_id,
            version,
            publisher,
            kernel_protocol,
            runtime_entry: "runtime/main.js".to_owned(),
            ui_entry: "ui/index.html".to_owned(),
            tools: None,
            events: None,
            actions: None,
            information_sets: None,
            capabilities: Capabilities::default(),
        }
    }
}

pub fn generate_manifest(options: ManifestOptions, limits: &Limits) -> Result<Manifest> {
    let manifest = Manifest {
        schema: APP_SCHEMA.to_owned(),
        app_id: options.app_id,
        version: options.version,
        publisher: options.publisher,
        sdk_version: SUPPORTED_SDK_VERSION.to_owned(),
        app_contract_version: APP_CONTRACT_VERSION,
        min_kernel_protocol: options.kernel_protocol,
        resource_policy: RESOURCE_POLICY.to_owned(),
        runtime: Runtime {
            engine: RuntimeEngine::Node,
            entry: options.runtime_entry,
        },
        ui: Ui {
            entry: options.ui_entry,
        },
        tools: options.tools,
        events: options.events,
        actions: options.actions,
        information_sets: options.information_sets,
        capabilities: options.capabilities,
        migrations: None,
    };
    manifest.validate(limits)?;
    Ok(manifest)
}

pub fn write_manifest(path: &Path, manifest: &Manifest, limits: &Limits) -> Result<()> {
    manifest.validate(limits)?;
    let bytes = json::canonical(manifest)?;
    if bytes.len() > limits.max_metadata_bytes {
        return Err(PackageError::new(
            ErrorCode::ArchiveLimit,
            "manifest exceeds byte limit",
        ));
    }
    fs::write_new(path, &bytes)
}

pub fn read_manifest(path: &Path, limits: &Limits) -> Result<Manifest> {
    let bytes = fs::read(path, limits.max_metadata_bytes, false)?;
    let manifest: Manifest = json::read_json(
        &bytes,
        limits.max_metadata_bytes,
        limits,
        ErrorCode::InvalidManifest,
        false,
    )?;
    manifest.validate(limits)?;
    Ok(manifest)
}

/// Read a completed build directory into the existing deterministic pack API.
/// Empty directories count against the walk bound but create no archive entry.
pub fn read_bundle(path: &Path, limits: &Limits) -> Result<BTreeMap<String, Vec<u8>>> {
    read_bundle_open(&fs::Directory::open(path)?, limits, None)
}

fn read_bundle_open(
    root: &fs::Directory,
    limits: &Limits,
    signing_identity: Option<fs::Identity>,
) -> Result<BTreeMap<String, Vec<u8>>> {
    struct Walk<'a> {
        limits: &'a Limits,
        entries: usize,
        bytes: usize,
        files: BTreeMap<String, Vec<u8>>,
        signing_identity: Option<fs::Identity>,
    }
    impl Walk<'_> {
        fn visit(&mut self, directory: &fs::Directory, prefix: &str) -> Result<()> {
            for name in directory.names(self.limits.max_entries)? {
                self.entries = self
                    .entries
                    .checked_add(1)
                    .filter(|value| *value <= self.limits.max_entries)
                    .ok_or_else(|| {
                        PackageError::new(
                            ErrorCode::ArchiveLimit,
                            "bundle exceeds traversal entry limit",
                        )
                    })?;
                let local = name.to_str().map_err(|_| {
                    PackageError::new(
                        ErrorCode::InvalidPath,
                        "bundle filenames must be portable ASCII",
                    )
                })?;
                let path = if prefix.is_empty() {
                    local.to_owned()
                } else {
                    format!("{prefix}/{local}")
                };
                archive::validate_path(&path, self.limits)?;
                match directory.entry(&name)? {
                    fs::Entry::Directory(child) => self.visit(&child, &path)?,
                    fs::Entry::File(file) => {
                        if self.signing_identity == Some(fs::identity(&file)?) {
                            return Err(PackageError::new(
                                ErrorCode::InvalidPath,
                                "bundle contains the signing key inode",
                            ));
                        }
                        let remaining = self
                            .limits
                            .max_archive_bytes
                            .checked_sub(self.bytes)
                            .ok_or_else(|| {
                                PackageError::new(
                                    ErrorCode::ArchiveLimit,
                                    "bundle exceeds byte limit",
                                )
                            })?;
                        let bytes = fs::read_open(file, self.limits.max_file_bytes.min(remaining))?;
                        self.bytes += bytes.len();
                        self.files.insert(path, bytes);
                    }
                }
            }
            Ok(())
        }
    }
    let mut walk = Walk {
        limits,
        entries: 0,
        bytes: 0,
        files: BTreeMap::new(),
        signing_identity,
    };
    walk.visit(root, "")?;
    Ok(walk.files)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageReport {
    pub status: &'static str,
    pub manifest: Manifest,
    pub package_digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_count: Option<usize>,
}

pub fn pack_directory(
    bundle: &Path,
    manifest: &Manifest,
    private_key: &Path,
    output: &Path,
    kernel_protocol: u32,
    limits: &Limits,
) -> Result<PackageReport> {
    if kernel_protocol == 0 {
        return Err(PackageError::new(
            ErrorCode::InvalidArguments,
            "kernel protocol must be positive",
        ));
    }
    // Hold the actual selected objects from containment checks through reading
    // and publication. Reopening paths after a rename would change selection.
    let bundle = fs::Directory::open(bundle)?;
    let signing = keys::read_signing_key(private_key)?;
    let destination = fs::Destination::open(output, false)?;
    fs::outside_bundle(&bundle, &signing.directory)?;
    fs::outside_bundle(&bundle, &destination.directory)?;
    if manifest.publisher.key_id != keys::key_id(&signing.key.verifying_key()) {
        return Err(PackageError::new(
            ErrorCode::InvalidDeveloperKey,
            "manifest publisher key ID does not match the private signing key",
        ));
    }
    let files = read_bundle_open(&bundle, limits, Some(fs::identity(&signing.file)?))?;
    let bytes = crate::pack(manifest, &files, &signing.key, limits)?;
    let mut policy = VerificationPolicy::new(
        kernel_protocol,
        vec![crate::TrustedPublisher {
            publisher_id: manifest.publisher.id.clone(),
            key_id: manifest.publisher.key_id.clone(),
            public_key: signing.key.verifying_key(),
        }],
    );
    policy.limits = limits.clone();
    let verified = crate::verify(&bytes, &policy)?;
    let report = PackageReport {
        status: "packed-locally",
        manifest: verified.manifest().clone(),
        package_digest: verified.package_digest().to_owned(),
        file_count: Some(verified.files().count()),
    };
    // File bytes have passed the same verifier the installer will use. No
    // content has been extracted or executed, and output never replaces a file.
    fs::outside_bundle(&bundle, &destination.directory)?;
    destination.prepare(&bytes)?.publish()?;
    Ok(report)
}

pub fn validate_archive(
    path: &Path,
    publisher: &PublisherFile,
    kernel_protocol: u32,
    limits: &Limits,
) -> Result<PackageReport> {
    if kernel_protocol == 0 {
        return Err(PackageError::new(
            ErrorCode::InvalidArguments,
            "kernel protocol must be positive",
        ));
    }
    let bytes = fs::read(path, limits.max_archive_bytes, false)?;
    let mut policy = VerificationPolicy::new(kernel_protocol, vec![publisher.trusted_publisher()?]);
    policy.limits = limits.clone();
    let verified = crate::verify(&bytes, &policy)?;
    Ok(PackageReport {
        status: "verified-against-explicit-publisher-file",
        manifest: verified.manifest().clone(),
        package_digest: verified.package_digest().to_owned(),
        file_count: Some(verified.files().count()),
    })
}

pub fn inspect_archive(path: &Path, limits: &Limits) -> Result<PackageReport> {
    let bytes = fs::read(path, limits.max_archive_bytes, false)?;
    let identity = crate::inspect_untrusted(&bytes, limits)?;
    Ok(PackageReport {
        status: "untrusted-claims-only",
        manifest: identity.manifest,
        package_digest: identity.package_digest,
        file_count: None,
    })
}

/// Stable process exit contract for JSON CLI wrappers.
pub fn exit_code(error: &PackageError) -> i32 {
    match error.code {
        ErrorCode::InvalidArguments => 2,
        ErrorCode::Io => 4,
        _ => 3,
    }
}
