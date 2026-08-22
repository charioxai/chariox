use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::development::MAX_PACKAGE_BYTES as MAX_DEVELOPMENT_BYTES;
use super::development::{
    import_development_context_with_publication, recover_development_context_publication,
    DevelopmentContextImportRequest, DevelopmentContextPublicationReceipt,
    DevelopmentRepositoryRole, DevelopmentSourceRepositoryBinding, MAX_PUBLICATION_RECEIPT_BYTES,
};
use super::kernel::{
    cleanup_kernel_context_import, configured_managed_kernel_context_paths, import_kernel_context,
    KernelContextImportReceipt, KernelContextImportRequest, KernelContextSnapshot,
};
use super::scm::{
    materialize_git_credentials, receipt_for_materialization, rollback_git_credentials,
    validate_materializations as validate_git_credential_materializations,
    validate_selection as validate_git_credential_selection, GitCredentialCommandContext,
    GitCredentialMaterialization, ManagedContextGitCredentialReceipt,
};
use crate::account_profile::{
    provider_account_materialization_sha256, ManagedContextProviderAccountReceipt,
    ProviderAccountMaterialization, ProviderAccountProfileRegistry,
};
use crate::error::DaemonError;
use crate::secret::TransferredVaultSourceBinding;

const PACKAGE_SCHEMA_VERSION: u32 = 4;
const LEGACY_PACKAGE_SCHEMA_VERSIONS: [u32; 2] = [2, 3];
const PACKAGE_MAGIC: &[u8; 16] = b"CHARIOXCTXPKG1\r\n";
const PACKAGE_TRAILER: &[u8; 16] = b"CHARIOXCTXEND1\r\n";
const MAX_PLAN_BINDING_BYTES: usize = 72 * 1024;
const MAX_MANIFEST_BYTES: usize = MAX_PLAN_BINDING_BYTES + 24 * 1024;
const KERNEL_CONTEXT_WRAPPER_BYTES: u64 = 64 * 1024;
const MAX_KERNEL_CONTEXT_BYTES: u64 =
    super::kernel::MAX_KERNEL_CONTEXT_SNAPSHOT_BYTES as u64 + KERNEL_CONTEXT_WRAPPER_BYTES;
pub(crate) const MAX_PROVIDER_ACCOUNT_COMPONENT_BYTES: u64 = 24 * 1024 * 1024;
pub(crate) const MAX_GIT_CREDENTIAL_COMPONENT_BYTES: u64 = 1024 * 1024;
const PACKAGE_OVERHEAD_BYTES: u64 = MAX_MANIFEST_BYTES as u64 + 64;
pub(crate) const MAX_MANAGED_CONTEXT_PACKAGE_BYTES: u64 = MAX_DEVELOPMENT_BYTES
    + MAX_KERNEL_CONTEXT_BYTES
    + MAX_PROVIDER_ACCOUNT_COMPONENT_BYTES
    + MAX_GIT_CREDENTIAL_COMPONENT_BYTES
    + PACKAGE_OVERHEAD_BYTES;

