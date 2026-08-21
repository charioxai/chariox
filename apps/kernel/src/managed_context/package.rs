use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::development::MAX_PACKAGE_BYTES as MAX_DEVELOPMENT_BYTES;
use super::development::{
    import_development_context_with_publication, recover_development_context_publication,
    DevelopmentContextImportRequest, DevelopmentContextPublicationReceipt,
};
use super::kernel::{
    configured_managed_kernel_context_paths, import_kernel_context, KernelContextImportReceipt,
    KernelContextImportRequest, KernelContextSnapshot,
};
use crate::error::DaemonError;
use crate::secret::TransferredVaultSourceBinding;

const PACKAGE_SCHEMA_VERSION: u32 = 1;
const PACKAGE_MAGIC: &[u8; 16] = b"CHARIOXCTXPKG1\r\n";
const PACKAGE_TRAILER: &[u8; 16] = b"CHARIOXCTXEND1\r\n";
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const KERNEL_CONTEXT_WRAPPER_BYTES: u64 = 64 * 1024;
const MAX_KERNEL_CONTEXT_BYTES: u64 =
    super::kernel::MAX_KERNEL_CONTEXT_SNAPSHOT_BYTES as u64 + KERNEL_CONTEXT_WRAPPER_BYTES;
const PACKAGE_OVERHEAD_BYTES: u64 = MAX_MANIFEST_BYTES as u64 + 64;
pub(crate) const MAX_MANAGED_CONTEXT_PACKAGE_BYTES: u64 =
    MAX_DEVELOPMENT_BYTES + MAX_KERNEL_CONTEXT_BYTES + PACKAGE_OVERHEAD_BYTES;

#[derive(Clone, PartialEq)]
pub enum ManagedContextPackageKernel {
    Empty,
    FromKernel(KernelContextSnapshot),
}

impl std::fmt::Debug for ManagedContextPackageKernel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("Empty"),
            Self::FromKernel(snapshot) => {
                formatter.debug_tuple("FromKernel").field(snapshot).finish()
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ManagedContextPackageExportRequest {
    pub context_id: String,
    pub project_id: String,
    pub target_environment_id: String,
    pub source_kernel_id: String,
    pub source_key_thumbprint: String,
    pub target_kernel_id: String,
    pub target_key_thumbprint: String,
    pub development_archive_path: PathBuf,
    pub development_archive_sha256: String,
    pub kernel_context: ManagedContextPackageKernel,
    pub package_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedContextPackageExportResult {
    pub package_path: PathBuf,
    pub package_sha256: String,
    pub package_size_bytes: u64,
    pub development_archive_sha256: String,
    pub kernel_context_snapshot_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedContextPackageBinding {
    pub context_id: String,
    pub project_id: String,
    pub target_environment_id: String,
    pub source_kernel_id: String,
    pub source_key_thumbprint: String,
    pub target_kernel_id: String,
    pub target_key_thumbprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedContextPackageImportRequest {
    pub package_path: PathBuf,
    pub expected_package_sha256: String,
    pub expected_binding: ManagedContextPackageBinding,
}

#[derive(Clone, PartialEq)]
pub(crate) struct ManagedContextPackageApplicationRequest {
    pub transfer_id: String,
    pub package_path: PathBuf,
    pub expected_package_sha256: String,
    pub expected_binding: ManagedContextPackageBinding,
    pub development_destination_root: PathBuf,
    pub target_private_key: String,
}

impl std::fmt::Debug for ManagedContextPackageApplicationRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedContextPackageApplicationRequest")
            .field("transfer_id", &self.transfer_id)
            .field("package_path", &self.package_path)
            .field("expected_package_sha256", &self.expected_package_sha256)
            .field("expected_binding", &self.expected_binding)
            .field(
                "development_destination_root",
                &self.development_destination_root,
            )
            .field("target_private_key", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManagedContextPackageImportReceipt {
    pub schema_version: u32,
    pub transfer_id: String,
    pub package_sha256: String,
    pub project_id: String,
    pub development: DevelopmentContextPublicationReceipt,
    pub kernel_context: ManagedContextImportedKernelContext,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ManagedContextImportedKernelContext {
    Empty,
    FromKernel { receipt: KernelContextImportReceipt },
}

#[derive(Debug)]
pub(crate) struct ExtractedManagedContextPackage {
    pub development_archive_path: PathBuf,
    pub development_archive_sha256: String,
    pub kernel_context: ManagedContextPackageKernel,
    cleanup: PackageExtractionCleanup,
}

impl ExtractedManagedContextPackage {
    pub(crate) fn component_root(&self) -> &Path {
        &self.cleanup.root
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManagedContextPackageManifest {
    schema_version: u32,
    context_id: String,
    project_id: String,
    target_environment_id: String,
    source_kernel_id: String,
    source_key_thumbprint: String,
    target_kernel_id: String,
    target_key_thumbprint: String,
    development: PackageComponentManifest,
    kernel_context: KernelContextComponentManifest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PackageComponentManifest {
    size_bytes: u64,
    sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum KernelContextComponentManifest {
    Empty,
    FromKernel {
        size_bytes: u64,
        sha256: String,
        snapshot_sha256: String,
    },
}

pub fn export_managed_context_package(
    request: ManagedContextPackageExportRequest,
) -> Result<ManagedContextPackageExportResult, DaemonError> {
    let binding = ManagedContextPackageBinding {
        context_id: request.context_id,
        project_id: request.project_id,
        target_environment_id: request.target_environment_id,
        source_kernel_id: request.source_kernel_id,
        source_key_thumbprint: request.source_key_thumbprint,
        target_kernel_id: request.target_kernel_id,
        target_key_thumbprint: request.target_key_thumbprint,
    };
    validate_binding(&binding)?;
    validate_sha256(
        &request.development_archive_sha256,
        "development archive digest",
    )?;
    validate_output_path(&request.package_path)?;

    let mut development = open_regular_file_no_follow(&request.development_archive_path)?;
    let development_size = development
        .metadata()
        .map_err(|error| package_io_error("inspect development archive", error))?
        .len();
    if development_size == 0 || development_size > MAX_DEVELOPMENT_BYTES {
        return Err(package_error("development archive size is invalid"));
    }
    let actual_development_sha256 = hash_reader(&mut development, MAX_DEVELOPMENT_BYTES)?;
    if actual_development_sha256 != request.development_archive_sha256.to_ascii_lowercase() {
        return Err(package_error(
            "development archive digest does not match the requested digest",
        ));
    }
    development
        .seek(SeekFrom::Start(0))
        .map_err(|error| package_io_error("rewind development archive", error))?;

    let (kernel_bytes, kernel_manifest, kernel_snapshot_sha256) = match request.kernel_context {
        ManagedContextPackageKernel::Empty => (None, KernelContextComponentManifest::Empty, None),
        ManagedContextPackageKernel::FromKernel(snapshot) => {
            validate_snapshot_binding(&snapshot, &binding)?;
            let bytes = serialize_bounded_json(&snapshot, MAX_KERNEL_CONTEXT_BYTES as usize)?;
            let sha256 = sha256_bytes(&bytes);
            let snapshot_sha256 = snapshot.snapshot_sha256.clone();
            let manifest = KernelContextComponentManifest::FromKernel {
                size_bytes: bytes.len() as u64,
                sha256,
                snapshot_sha256: snapshot_sha256.clone(),
            };
            (Some(bytes), manifest, Some(snapshot_sha256))
        }
    };
    let manifest = ManagedContextPackageManifest {
        schema_version: PACKAGE_SCHEMA_VERSION,
        context_id: binding.context_id,
        project_id: binding.project_id,
        target_environment_id: binding.target_environment_id,
        source_kernel_id: binding.source_kernel_id,
        source_key_thumbprint: binding.source_key_thumbprint,
        target_kernel_id: binding.target_kernel_id,
        target_key_thumbprint: binding.target_key_thumbprint,
        development: PackageComponentManifest {
            size_bytes: development_size,
            sha256: actual_development_sha256.clone(),
        },
        kernel_context: kernel_manifest,
    };
    let manifest_bytes = serialize_bounded_json(&manifest, MAX_MANIFEST_BYTES)?;

    let parent = request
        .package_path
        .parent()
        .ok_or_else(|| package_error("managed context package must have a parent directory"))?;
    fs::create_dir_all(parent)
        .map_err(|error| package_io_error("create managed context package parent", error))?;
    let canonical_parent = fs::canonicalize(parent)
        .map_err(|error| package_io_error("resolve managed context package parent", error))?;
    let file_name = request
        .package_path
        .file_name()
        .ok_or_else(|| package_error("managed context package must have a file name"))?;
    let destination = canonical_parent.join(file_name);
    if destination.exists() {
        return Err(package_error("managed context package already exists"));
    }
    let temporary = canonical_parent.join(format!(
        ".tmp-chariox-context-package-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    let file = create_private_file(&temporary)?;
    let mut cleanup = PackageFileCleanup::new(temporary.clone());
    let mut writer = HashingWriter::new(file, MAX_MANAGED_CONTEXT_PACKAGE_BYTES);
    writer
        .write_all(PACKAGE_MAGIC)
        .map_err(package_write_error)?;
    writer
        .write_all(&(manifest_bytes.len() as u32).to_be_bytes())
        .map_err(package_write_error)?;
    writer
        .write_all(&manifest_bytes)
        .map_err(package_write_error)?;
    let copied_development = copy_component(
        &mut development,
        &mut writer,
        development_size,
        MAX_DEVELOPMENT_BYTES,
    )?;
    if copied_development != actual_development_sha256 {
        return Err(package_error(
            "development archive changed while composing the managed context package",
        ));
    }
    if let Some(bytes) = kernel_bytes.as_deref() {
        writer.write_all(bytes).map_err(package_write_error)?;
    }
    writer
        .write_all(PACKAGE_TRAILER)
        .map_err(package_write_error)?;
    let (mut file, package_size_bytes, package_sha256) = writer.finish();
    file.flush()
        .and_then(|_| file.sync_all())
        .map_err(|error| package_io_error("sync managed context package", error))?;
    super::development::publish_archive_no_clobber(&temporary, &destination, &canonical_parent)?;
    fs::remove_file(&temporary)
        .map_err(|error| package_io_error("remove managed context package staging", error))?;
    sync_directory(&canonical_parent)?;
    cleanup.keep();
    Ok(ManagedContextPackageExportResult {
        package_path: destination,
        package_sha256,
        package_size_bytes,
        development_archive_sha256: actual_development_sha256,
        kernel_context_snapshot_sha256: kernel_snapshot_sha256,
    })
}

pub(crate) fn apply_managed_context_package(
    request: ManagedContextPackageApplicationRequest,
) -> Result<ManagedContextPackageImportReceipt, DaemonError> {
    let extracted = extract_managed_context_package(ManagedContextPackageImportRequest {
        package_path: request.package_path,
        expected_package_sha256: request.expected_package_sha256.clone(),
        expected_binding: request.expected_binding.clone(),
    })?;
    let development_request = DevelopmentContextImportRequest {
        archive_path: extracted.development_archive_path.clone(),
        expected_archive_sha256: extracted.development_archive_sha256.clone(),
        expected_project_id: request.expected_binding.project_id.clone(),
        destination_root: request.development_destination_root,
    };
    let development = match recover_development_context_publication(
        &development_request,
        &request.transfer_id,
    )? {
        Some(receipt) => receipt,
        None => import_development_context_with_publication(
            development_request,
            request.transfer_id.clone(),
        )?,
    };
    let kernel_context = match &extracted.kernel_context {
        ManagedContextPackageKernel::Empty => ManagedContextImportedKernelContext::Empty,
        ManagedContextPackageKernel::FromKernel(snapshot) => {
            let (capability_root, vault_path) = configured_managed_kernel_context_paths()?;
            let receipt = import_kernel_context(KernelContextImportRequest {
                snapshot: snapshot.clone(),
                expected_source: TransferredVaultSourceBinding {
                    context_id: request.expected_binding.context_id.clone(),
                    source_kernel_id: request.expected_binding.source_kernel_id.clone(),
                    source_key_thumbprint: request.expected_binding.source_key_thumbprint.clone(),
                },
                target_kernel_id: request.expected_binding.target_kernel_id.clone(),
                target_private_key: request.target_private_key,
                capability_root,
                vault_path,
            })?;
            ManagedContextImportedKernelContext::FromKernel { receipt }
        }
    };
    Ok(ManagedContextPackageImportReceipt {
        schema_version: PACKAGE_SCHEMA_VERSION,
        transfer_id: request.transfer_id,
        package_sha256: request.expected_package_sha256,
        project_id: request.expected_binding.project_id,
        development,
        kernel_context,
    })
}

pub(crate) fn extract_managed_context_package(
    request: ManagedContextPackageImportRequest,
) -> Result<ExtractedManagedContextPackage, DaemonError> {
    validate_sha256(
        &request.expected_package_sha256,
        "managed context package digest",
    )?;
    validate_binding(&request.expected_binding)?;
    let mut package = open_regular_file_no_follow(&request.package_path)?;
    let package_size = package
        .metadata()
        .map_err(|error| package_io_error("inspect managed context package", error))?
        .len();
    if package_size == 0 || package_size > MAX_MANAGED_CONTEXT_PACKAGE_BYTES {
        return Err(package_error("managed context package size is invalid"));
    }

    let component_root = component_root_for_package(&request.package_path)?;
    cleanup_component_root(&component_root)?;
    create_private_directory(&component_root)?;
    let cleanup = PackageExtractionCleanup {
        root: component_root.clone(),
    };
    let mut reader = HashingReader::new(&mut package, MAX_MANAGED_CONTEXT_PACKAGE_BYTES);
    let mut magic = [0_u8; 16];
    reader.read_exact(&mut magic).map_err(package_read_error)?;
    if &magic != PACKAGE_MAGIC {
        return Err(package_error("managed context package magic is invalid"));
    }
    let mut manifest_length = [0_u8; 4];
    reader
        .read_exact(&mut manifest_length)
        .map_err(package_read_error)?;
    let manifest_length = u32::from_be_bytes(manifest_length) as usize;
    if manifest_length == 0 || manifest_length > MAX_MANIFEST_BYTES {
        return Err(package_error(
            "managed context package manifest size is invalid",
        ));
    }
    let mut manifest_bytes = vec![0_u8; manifest_length];
    reader
        .read_exact(&mut manifest_bytes)
        .map_err(package_read_error)?;
    let manifest = serde_json::from_slice::<ManagedContextPackageManifest>(&manifest_bytes)
        .map_err(|_| package_error("managed context package manifest is invalid"))?;
    validate_manifest(&manifest, &request.expected_binding)?;

    let development_archive_path = component_root.join("development.tar.gz");
    let development_file = create_private_file(&development_archive_path)?;
    let development_archive_sha256 = copy_component_to_file(
        &mut reader,
        development_file,
        manifest.development.size_bytes,
        MAX_DEVELOPMENT_BYTES,
    )?;
    if development_archive_sha256 != manifest.development.sha256 {
        return Err(package_error(
            "managed context development component digest does not match",
        ));
    }

    let kernel_context = match &manifest.kernel_context {
        KernelContextComponentManifest::Empty => ManagedContextPackageKernel::Empty,
        KernelContextComponentManifest::FromKernel {
            size_bytes,
            sha256,
            snapshot_sha256,
        } => {
            let path = component_root.join("kernel-context.json");
            let file = create_private_file(&path)?;
            let actual_sha256 =
                copy_component_to_file(&mut reader, file, *size_bytes, MAX_KERNEL_CONTEXT_BYTES)?;
            if &actual_sha256 != sha256 {
                return Err(package_error(
                    "managed context kernel component digest does not match",
                ));
            }
            let file = open_regular_file_no_follow(&path)?;
            let snapshot = serde_json::from_reader::<_, KernelContextSnapshot>(file)
                .map_err(|_| package_error("managed context kernel component is invalid"))?;
            if &snapshot.snapshot_sha256 != snapshot_sha256 {
                return Err(package_error(
                    "managed context kernel snapshot digest does not match its manifest",
                ));
            }
            validate_snapshot_binding(&snapshot, &request.expected_binding)?;
            ManagedContextPackageKernel::FromKernel(snapshot)
        }
    };
    let mut trailer = [0_u8; 16];
    reader
        .read_exact(&mut trailer)
        .map_err(package_read_error)?;
    if &trailer != PACKAGE_TRAILER {
        return Err(package_error("managed context package trailer is invalid"));
    }
    let mut trailing = [0_u8; 1];
    if reader.read(&mut trailing).map_err(package_read_error)? != 0 {
        return Err(package_error("managed context package has trailing bytes"));
    }
    let (read_bytes, package_sha256) = reader.finish();
    if read_bytes != package_size
        || package_sha256 != request.expected_package_sha256.to_ascii_lowercase()
    {
        return Err(package_error(
            "managed context package digest does not match the expected digest",
        ));
    }
    sync_directory(&component_root)?;
    Ok(ExtractedManagedContextPackage {
        development_archive_path,
        development_archive_sha256,
        kernel_context,
        cleanup,
    })
}

pub(crate) fn cleanup_package_components(package_path: &Path) -> Result<(), DaemonError> {
    cleanup_component_root(&component_root_for_package(package_path)?)
}

fn validate_manifest(
    manifest: &ManagedContextPackageManifest,
    expected: &ManagedContextPackageBinding,
) -> Result<(), DaemonError> {
    if manifest.schema_version != PACKAGE_SCHEMA_VERSION {
        return Err(package_error("unsupported managed context package version"));
    }
    let actual = ManagedContextPackageBinding {
        context_id: manifest.context_id.clone(),
        project_id: manifest.project_id.clone(),
        target_environment_id: manifest.target_environment_id.clone(),
        source_kernel_id: manifest.source_kernel_id.clone(),
        source_key_thumbprint: manifest.source_key_thumbprint.clone(),
        target_kernel_id: manifest.target_kernel_id.clone(),
        target_key_thumbprint: manifest.target_key_thumbprint.clone(),
    };
    validate_binding(&actual)?;
    if &actual != expected {
        return Err(package_error(
            "managed context package source or target binding does not match",
        ));
    }
    if manifest.development.size_bytes == 0
        || manifest.development.size_bytes > MAX_DEVELOPMENT_BYTES
    {
        return Err(package_error(
            "managed context development component size is invalid",
        ));
    }
    validate_sha256(&manifest.development.sha256, "development component digest")?;
    match &manifest.kernel_context {
        KernelContextComponentManifest::Empty => Ok(()),
        KernelContextComponentManifest::FromKernel {
            size_bytes,
            sha256,
            snapshot_sha256,
        } => {
            if *size_bytes == 0 || *size_bytes > MAX_KERNEL_CONTEXT_BYTES {
                return Err(package_error(
                    "managed context kernel component size is invalid",
                ));
            }
            validate_sha256(sha256, "kernel component digest")?;
            validate_sha256(snapshot_sha256, "kernel snapshot digest")
        }
    }
}

fn validate_snapshot_binding(
    snapshot: &KernelContextSnapshot,
    binding: &ManagedContextPackageBinding,
) -> Result<(), DaemonError> {
    let payload = &snapshot.payload;
    if payload.context_id != binding.context_id
        || payload.source_kernel_id != binding.source_kernel_id
        || payload.source_key_thumbprint != binding.source_key_thumbprint
        || payload.target_kernel_id != binding.target_kernel_id
        || payload.target_key_thumbprint != binding.target_key_thumbprint
    {
        return Err(package_error(
            "kernel context snapshot does not match the package binding",
        ));
    }
    Ok(())
}

fn validate_binding(binding: &ManagedContextPackageBinding) -> Result<(), DaemonError> {
    for (label, value) in [
        ("context id", binding.context_id.as_str()),
        ("project id", binding.project_id.as_str()),
        (
            "target environment id",
            binding.target_environment_id.as_str(),
        ),
        ("source kernel id", binding.source_kernel_id.as_str()),
        ("target kernel id", binding.target_kernel_id.as_str()),
    ] {
        validate_identifier(value, label)?;
    }
    validate_sha256(&binding.source_key_thumbprint, "source key thumbprint")?;
    validate_sha256(&binding.target_key_thumbprint, "target key thumbprint")
}

fn validate_identifier(value: &str, label: &str) -> Result<(), DaemonError> {
    if value.is_empty()
        || value.len() > 4096
        || value.chars().any(char::is_control)
        || value.contains(['/', '\\'])
    {
        return Err(package_error(format!("managed context {label} is invalid")));
    }
    Ok(())
}

fn validate_sha256(value: &str, label: &str) -> Result<(), DaemonError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(package_error(format!("managed context {label} is invalid")));
    }
    Ok(())
}

fn validate_output_path(path: &Path) -> Result<(), DaemonError> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
        || path.file_name().is_none()
    {
        return Err(package_error("managed context package path is invalid"));
    }
    Ok(())
}

fn component_root_for_package(path: &Path) -> Result<PathBuf, DaemonError> {
    let parent = path
        .parent()
        .ok_or_else(|| package_error("managed context package has no parent"))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| package_error("managed context package name is invalid"))?;
    if name.is_empty() || name.len() > 200 || name.contains(['/', '\\']) {
        return Err(package_error("managed context package name is invalid"));
    }
    Ok(parent.join(format!("{name}.components")))
}

fn cleanup_component_root(path: &Path) -> Result<(), DaemonError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(package_io_error("inspect package component staging", error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(package_error(
            "managed context package component staging is not a real directory",
        ));
    }
    fs::remove_dir_all(path)
        .map_err(|error| package_io_error("remove package component staging", error))?;
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

fn create_private_directory(path: &Path) -> Result<(), DaemonError> {
    fs::create_dir(path)
        .map_err(|error| package_io_error("create package component staging", error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| package_io_error("secure package component staging", error))?;
    }
    Ok(())
}

fn create_private_file(path: &Path) -> Result<File, DaemonError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|error| package_io_error("create managed context package file", error))
}

fn open_regular_file_no_follow(path: &Path) -> Result<File, DaemonError> {
    let mut options = OpenOptions::new();
    options.read(true);
    configure_no_follow(&mut options);
    let file = options
        .open(path)
        .map_err(|error| package_io_error("open managed context package file", error))?;
    let metadata = file
        .metadata()
        .map_err(|error| package_io_error("inspect managed context package file", error))?;
    if !metadata.is_file() {
        return Err(package_error(
            "managed context package input must be a regular file",
        ));
    }
    Ok(file)
}

fn configure_no_follow(options: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
}

fn serialize_bounded_json<T: Serialize>(
    value: &T,
    maximum_bytes: usize,
) -> Result<Vec<u8>, DaemonError> {
    let mut writer = BoundedVecWriter {
        bytes: Vec::new(),
        maximum_bytes,
    };
    serde_json::to_writer(&mut writer, value)
        .map_err(|error| package_error(format!("serialize managed context package: {error}")))?;
    Ok(writer.bytes)
}

fn hash_reader(file: &mut File, maximum_bytes: u64) -> Result<String, DaemonError> {
    let mut hasher = Sha256::new();
    let mut read_bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| package_io_error("read managed context component", error))?;
        if read == 0 {
            break;
        }
        read_bytes = read_bytes.saturating_add(read as u64);
        if read_bytes > maximum_bytes {
            return Err(package_error(
                "managed context component exceeds its size limit",
            ));
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn copy_component<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    size_bytes: u64,
    maximum_bytes: u64,
) -> Result<String, DaemonError> {
    if size_bytes == 0 || size_bytes > maximum_bytes {
        return Err(package_error("managed context component size is invalid"));
    }
    let mut remaining = size_bytes;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    while remaining > 0 {
        let requested = usize::try_from(remaining.min(buffer.len() as u64))
            .map_err(|_| package_error("managed context component size is invalid"))?;
        let read = reader
            .read(&mut buffer[..requested])
            .map_err(package_read_error)?;
        if read == 0 {
            return Err(package_error("managed context component is truncated"));
        }
        writer
            .write_all(&buffer[..read])
            .map_err(package_write_error)?;
        hasher.update(&buffer[..read]);
        remaining -= read as u64;
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn copy_component_to_file<R: Read>(
    reader: &mut R,
    mut file: File,
    size_bytes: u64,
    maximum_bytes: u64,
) -> Result<String, DaemonError> {
    let sha256 = copy_component(reader, &mut file, size_bytes, maximum_bytes)?;
    file.flush()
        .and_then(|_| file.sync_all())
        .map_err(|error| package_io_error("sync managed context component", error))?;
    Ok(sha256)
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn sync_directory(path: &Path) -> Result<(), DaemonError> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| package_io_error("sync managed context package directory", error))?;
    Ok(())
}

struct BoundedVecWriter {
    bytes: Vec<u8>,
    maximum_bytes: usize,
}

impl Write for BoundedVecWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.maximum_bytes.saturating_sub(self.bytes.len()) {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "managed context JSON exceeds its size limit",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct HashingWriter {
    file: File,
    hasher: Sha256,
    bytes: u64,
    maximum_bytes: u64,
}

impl HashingWriter {
    fn new(file: File, maximum_bytes: u64) -> Self {
        Self {
            file,
            hasher: Sha256::new(),
            bytes: 0,
            maximum_bytes,
        }
    }

    fn finish(self) -> (File, u64, String) {
        (
            self.file,
            self.bytes,
            format!("{:x}", self.hasher.finalize()),
        )
    }
}

impl Write for HashingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() as u64 > self.maximum_bytes.saturating_sub(self.bytes) {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "managed context package exceeds its size limit",
            ));
        }
        let written = self.file.write(bytes)?;
        self.hasher.update(&bytes[..written]);
        self.bytes += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

struct HashingReader<'a> {
    file: &'a mut File,
    hasher: Sha256,
    bytes: u64,
    maximum_bytes: u64,
}

impl<'a> HashingReader<'a> {
    fn new(file: &'a mut File, maximum_bytes: u64) -> Self {
        Self {
            file,
            hasher: Sha256::new(),
            bytes: 0,
            maximum_bytes,
        }
    }

    fn finish(self) -> (u64, String) {
        (self.bytes, format!("{:x}", self.hasher.finalize()))
    }
}

impl Read for HashingReader<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let available = self.maximum_bytes.saturating_sub(self.bytes);
        if available == 0 && !bytes.is_empty() {
            let mut trailing = [0_u8; 1];
            return match self.file.read(&mut trailing)? {
                0 => Ok(0),
                _ => Err(io::Error::new(
                    io::ErrorKind::FileTooLarge,
                    "managed context package exceeds its size limit",
                )),
            };
        }
        let limit = usize::try_from(available.min(bytes.len() as u64)).unwrap_or(bytes.len());
        let read = self.file.read(&mut bytes[..limit])?;
        self.hasher.update(&bytes[..read]);
        self.bytes += read as u64;
        Ok(read)
    }
}

#[derive(Debug)]
struct PackageExtractionCleanup {
    root: PathBuf,
}

impl Drop for PackageExtractionCleanup {
    fn drop(&mut self) {
        let _ = cleanup_component_root(&self.root);
    }
}

struct PackageFileCleanup {
    path: PathBuf,
    remove: bool,
}

impl PackageFileCleanup {
    fn new(path: PathBuf) -> Self {
        Self { path, remove: true }
    }

    fn keep(&mut self) {
        self.remove = false;
    }
}

impl Drop for PackageFileCleanup {
    fn drop(&mut self) {
        if self.remove {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn package_read_error(error: io::Error) -> DaemonError {
    if matches!(
        error.kind(),
        io::ErrorKind::UnexpectedEof | io::ErrorKind::InvalidData | io::ErrorKind::FileTooLarge
    ) {
        package_error("managed context package is malformed or truncated")
    } else {
        package_io_error("read managed context package", error)
    }
}

fn package_write_error(error: io::Error) -> DaemonError {
    if matches!(error.kind(), io::ErrorKind::FileTooLarge) {
        package_error("managed context package exceeds its size limit")
    } else {
        package_io_error("write managed context package", error)
    }
}

fn package_error(message: impl Into<String>) -> DaemonError {
    DaemonError::ManagedContext {
        code: "invalid_managed_context",
        operation: "managed context package",
        message: message.into(),
        retryable: false,
    }
}

fn package_io_error(operation: &'static str, error: io::Error) -> DaemonError {
    DaemonError::ManagedContext {
        code: "managed_context_unavailable",
        operation,
        message: error.to_string(),
        retryable: true,
    }
}

#[cfg(test)]
mod tests;