#[derive(Clone, PartialEq)]
pub enum ManagedContextPackageKernel {
    Empty,
    FromKernel(KernelContextSnapshot),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagedContextPackageDevelopment {
    Empty,
    FromSource {
        archive_path: PathBuf,
        archive_sha256: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ManagedContextPackageProviderAccounts {
    None,
    Selected {
        materializations: Vec<ProviderAccountMaterialization>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ManagedContextPackageGitCredentials {
    None,
    Selected {
        materializations: Vec<GitCredentialMaterialization>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedContextPlanBinding {
    pub context_id: String,
    pub plan_digest: String,
    pub kernel_context: ManagedContextKernelSelection,
    pub development: ManagedContextDevelopmentSelection,
    #[serde(default)]
    pub provider_accounts: ManagedContextProviderAccountSelection,
    #[serde(default)]
    pub git_credentials: ManagedContextGitCredentialSelection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedContextKernelSelection {
    Empty,
    SourceKernel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ManagedContextDevelopmentSelection {
    Empty,
    SourceProject {
        project_id: String,
        repositories: Vec<DevelopmentSourceRepositoryBinding>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ManagedContextProviderAccountSelection {
    None,
    Selected {
        accounts: Vec<ManagedContextProviderAccount>,
    },
}

impl Default for ManagedContextProviderAccountSelection {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedContextProviderAccount {
    pub provider: String,
    pub account_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ManagedContextGitCredentialSelection {
    None,
    Selected { credential_ids: Vec<String> },
}

impl Default for ManagedContextGitCredentialSelection {
    fn default() -> Self {
        Self::None
    }
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
    pub plan: ManagedContextPlanBinding,
    pub target_environment_id: String,
    pub source_kernel_id: String,
    pub source_key_thumbprint: String,
    pub target_kernel_id: String,
    pub target_key_thumbprint: String,
    pub development: ManagedContextPackageDevelopment,
    pub kernel_context: ManagedContextPackageKernel,
    pub provider_accounts: ManagedContextPackageProviderAccounts,
    pub git_credentials: ManagedContextPackageGitCredentials,
    pub package_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedContextPackageExportResult {
    pub plan: ManagedContextPlanBinding,
    pub package_path: PathBuf,
    pub package_sha256: String,
    pub package_size_bytes: u64,
    pub development_archive_sha256: Option<String>,
    pub kernel_context_snapshot_sha256: Option<String>,
    pub provider_accounts_sha256: Option<String>,
    pub git_credentials_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedContextPackageBinding {
    pub plan: ManagedContextPlanBinding,
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

#[derive(Clone)]
pub(crate) struct ManagedContextPackageApplicationRequest {
    pub transfer_id: String,
    pub package_path: PathBuf,
    pub expected_package_sha256: String,
    pub expected_binding: ManagedContextPackageBinding,
    pub development_destination_root: PathBuf,
    pub target_private_key: String,
    pub provider_account_target: Option<ManagedContextProviderAccountImportTarget>,
    pub git_credential_target: Option<ManagedContextGitCredentialImportTarget>,
}

#[derive(Clone)]
pub(crate) struct ManagedContextProviderAccountImportTarget {
    pub registry: ProviderAccountProfileRegistry,
    pub owner_user_id: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ManagedContextGitCredentialImportTarget {
    pub command_context: GitCredentialCommandContext,
}

impl std::fmt::Debug for ManagedContextProviderAccountImportTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedContextProviderAccountImportTarget")
            .field("owner_user_id", &self.owner_user_id)
            .finish_non_exhaustive()
    }
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
            .field("provider_account_target", &self.provider_account_target)
            .field("git_credential_target", &self.git_credential_target)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManagedContextPackageImportReceipt {
    pub schema_version: u32,
    pub transfer_id: String,
    pub package_sha256: String,
    pub plan_digest: String,
    pub development: ManagedContextImportedDevelopment,
    pub kernel_context: ManagedContextImportedKernelContext,
    #[serde(default)]
    pub provider_accounts: ManagedContextImportedProviderAccounts,
    #[serde(default)]
    pub git_credentials: ManagedContextImportedGitCredentials,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ManagedContextImportedProviderAccounts {
    None,
    Selected {
        accounts: Vec<ManagedContextProviderAccountReceipt>,
    },
}

impl Default for ManagedContextImportedProviderAccounts {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ManagedContextImportedGitCredentials {
    None,
    Selected {
        credentials: Vec<ManagedContextGitCredentialReceipt>,
    },
}

impl Default for ManagedContextImportedGitCredentials {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ManagedContextImportedKernelContext {
    Empty,
    FromKernel { receipt: KernelContextImportReceipt },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ManagedContextImportedDevelopment {
    Empty,
    FromSource {
        project_id: String,
        receipt: DevelopmentContextPublicationReceipt,
    },
}

#[derive(Debug)]
pub(crate) struct ExtractedManagedContextPackage {
    pub development: ExtractedManagedContextDevelopment,
    pub kernel_context: ManagedContextPackageKernel,
    pub provider_accounts: ManagedContextPackageProviderAccounts,
    pub git_credentials: ManagedContextPackageGitCredentials,
    cleanup: PackageExtractionCleanup,
}

#[derive(Debug)]
pub(crate) enum ExtractedManagedContextDevelopment {
    Empty,
    FromSource {
        archive_path: PathBuf,
        archive_sha256: String,
    },
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
    plan: ManagedContextPlanBinding,
    target_environment_id: String,
    source_kernel_id: String,
    source_key_thumbprint: String,
    target_kernel_id: String,
    target_key_thumbprint: String,
    development: DevelopmentContextComponentManifest,
    kernel_context: KernelContextComponentManifest,
    #[serde(default)]
    provider_accounts: ProviderAccountComponentManifest,
    #[serde(default)]
    git_credentials: GitCredentialComponentManifest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum DevelopmentContextComponentManifest {
    Empty,
    FromSource { size_bytes: u64, sha256: String },
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ProviderAccountComponentManifest {
    None,
    Selected {
        size_bytes: u64,
        sha256: String,
        account_count: usize,
    },
}

impl Default for ProviderAccountComponentManifest {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum GitCredentialComponentManifest {
    None,
    Selected {
        size_bytes: u64,
        sha256: String,
        credential_count: usize,
    },
}

impl Default for GitCredentialComponentManifest {
    fn default() -> Self {
        Self::None
    }
}

pub fn export_managed_context_package(
    request: ManagedContextPackageExportRequest,
) -> Result<ManagedContextPackageExportResult, DaemonError> {
    let binding = ManagedContextPackageBinding {
        plan: request.plan,
        target_environment_id: request.target_environment_id,
        source_kernel_id: request.source_kernel_id,
        source_key_thumbprint: request.source_key_thumbprint,
        target_kernel_id: request.target_kernel_id,
        target_key_thumbprint: request.target_key_thumbprint,
    };
    validate_binding(&binding)?;
    validate_supported_package_plan(&binding.plan)?;
    validate_output_path(&request.package_path)?;

    let (mut development, development_manifest, development_sha256) = match request.development {
        ManagedContextPackageDevelopment::Empty => {
            if !matches!(
                binding.plan.development,
                ManagedContextDevelopmentSelection::Empty
            ) {
                return Err(package_error(
                    "managed context package omits the selected development context",
                ));
            }
            (None, DevelopmentContextComponentManifest::Empty, None)
        }
        ManagedContextPackageDevelopment::FromSource {
            archive_path,
            archive_sha256,
        } => {
            if !matches!(
                binding.plan.development,
                ManagedContextDevelopmentSelection::SourceProject { .. }
            ) {
                return Err(package_error(
                    "managed context package includes unselected development context",
                ));
            }
            validate_sha256(&archive_sha256, "development archive digest")?;
            let mut archive = open_regular_file_no_follow(&archive_path)?;
            let size = archive
                .metadata()
                .map_err(|error| package_io_error("inspect development archive", error))?
                .len();
            if size == 0 || size > MAX_DEVELOPMENT_BYTES {
                return Err(package_error("development archive size is invalid"));
            }
            let actual_sha256 = hash_reader(&mut archive, MAX_DEVELOPMENT_BYTES)?;
            if actual_sha256 != archive_sha256.to_ascii_lowercase() {
                return Err(package_error(
                    "development archive digest does not match the requested digest",
                ));
            }
            archive
                .seek(SeekFrom::Start(0))
                .map_err(|error| package_io_error("rewind development archive", error))?;
            (
                Some((archive, size)),
                DevelopmentContextComponentManifest::FromSource {
                    size_bytes: size,
                    sha256: actual_sha256.clone(),
                },
                Some(actual_sha256),
            )
        }
    };

    let (kernel_bytes, kernel_manifest, kernel_snapshot_sha256) = match request.kernel_context {
        ManagedContextPackageKernel::Empty => {
            if binding.plan.kernel_context != ManagedContextKernelSelection::Empty {
                return Err(package_error(
                    "managed context package omits the selected kernel context",
                ));
            }
            (None, KernelContextComponentManifest::Empty, None)
        }
        ManagedContextPackageKernel::FromKernel(snapshot) => {
            if binding.plan.kernel_context != ManagedContextKernelSelection::SourceKernel {
                return Err(package_error(
                    "managed context package includes unselected kernel context",
                ));
            }
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
    let (provider_account_bytes, provider_account_manifest, provider_accounts_sha256) =
        match request.provider_accounts {
            ManagedContextPackageProviderAccounts::None => {
                if !matches!(
                    binding.plan.provider_accounts,
                    ManagedContextProviderAccountSelection::None
                ) {
                    return Err(package_error(
                        "managed context package omits the selected provider accounts",
                    ));
                }
                (None, ProviderAccountComponentManifest::None, None)
            }
            ManagedContextPackageProviderAccounts::Selected { materializations } => {
                validate_provider_account_materializations(&binding.plan, &materializations)?;
                let bytes = serialize_bounded_json(
                    &materializations,
                    MAX_PROVIDER_ACCOUNT_COMPONENT_BYTES as usize,
                )?;
                let size_bytes = bytes.len() as u64;
                let sha256 = sha256_bytes(&bytes);
                let account_count = materializations.len();
                (
                    Some(bytes),
                    ProviderAccountComponentManifest::Selected {
                        size_bytes,
                        sha256: sha256.clone(),
                        account_count,
                    },
                    Some(sha256),
                )
            }
        };
    let (git_credential_bytes, git_credential_manifest, git_credentials_sha256) =
        match request.git_credentials {
            ManagedContextPackageGitCredentials::None => {
                if !matches!(
                    binding.plan.git_credentials,
                    ManagedContextGitCredentialSelection::None
                ) {
                    return Err(package_error(
                        "managed context package omits the selected Git credentials",
                    ));
                }
                (None, GitCredentialComponentManifest::None, None)
            }
            ManagedContextPackageGitCredentials::Selected { materializations } => {
                validate_git_credential_materializations(
                    &binding.plan.git_credentials,
                    &materializations,
                )?;
                let bytes = serialize_bounded_json(
                    &materializations,
                    MAX_GIT_CREDENTIAL_COMPONENT_BYTES as usize,
                )?;
                let size_bytes = bytes.len() as u64;
                let sha256 = sha256_bytes(&bytes);
                let credential_count = materializations.len();
                (
                    Some(bytes),
                    GitCredentialComponentManifest::Selected {
                        size_bytes,
                        sha256: sha256.clone(),
                        credential_count,
                    },
                    Some(sha256),
                )
            }
        };
    let manifest = ManagedContextPackageManifest {
        schema_version: PACKAGE_SCHEMA_VERSION,
        plan: binding.plan,
        target_environment_id: binding.target_environment_id,
        source_kernel_id: binding.source_kernel_id,
        source_key_thumbprint: binding.source_key_thumbprint,
        target_kernel_id: binding.target_kernel_id,
        target_key_thumbprint: binding.target_key_thumbprint,
        development: development_manifest,
        kernel_context: kernel_manifest,
        provider_accounts: provider_account_manifest,
        git_credentials: git_credential_manifest,
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
    if let Some((archive, size)) = development.as_mut() {
        let copied_development =
            copy_component(archive, &mut writer, *size, MAX_DEVELOPMENT_BYTES)?;
        if Some(copied_development) != development_sha256 {
            return Err(package_error(
                "development archive changed while composing the managed context package",
            ));
        }
    }
    if let Some(bytes) = kernel_bytes.as_deref() {
        writer.write_all(bytes).map_err(package_write_error)?;
    }
    if let Some(bytes) = provider_account_bytes.as_deref() {
        writer.write_all(bytes).map_err(package_write_error)?;
    }
    if let Some(bytes) = git_credential_bytes.as_deref() {
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
        plan: manifest.plan,
        package_path: destination,
        package_sha256,
        package_size_bytes,
        development_archive_sha256: development_sha256,
        kernel_context_snapshot_sha256: kernel_snapshot_sha256,
        provider_accounts_sha256,
        git_credentials_sha256,
    })
}

pub(crate) fn apply_managed_context_package(
    request: ManagedContextPackageApplicationRequest,
) -> Result<ManagedContextPackageImportReceipt, DaemonError> {
    let extracted = extract_managed_context_package(ManagedContextPackageImportRequest {
        package_path: request.package_path.clone(),
        expected_package_sha256: request.expected_package_sha256.clone(),
        expected_binding: request.expected_binding.clone(),
    })?;
    preflight_import_receipt_capacity(&request, &extracted)?;
    let provider_accounts = import_provider_accounts(&request, &extracted.provider_accounts)?;
    let git_credentials = match import_git_credentials(&request, &extracted.git_credentials) {
        Ok(credentials) => credentials,
        Err(error) => {
            if let Err(rollback_error) = rollback_imported_provider_accounts(
                request.provider_account_target.as_ref(),
                &provider_accounts,
            ) {
                return Err(package_unavailable(format!(
                    "{error}; roll back provider accounts: {rollback_error}"
                )));
            }
            return Err(error);
        }
    };
    let imported_components = (|| {
        let development = match (
            &extracted.development,
            &request.expected_binding.plan.development,
        ) {
            (
                ExtractedManagedContextDevelopment::Empty,
                ManagedContextDevelopmentSelection::Empty,
            ) => ManagedContextImportedDevelopment::Empty,
            (
                ExtractedManagedContextDevelopment::FromSource {
                    archive_path,
                    archive_sha256,
                },
                ManagedContextDevelopmentSelection::SourceProject {
                    project_id,
                    repositories,
                },
            ) => {
                let development_request = DevelopmentContextImportRequest {
                    archive_path: archive_path.clone(),
                    expected_archive_sha256: archive_sha256.clone(),
                    expected_project_id: project_id.clone(),
                    expected_source_repositories: Some(repositories.clone()),
                    destination_root: request.development_destination_root.clone(),
                };
                let receipt = match recover_development_context_publication(
                    &development_request,
                    &request.transfer_id,
                )? {
                    Some(receipt) => receipt,
                    None => import_development_context_with_publication(
                        development_request,
                        request.transfer_id.clone(),
                    )?,
                };
                ManagedContextImportedDevelopment::FromSource {
                    project_id: project_id.clone(),
                    receipt,
                }
            }
            _ => {
                return Err(package_error(
                    "managed context development component does not match the launch plan",
                ))
            }
        };
        let kernel_context = match &extracted.kernel_context {
            ManagedContextPackageKernel::Empty => ManagedContextImportedKernelContext::Empty,
            ManagedContextPackageKernel::FromKernel(snapshot) => {
                let (capability_root, vault_path) = configured_managed_kernel_context_paths()?;
                let receipt = import_kernel_context(KernelContextImportRequest {
                    snapshot: snapshot.clone(),
                    expected_source: TransferredVaultSourceBinding {
                        context_id: request.expected_binding.plan.context_id.clone(),
                        source_kernel_id: request.expected_binding.source_kernel_id.clone(),
                        source_key_thumbprint: request
                            .expected_binding
                            .source_key_thumbprint
                            .clone(),
                    },
                    target_kernel_id: request.expected_binding.target_kernel_id.clone(),
                    target_private_key: request.target_private_key.clone(),
                    capability_root,
                    vault_path,
                })?;
                ManagedContextImportedKernelContext::FromKernel { receipt }
            }
        };
        Ok::<_, DaemonError>((development, kernel_context))
    })();
    let (development, kernel_context) = match imported_components {
        Ok(imported) => imported,
        Err(error) => {
            let git_rollback = rollback_imported_git_credentials(
                request.git_credential_target.as_ref(),
                &git_credentials,
            );
            let provider_rollback = rollback_imported_provider_accounts(
                request.provider_account_target.as_ref(),
                &provider_accounts,
            );
            if let Err(rollback_error) = git_rollback.and(provider_rollback) {
                return Err(package_unavailable(format!(
                    "{error}; roll back imported credentials: {rollback_error}"
                )));
            }
            return Err(error);
        }
    };
    Ok(ManagedContextPackageImportReceipt {
        schema_version: PACKAGE_SCHEMA_VERSION,
        transfer_id: request.transfer_id,
        package_sha256: request.expected_package_sha256,
        plan_digest: request.expected_binding.plan.plan_digest,
        development,
        kernel_context,
        provider_accounts,
        git_credentials,
    })
}

fn import_provider_accounts(
    request: &ManagedContextPackageApplicationRequest,
    provider_accounts: &ManagedContextPackageProviderAccounts,
) -> Result<ManagedContextImportedProviderAccounts, DaemonError> {
    match (
        provider_accounts,
        &request.expected_binding.plan.provider_accounts,
    ) {
        (
            ManagedContextPackageProviderAccounts::None,
            ManagedContextProviderAccountSelection::None,
        ) => Ok(ManagedContextImportedProviderAccounts::None),
        (
            ManagedContextPackageProviderAccounts::Selected { materializations },
            ManagedContextProviderAccountSelection::Selected { .. },
        ) => {
            let target = request.provider_account_target.as_ref().ok_or_else(|| {
                package_error("managed context provider-account target is unavailable")
            })?;
            let mut accounts = Vec::with_capacity(materializations.len());
            for materialization in materializations {
                match target.registry.materialize_managed_context_replica(
                    &target.owner_user_id,
                    &request.expected_binding.plan.context_id,
                    &request.expected_package_sha256,
                    materialization,
                ) {
                    Ok(receipt) => accounts.push(receipt),
                    Err(error) => {
                        let imported =
                            ManagedContextImportedProviderAccounts::Selected { accounts };
                        if let Err(rollback_error) =
                            rollback_imported_provider_accounts(Some(target), &imported)
                        {
                            return Err(package_unavailable(format!(
                                "{error}; roll back partial provider-account import: {rollback_error}"
                            )));
                        }
                        return Err(error);
                    }
                }
            }
            Ok(ManagedContextImportedProviderAccounts::Selected { accounts })
        }
        _ => Err(package_error(
            "managed context provider-account component does not match the launch plan",
        )),
    }
}

fn rollback_imported_provider_accounts(
    target: Option<&ManagedContextProviderAccountImportTarget>,
    provider_accounts: &ManagedContextImportedProviderAccounts,
) -> Result<(), DaemonError> {
    let ManagedContextImportedProviderAccounts::Selected { accounts } = provider_accounts else {
        return Ok(());
    };
    let target = target.ok_or_else(|| {
        package_error("managed context provider-account rollback target is unavailable")
    })?;
    let mut failures = Vec::new();
    for receipt in accounts.iter().rev() {
        if let Err(error) = target
            .registry
            .rollback_managed_context_replica(&target.owner_user_id, receipt)
        {
            failures.push(error);
        }
    }
    if let Some(first) = failures.first() {
        return Err(package_unavailable(format!(
            "{} provider-account rollback(s) failed; first failure: {first}",
            failures.len()
        )));
    }
    Ok(())
}

fn import_git_credentials(
    request: &ManagedContextPackageApplicationRequest,
    git_credentials: &ManagedContextPackageGitCredentials,
) -> Result<ManagedContextImportedGitCredentials, DaemonError> {
    match (
        git_credentials,
        &request.expected_binding.plan.git_credentials,
    ) {
        (ManagedContextPackageGitCredentials::None, ManagedContextGitCredentialSelection::None) => {
            Ok(ManagedContextImportedGitCredentials::None)
        }
        (
            ManagedContextPackageGitCredentials::Selected { materializations },
            selection @ ManagedContextGitCredentialSelection::Selected { .. },
        ) => {
            let target = request.git_credential_target.as_ref().ok_or_else(|| {
                package_error("managed context Git credential target is unavailable")
            })?;
            let credentials = materialize_git_credentials(
                &target.command_context,
                &request.expected_binding.plan.context_id,
                &request.expected_package_sha256,
                selection,
                materializations,
            )?;
            Ok(ManagedContextImportedGitCredentials::Selected { credentials })
        }
        _ => Err(package_error(
            "managed context Git credential component does not match the launch plan",
        )),
    }
}

fn rollback_imported_git_credentials(
    target: Option<&ManagedContextGitCredentialImportTarget>,
    git_credentials: &ManagedContextImportedGitCredentials,
) -> Result<(), DaemonError> {
    let ManagedContextImportedGitCredentials::Selected { credentials } = git_credentials else {
        return Ok(());
    };
    let target = target.ok_or_else(|| {
        package_error("managed context Git credential rollback target is unavailable")
    })?;
    rollback_git_credentials(&target.command_context, credentials)
}

pub(crate) fn rollback_managed_context_package_application(
    receipt: &ManagedContextPackageImportReceipt,
    target_private_key: &str,
    provider_account_target: Option<&ManagedContextProviderAccountImportTarget>,
    git_credential_target: Option<&ManagedContextGitCredentialImportTarget>,
) -> Result<(), DaemonError> {
    let mut failures = Vec::new();
    if let Err(error) =
        rollback_imported_git_credentials(git_credential_target, &receipt.git_credentials)
    {
        failures.push(error);
    }
    if let Err(error) =
        rollback_imported_provider_accounts(provider_account_target, &receipt.provider_accounts)
    {
        failures.push(error);
    }
    if let ManagedContextImportedKernelContext::FromKernel { receipt } = &receipt.kernel_context {
        match configured_managed_kernel_context_paths() {
            Ok((_, vault_path)) => {
                if let Err(error) =
                    cleanup_kernel_context_import(receipt, &vault_path, target_private_key)
                {
                    failures.push(error);
                }
            }
            Err(error) => failures.push(error),
        }
    }
    if let ManagedContextImportedDevelopment::FromSource { receipt, .. } = &receipt.development {
        if let Err(error) = super::development::cleanup_development_context_publication_staging(
            &receipt.destination_root,
            &receipt.publication_id,
        ) {
            failures.push(error);
        }
        if let Err(error) = super::development::cleanup_development_context_publication(
            &receipt.destination_root,
            &receipt.publication_id,
        ) {
            failures.push(error);
        }
    }
    if let Some(first) = failures.first() {
        return Err(package_unavailable(format!(
            "{} managed-context rollback(s) failed; first failure: {first}",
            failures.len()
        )));
    }
    Ok(())
}

pub(crate) fn rollback_persisted_managed_context_publication(
    request: ManagedContextPackageImportRequest,
    target_private_key: &str,
    provider_account_target: Option<&ManagedContextProviderAccountImportTarget>,
    git_credential_target: Option<&ManagedContextGitCredentialImportTarget>,
) -> Result<(), DaemonError> {
    let context_id = request.expected_binding.plan.context_id.clone();
    let package_sha256 = request.expected_package_sha256.clone();
    let extracted = extract_managed_context_package(request)?;
    let provider_accounts = match &extracted.provider_accounts {
        ManagedContextPackageProviderAccounts::None => ManagedContextImportedProviderAccounts::None,
        ManagedContextPackageProviderAccounts::Selected { materializations } => {
            let accounts = materializations
                .iter()
                .map(|materialization| {
                    Ok(ManagedContextProviderAccountReceipt {
                        context_id: context_id.clone(),
                        package_sha256: package_sha256.clone(),
                        materialization_sha256: provider_account_materialization_sha256(
                            materialization,
                        )?,
                        provider: materialization.profile.provider.clone(),
                        profile_id: materialization.profile.profile_id.clone(),
                    })
                })
                .collect::<Result<Vec<_>, DaemonError>>()?;
            ManagedContextImportedProviderAccounts::Selected { accounts }
        }
    };
    let git_credentials = match &extracted.git_credentials {
        ManagedContextPackageGitCredentials::None => ManagedContextImportedGitCredentials::None,
        ManagedContextPackageGitCredentials::Selected { materializations } => {
            let credentials = materializations
                .iter()
                .map(|materialization| {
                    receipt_for_materialization(&context_id, &package_sha256, materialization)
                })
                .collect::<Result<Vec<_>, DaemonError>>()?;
            ManagedContextImportedGitCredentials::Selected { credentials }
        }
    };
    let mut failures = Vec::new();
    if let Err(error) = rollback_imported_git_credentials(git_credential_target, &git_credentials) {
        failures.push(error);
    }
    if let Err(error) =
        rollback_imported_provider_accounts(provider_account_target, &provider_accounts)
    {
        failures.push(error);
    }
    if let ManagedContextPackageKernel::FromKernel(snapshot) = &extracted.kernel_context {
        match configured_managed_kernel_context_paths() {
            Ok((capability_root, vault_path)) => {
                if let Err(error) = cleanup_kernel_context_import(
                    &KernelContextImportReceipt {
                        schema_version: 1,
                        context_id: snapshot.payload.context_id.clone(),
                        source_kernel_id: snapshot.payload.source_kernel_id.clone(),
                        source_key_thumbprint: snapshot.payload.source_key_thumbprint.clone(),
                        target_kernel_id: snapshot.payload.target_kernel_id.clone(),
                        target_key_thumbprint: snapshot.payload.target_key_thumbprint.clone(),
                        snapshot_sha256: snapshot.snapshot_sha256.clone(),
                        capability_root,
                        extension_count: snapshot.payload.extensions.len(),
                        dependency_count: snapshot.payload.dependencies.len(),
                    },
                    &vault_path,
                    target_private_key,
                ) {
                    failures.push(error);
                }
            }
            Err(error) => failures.push(error),
        }
    }
    if let Some(first) = failures.first() {
        return Err(package_unavailable(format!(
            "{} recovered managed-context rollback(s) failed; first failure: {first}",
            failures.len()
        )));
    }
    Ok(())
}

fn preflight_import_receipt_capacity(
    request: &ManagedContextPackageApplicationRequest,
    extracted: &ExtractedManagedContextPackage,
) -> Result<(), DaemonError> {
    let (development, placeholder_receipt_bytes) = match (
        &extracted.development,
        &request.expected_binding.plan.development,
    ) {
        (ExtractedManagedContextDevelopment::Empty, ManagedContextDevelopmentSelection::Empty) => {
            (ManagedContextImportedDevelopment::Empty, 0)
        }
        (
            ExtractedManagedContextDevelopment::FromSource { .. },
            ManagedContextDevelopmentSelection::SourceProject { project_id, .. },
        ) => {
            let placeholder = DevelopmentContextPublicationReceipt {
                schema_version: 2,
                publication_id: request.transfer_id.clone(),
                archive_sha256: "0".repeat(64),
                project_id: project_id.clone(),
                destination_root: PathBuf::new(),
                primary_repository_id: "repository".to_string(),
                source_repository_binding_sha256s: Vec::new(),
                repositories: Vec::new(),
            };
            let placeholder_receipt_bytes = serde_json::to_vec(&placeholder)
                .map_err(|error| package_error(format!("serialize receipt preflight: {error}")))?
                .len();
            (
                ManagedContextImportedDevelopment::FromSource {
                    project_id: project_id.clone(),
                    receipt: placeholder,
                },
                placeholder_receipt_bytes,
            )
        }
        _ => {
            return Err(package_error(
                "managed context development component does not match the launch plan",
            ))
        }
    };
    let kernel_context = match &extracted.kernel_context {
        ManagedContextPackageKernel::Empty => ManagedContextImportedKernelContext::Empty,
        ManagedContextPackageKernel::FromKernel(snapshot) => {
            let (capability_root, _) = configured_managed_kernel_context_paths()?;
            ManagedContextImportedKernelContext::FromKernel {
                receipt: KernelContextImportReceipt {
                    schema_version: 1,
                    context_id: snapshot.payload.context_id.clone(),
                    source_kernel_id: snapshot.payload.source_kernel_id.clone(),
                    source_key_thumbprint: snapshot.payload.source_key_thumbprint.clone(),
                    target_kernel_id: snapshot.payload.target_kernel_id.clone(),
                    target_key_thumbprint: snapshot.payload.target_key_thumbprint.clone(),
                    snapshot_sha256: snapshot.snapshot_sha256.clone(),
                    capability_root,
                    extension_count: snapshot.payload.extensions.len(),
                    dependency_count: snapshot.payload.dependencies.len(),
                },
            }
        }
    };
    let provider_accounts = match &extracted.provider_accounts {
        ManagedContextPackageProviderAccounts::None => ManagedContextImportedProviderAccounts::None,
        ManagedContextPackageProviderAccounts::Selected { materializations } => {
            ManagedContextImportedProviderAccounts::Selected {
                accounts: materializations
                    .iter()
                    .map(|materialization| {
                        Ok(ManagedContextProviderAccountReceipt {
                            context_id: request.expected_binding.plan.context_id.clone(),
                            package_sha256: request.expected_package_sha256.clone(),
                            materialization_sha256: provider_account_materialization_sha256(
                                materialization,
                            )?,
                            provider: materialization.profile.provider.clone(),
                            profile_id: materialization.profile.profile_id.clone(),
                        })
                    })
                    .collect::<Result<Vec<_>, DaemonError>>()?,
            }
        }
    };
    let git_credentials = match &extracted.git_credentials {
        ManagedContextPackageGitCredentials::None => ManagedContextImportedGitCredentials::None,
        ManagedContextPackageGitCredentials::Selected { materializations } => {
            ManagedContextImportedGitCredentials::Selected {
                credentials: materializations
                    .iter()
                    .map(|materialization| {
                        receipt_for_materialization(
                            &request.expected_binding.plan.context_id,
                            &request.expected_package_sha256,
                            materialization,
                        )
                    })
                    .collect::<Result<Vec<_>, DaemonError>>()?,
            }
        }
    };
    let receipt = ManagedContextPackageImportReceipt {
        schema_version: PACKAGE_SCHEMA_VERSION,
        transfer_id: request.transfer_id.clone(),
        package_sha256: request.expected_package_sha256.clone(),
        plan_digest: request.expected_binding.plan.plan_digest.clone(),
        development,
        kernel_context,
        provider_accounts,
        git_credentials,
    };
    let placeholder_outer_bytes = serde_json::to_vec(&receipt)
        .map_err(|error| package_error(format!("serialize receipt preflight: {error}")))?
        .len();
    let maximum_receipt_bytes = placeholder_outer_bytes
        .saturating_sub(placeholder_receipt_bytes)
        .saturating_add(if placeholder_receipt_bytes == 0 {
            0
        } else {
            MAX_PUBLICATION_RECEIPT_BYTES
        });
    if maximum_receipt_bytes > super::transfer::MAX_IMPORT_RECEIPT_BYTES {
        return Err(package_error(
            "managed context import receipt would exceed its durable size limit",
        ));
    }
    Ok(())
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

    let development = match &manifest.development {
        DevelopmentContextComponentManifest::Empty => ExtractedManagedContextDevelopment::Empty,
        DevelopmentContextComponentManifest::FromSource { size_bytes, sha256 } => {
            let archive_path = component_root.join("development.tar.gz");
            let file = create_private_file(&archive_path)?;
            let actual_sha256 =
                copy_component_to_file(&mut reader, file, *size_bytes, MAX_DEVELOPMENT_BYTES)?;
            if &actual_sha256 != sha256 {
                return Err(package_error(
                    "managed context development component digest does not match",
                ));
            }
            ExtractedManagedContextDevelopment::FromSource {
                archive_path,
                archive_sha256: actual_sha256,
            }
        }
    };

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
    let provider_accounts = match &manifest.provider_accounts {
        ProviderAccountComponentManifest::None => ManagedContextPackageProviderAccounts::None,
        ProviderAccountComponentManifest::Selected {
            size_bytes,
            sha256,
            account_count,
        } => {
            let path = component_root.join("provider-accounts.json");
            let file = create_private_file(&path)?;
            let actual_sha256 = copy_component_to_file(
                &mut reader,
                file,
                *size_bytes,
                MAX_PROVIDER_ACCOUNT_COMPONENT_BYTES,
            )?;
            if &actual_sha256 != sha256 {
                return Err(package_error(
                    "managed context provider-account component digest does not match",
                ));
            }
            let file = open_regular_file_no_follow(&path)?;
            let materializations =
                serde_json::from_reader::<_, Vec<ProviderAccountMaterialization>>(file).map_err(
                    |_| package_error("managed context provider-account component is invalid"),
                )?;
            if materializations.len() != *account_count {
                return Err(package_error(
                    "managed context provider-account count does not match its manifest",
                ));
            }
            validate_provider_account_materializations(
                &request.expected_binding.plan,
                &materializations,
            )?;
            ManagedContextPackageProviderAccounts::Selected { materializations }
        }
    };
    let git_credentials = match &manifest.git_credentials {
        GitCredentialComponentManifest::None => ManagedContextPackageGitCredentials::None,
        GitCredentialComponentManifest::Selected {
            size_bytes,
            sha256,
            credential_count,
        } => {
            let path = component_root.join("git-credentials.json");
            let file = create_private_file(&path)?;
            let actual_sha256 = copy_component_to_file(
                &mut reader,
                file,
                *size_bytes,
                MAX_GIT_CREDENTIAL_COMPONENT_BYTES,
            )?;
            if &actual_sha256 != sha256 {
                return Err(package_error(
                    "managed context Git credential component digest does not match",
                ));
            }
            let file = open_regular_file_no_follow(&path)?;
            let materializations = serde_json::from_reader::<_, Vec<GitCredentialMaterialization>>(
                file,
            )
            .map_err(|_| package_error("managed context Git credential component is invalid"))?;
            if materializations.len() != *credential_count {
                return Err(package_error(
                    "managed context Git credential count does not match its manifest",
                ));
            }
            validate_git_credential_materializations(
                &request.expected_binding.plan.git_credentials,
                &materializations,
            )?;
            ManagedContextPackageGitCredentials::Selected { materializations }
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
        development,
        kernel_context,
        provider_accounts,
        git_credentials,
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
    if manifest.schema_version != PACKAGE_SCHEMA_VERSION
        && !LEGACY_PACKAGE_SCHEMA_VERSIONS.contains(&manifest.schema_version)
    {
        return Err(package_error("unsupported managed context package version"));
    }
    if manifest.schema_version == 2
        && (!matches!(
            &manifest.provider_accounts,
            ProviderAccountComponentManifest::None
        ) || !matches!(
            &manifest.plan.provider_accounts,
            ManagedContextProviderAccountSelection::None
        ))
    {
        return Err(package_error(
            "legacy managed context packages cannot carry provider accounts",
        ));
    }
    if manifest.schema_version < PACKAGE_SCHEMA_VERSION
        && (!matches!(
            &manifest.git_credentials,
            GitCredentialComponentManifest::None
        ) || !matches!(
            &manifest.plan.git_credentials,
            ManagedContextGitCredentialSelection::None
        ))
    {
        return Err(package_error(
            "legacy managed context packages cannot carry Git credentials",
        ));
    }
    let actual = ManagedContextPackageBinding {
        plan: manifest.plan.clone(),
        target_environment_id: manifest.target_environment_id.clone(),
        source_kernel_id: manifest.source_kernel_id.clone(),
        source_key_thumbprint: manifest.source_key_thumbprint.clone(),
        target_kernel_id: manifest.target_kernel_id.clone(),
        target_key_thumbprint: manifest.target_key_thumbprint.clone(),
    };
    validate_binding(&actual)?;
    validate_supported_package_plan(&actual.plan)?;
    if &actual != expected {
        return Err(package_error(
            "managed context package source or target binding does not match",
        ));
    }
    match (&manifest.development, &manifest.plan.development) {
        (DevelopmentContextComponentManifest::Empty, ManagedContextDevelopmentSelection::Empty) => {
        }
        (
            DevelopmentContextComponentManifest::FromSource { size_bytes, sha256 },
            ManagedContextDevelopmentSelection::SourceProject { .. },
        ) => {
            if *size_bytes == 0 || *size_bytes > MAX_DEVELOPMENT_BYTES {
                return Err(package_error(
                    "managed context development component size is invalid",
                ));
            }
            validate_sha256(sha256, "development component digest")?;
        }
        _ => {
            return Err(package_error(
                "managed context development component does not match the launch plan",
            ))
        }
    }
    match (&manifest.kernel_context, manifest.plan.kernel_context) {
        (KernelContextComponentManifest::Empty, ManagedContextKernelSelection::Empty) => Ok(()),
        (
            KernelContextComponentManifest::FromKernel {
                size_bytes,
                sha256,
                snapshot_sha256,
            },
            ManagedContextKernelSelection::SourceKernel,
        ) => {
            if *size_bytes == 0 || *size_bytes > MAX_KERNEL_CONTEXT_BYTES {
                return Err(package_error(
                    "managed context kernel component size is invalid",
                ));
            }
            validate_sha256(sha256, "kernel component digest")?;
            validate_sha256(snapshot_sha256, "kernel snapshot digest")
        }
        _ => Err(package_error(
            "managed context kernel component does not match the launch plan",
        )),
    }?;
    match (
        &manifest.provider_accounts,
        &manifest.plan.provider_accounts,
    ) {
        (ProviderAccountComponentManifest::None, ManagedContextProviderAccountSelection::None) => {
            Ok(())
        }
        (
            ProviderAccountComponentManifest::Selected {
                size_bytes,
                sha256,
                account_count,
            },
            ManagedContextProviderAccountSelection::Selected { accounts },
        ) => {
            if *size_bytes == 0
                || *size_bytes > MAX_PROVIDER_ACCOUNT_COMPONENT_BYTES
                || *account_count != accounts.len()
            {
                return Err(package_error(
                    "managed context provider-account component size or count is invalid",
                ));
            }
            validate_sha256(sha256, "provider-account component digest")
        }
        _ => Err(package_error(
            "managed context provider-account component does not match the launch plan",
        )),
    }?;
    match (&manifest.git_credentials, &manifest.plan.git_credentials) {
        (GitCredentialComponentManifest::None, ManagedContextGitCredentialSelection::None) => {
            Ok(())
        }
        (
            GitCredentialComponentManifest::Selected {
                size_bytes,
                sha256,
                credential_count,
            },
            ManagedContextGitCredentialSelection::Selected { credential_ids },
        ) => {
            if *size_bytes == 0
                || *size_bytes > MAX_GIT_CREDENTIAL_COMPONENT_BYTES
                || *credential_count != credential_ids.len()
            {
                return Err(package_error(
                    "managed context Git credential component size or count is invalid",
                ));
            }
            validate_sha256(sha256, "Git credential component digest")
        }
        _ => Err(package_error(
            "managed context Git credential component does not match the launch plan",
        )),
    }
}

fn validate_snapshot_binding(
    snapshot: &KernelContextSnapshot,
    binding: &ManagedContextPackageBinding,
) -> Result<(), DaemonError> {
    let payload = &snapshot.payload;
    if payload.context_id != binding.plan.context_id
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
    validate_plan_binding(&binding.plan)?;
    for (label, value) in [
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

pub(crate) fn validate_plan_binding(plan: &ManagedContextPlanBinding) -> Result<(), DaemonError> {
    serialize_bounded_json(plan, MAX_PLAN_BINDING_BYTES)?;
    validate_identifier(&plan.context_id, "context id")?;
    let digest = plan
        .plan_digest
        .strip_prefix("sha256:")
        .ok_or_else(|| package_error("managed context plan digest is invalid"))?;
    validate_sha256(digest, "plan digest")?;
    validate_development_selection(&plan.development)?;
    match &plan.provider_accounts {
        ManagedContextProviderAccountSelection::None => {}
        ManagedContextProviderAccountSelection::Selected { accounts } => {
            if accounts.is_empty() || accounts.len() > 16 {
                return Err(package_error(
                    "managed context provider account selection is invalid",
                ));
            }
            let mut previous: Option<(&str, &str)> = None;
            for account in accounts {
                validate_identifier(&account.provider, "provider")?;
                validate_identifier(&account.account_profile, "provider account")?;
                let current = (account.provider.as_str(), account.account_profile.as_str());
                if previous.is_some_and(|value| value >= current) {
                    return Err(package_error(
                        "managed context provider account selection is not canonical",
                    ));
                }
                previous = Some(current);
            }
        }
    }
    match &plan.git_credentials {
        ManagedContextGitCredentialSelection::None => {}
        ManagedContextGitCredentialSelection::Selected { credential_ids } => {
            if credential_ids.is_empty() || credential_ids.len() > 16 {
                return Err(package_error(
                    "managed context Git credential selection is invalid",
                ));
            }
            let mut previous: Option<&str> = None;
            for credential_id in credential_ids {
                validate_identifier(credential_id, "Git credential")?;
                if previous.is_some_and(|value| value >= credential_id.as_str()) {
                    return Err(package_error(
                        "managed context Git credential selection is not canonical",
                    ));
                }
                previous = Some(credential_id);
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_development_selection(
    development: &ManagedContextDevelopmentSelection,
) -> Result<(), DaemonError> {
    match development {
        ManagedContextDevelopmentSelection::Empty => {}
        ManagedContextDevelopmentSelection::SourceProject {
            project_id,
            repositories,
        } => {
            validate_identifier(project_id, "project id")?;
            if repositories.is_empty() || repositories.len() > 32 {
                return Err(package_error(
                    "managed context repository selection is invalid",
                ));
            }
            let mut primary = 0;
            let mut seen = std::collections::HashSet::new();
            for repository in repositories {
                primary += usize::from(repository.role == DevelopmentRepositoryRole::Primary);
                validate_reference(&repository.workspace_id, "Workspace id")?;
                if let Some(worktree_id) = repository.worktree_id.as_deref() {
                    validate_reference(worktree_id, "worktree id")?;
                }
                if !seen.insert((
                    repository.workspace_id.as_str(),
                    repository.worktree_id.as_deref(),
                )) {
                    return Err(package_error(
                        "managed context repository selection contains duplicates",
                    ));
                }
            }
            if primary != 1 {
                return Err(package_error(
                    "managed context repository selection must have one primary",
                ));
            }
        }
    }
    Ok(())
}

fn validate_supported_package_plan(plan: &ManagedContextPlanBinding) -> Result<(), DaemonError> {
    validate_git_credential_selection(&plan.git_credentials)
}

fn validate_provider_account_materializations(
    plan: &ManagedContextPlanBinding,
    materializations: &[ProviderAccountMaterialization],
) -> Result<(), DaemonError> {
    let ManagedContextProviderAccountSelection::Selected { accounts } = &plan.provider_accounts
    else {
        return Err(package_error(
            "managed context package includes unselected provider accounts",
        ));
    };
    if materializations.len() != accounts.len() {
        return Err(package_error(
            "managed context provider-account payload does not match the selection count",
        ));
    }
    let mut owner_user_id: Option<&str> = None;
    for (selection, materialization) in accounts.iter().zip(materializations) {
        let profile = &materialization.profile;
        let profile_matches_selection = if selection.account_profile == "default" {
            profile.is_default
        } else {
            profile.profile_id == selection.account_profile
        };
        if profile.provider != selection.provider
            || !profile_matches_selection
            || materialization.files.is_empty()
        {
            return Err(package_error(
                "managed context provider-account payload does not match the launch plan",
            ));
        }
        if !matches!(
            crate::provider::canonical_provider_family(&profile.provider),
            Some("codex" | "claude" | "opencode")
        ) {
            return Err(package_error(
                "managed context provider-account payload names an unsupported provider",
            ));
        }
        validate_reference(&profile.owner_user_id, "provider account owner")?;
        if owner_user_id.is_some_and(|owner| owner != profile.owner_user_id) {
            return Err(package_error(
                "managed context provider accounts have different owners",
            ));
        }
        owner_user_id = Some(&profile.owner_user_id);
        provider_account_materialization_sha256(materialization)?;
    }
    Ok(())
}

fn validate_reference(value: &str, label: &str) -> Result<(), DaemonError> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > 4096
        || value
            .bytes()
            .any(|byte| matches!(byte, b'\0' | b'\r' | b'\n'))
    {
        return Err(package_error(format!("managed context {label} is invalid")));
    }
    Ok(())
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

fn package_unavailable(message: impl Into<String>) -> DaemonError {
    DaemonError::ManagedContext {
        code: "managed_context_unavailable",
        operation: "managed context package",
        message: message.into(),
        retryable: true,
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
