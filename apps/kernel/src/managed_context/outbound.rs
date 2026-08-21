use std::fs::{File, OpenOptions};
use std::future::Future;
use std::io::{Read, Seek, SeekFrom};
#[cfg(test)]
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use chariox_relay::protocol::ClientTarget;
use rand::RngCore;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;

use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::managed_context::package::{
    ManagedContextPackageExportResult, ManagedContextPlanBinding,
};
use crate::transport::relay_client::{
    send_peer_request_to_known_kernel_via_relay, RelayClientState,
};
use crate::transport::relay_peer::{
    RelayManagedContextCapability, RelayManagedContextChunk, RelayManagedContextImportReceipt,
    RelayManagedContextTransferPhase, RelayManagedContextTransferStatus, RelayPeerRequest,
    RelayPeerResponse, RELAY_PEER_PROTOCOL_VERSION,
};

const IMPORT_STATUS_POLL_INTERVAL: Duration = Duration::from_millis(250);
const MAX_IMPORT_STATUS_POLLS: usize = 7_200;
const MAX_OUTBOUND_CHUNK_BYTES: usize = 512 * 1024;

pub(crate) trait ManagedContextPeerTransport: Send + Sync {
    fn send(
        &self,
        request: RelayPeerRequest,
    ) -> Pin<Box<dyn Future<Output = Result<RelayPeerResponse, DaemonError>> + Send + '_>>;
}

#[derive(Clone)]
pub(crate) struct RelayManagedContextPeerTransport {
    config: DaemonConfig,
    relay_state: Arc<RwLock<RelayClientState>>,
    target: ClientTarget,
    target_public_key: String,
}

impl RelayManagedContextPeerTransport {
    pub(crate) fn new(
        config: DaemonConfig,
        relay_state: Arc<RwLock<RelayClientState>>,
        target_kernel_id: String,
        target_public_key: String,
    ) -> Self {
        Self {
            config,
            relay_state,
            target: ClientTarget {
                daemon_id: Some(target_kernel_id),
                daemon_alias: None,
            },
            target_public_key,
        }
    }
}

impl ManagedContextPeerTransport for RelayManagedContextPeerTransport {
    fn send(
        &self,
        request: RelayPeerRequest,
    ) -> Pin<Box<dyn Future<Output = Result<RelayPeerResponse, DaemonError>> + Send + '_>> {
        Box::pin(send_peer_request_to_known_kernel_via_relay(
            &self.config,
            &self.relay_state,
            self.target.clone(),
            &self.target_public_key,
            request,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedContextOutboundTransferRequest {
    pub plan: ManagedContextPlanBinding,
    pub target_environment_id: String,
    pub target_kernel_id: String,
    pub target_key_thumbprint: String,
    pub package: ManagedContextPackageExportResult,
    pub capability: RelayManagedContextCapability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedContextOutboundTransferResult {
    pub transfer_id: String,
    pub package_sha256: String,
    pub package_size_bytes: u64,
    pub receipt: RelayManagedContextImportReceipt,
}

pub(crate) fn random_managed_context_capability() -> RelayManagedContextCapability {
    let mut bytes = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    RelayManagedContextCapability::new(
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes),
    )
}

pub(crate) async fn transfer_managed_context_package(
    transport: &impl ManagedContextPeerTransport,
    request: ManagedContextOutboundTransferRequest,
    mut observe: impl FnMut(&RelayManagedContextTransferStatus),
) -> Result<ManagedContextOutboundTransferResult, DaemonError> {
    validate_outbound_request(&request)?;
    let mut package = open_verified_package(&request.package)?;
    let capability = request.capability.clone();
    let armed = transport
        .send(RelayPeerRequest::ArmManagedContextImport {
            context_id: request.plan.context_id.clone(),
            plan_digest: request.plan.plan_digest.clone(),
            target_environment_id: request.target_environment_id,
            target_kernel_id: request.target_kernel_id,
            target_key_thumbprint: request.target_key_thumbprint,
            capability: capability.clone(),
            archive_sha256: request.package.package_sha256.clone(),
            archive_size_bytes: request.package.package_size_bytes,
        })
        .await?;
    let (transfer_id, armed_capability, max_chunk_bytes) = match armed {
        RelayPeerResponse::ManagedContextImportArmed {
            transfer_id,
            capability,
            max_chunk_bytes,
            relay_peer_protocol_version,
            ..
        } => {
            if relay_peer_protocol_version < RELAY_PEER_PROTOCOL_VERSION {
                return Err(outbound_error(
                    "target kernel does not support the selected managed-context protocol",
                    false,
                ));
            }
            if capability != request.capability {
                return Err(outbound_error(
                    "target kernel returned a different managed-context capability",
                    false,
                ));
            }
            if transfer_id.trim().is_empty()
                || max_chunk_bytes == 0
                || max_chunk_bytes > MAX_OUTBOUND_CHUNK_BYTES
            {
                return Err(outbound_error(
                    "target kernel returned invalid managed-context upload limits",
                    false,
                ));
            }
            (transfer_id, capability, max_chunk_bytes)
        }
        response => return Err(unexpected_response("arm", response)),
    };

    let mut status = status_response(
        "begin",
        transport
            .send(RelayPeerRequest::BeginManagedContextImport {
                transfer_id: transfer_id.clone(),
                capability: armed_capability.clone(),
            })
            .await?,
    )?;
    observe(&status);
    validate_status(&status, &transfer_id, request.package.package_size_bytes)?;

    while status.accepted_bytes < request.package.package_size_bytes {
        if matches!(
            status.phase,
            RelayManagedContextTransferPhase::ReadyToImport
                | RelayManagedContextTransferPhase::Importing
                | RelayManagedContextTransferPhase::Consumed
        ) {
            return Err(outbound_error(
                "target kernel ended upload before accepting the complete package",
                false,
            ));
        }
        let offset = status.accepted_bytes;
        let remaining = request.package.package_size_bytes - offset;
        let chunk_size = usize::try_from(remaining.min(max_chunk_bytes as u64))
            .map_err(|_| outbound_error("managed-context chunk size is invalid", false))?;
        let bytes = read_package_chunk(&mut package, offset, chunk_size)?;
        let next = status_response(
            "upload",
            transport
                .send(RelayPeerRequest::UploadManagedContextChunk {
                    transfer_id: transfer_id.clone(),
                    capability: armed_capability.clone(),
                    offset,
                    chunk_sha256: sha256_hex(&bytes),
                    data_base64: RelayManagedContextChunk::new(
                        base64::engine::general_purpose::STANDARD.encode(bytes),
                    ),
                })
                .await?,
        )?;
        validate_status(&next, &transfer_id, request.package.package_size_bytes)?;
        if next.accepted_bytes <= offset {
            return Err(outbound_error(
                "target kernel did not advance the managed-context upload offset",
                true,
            ));
        }
        status = next;
        observe(&status);
    }

    status = match transport
        .send(RelayPeerRequest::FinalizeManagedContextImport {
            transfer_id: transfer_id.clone(),
            capability: armed_capability.clone(),
        })
        .await
    {
        Ok(response) => status_response("finalize", response)?,
        Err(_) => get_status(transport, &transfer_id, &armed_capability).await?,
    };
    observe(&status);
    validate_status(&status, &transfer_id, request.package.package_size_bytes)?;

    for _ in 0..MAX_IMPORT_STATUS_POLLS {
        match status.phase {
            RelayManagedContextTransferPhase::Consumed => {
                let receipt = status.receipt.ok_or_else(|| {
                    outbound_error("target kernel omitted the managed-context receipt", false)
                })?;
                if receipt.transfer_id != transfer_id
                    || receipt.archive_sha256 != request.package.package_sha256
                    || receipt.plan_digest != request.plan.plan_digest
                    || !valid_sha256_hex(&receipt.receipt_sha256)
                {
                    return Err(outbound_error(
                        "target kernel returned a conflicting managed-context receipt",
                        false,
                    ));
                }
                return Ok(ManagedContextOutboundTransferResult {
                    transfer_id,
                    package_sha256: request.package.package_sha256,
                    package_size_bytes: request.package.package_size_bytes,
                    receipt,
                });
            }
            RelayManagedContextTransferPhase::Importing
            | RelayManagedContextTransferPhase::ReadyToImport => {
                tokio::time::sleep(IMPORT_STATUS_POLL_INTERVAL).await;
                status = get_status(transport, &transfer_id, &armed_capability).await?;
                observe(&status);
                validate_status(&status, &transfer_id, request.package.package_size_bytes)?;
            }
            RelayManagedContextTransferPhase::Armed
            | RelayManagedContextTransferPhase::Receiving => {
                return Err(outbound_error(
                    "target kernel regressed after receiving the complete package",
                    true,
                ));
            }
        }
    }
    Err(outbound_error(
        "timed out waiting for the target kernel to import managed context",
        true,
    ))
}

async fn get_status(
    transport: &impl ManagedContextPeerTransport,
    transfer_id: &str,
    capability: &RelayManagedContextCapability,
) -> Result<RelayManagedContextTransferStatus, DaemonError> {
    status_response(
        "status",
        transport
            .send(RelayPeerRequest::GetManagedContextImportStatus {
                transfer_id: transfer_id.to_string(),
                capability: capability.clone(),
            })
            .await?,
    )
}

fn status_response(
    operation: &'static str,
    response: RelayPeerResponse,
) -> Result<RelayManagedContextTransferStatus, DaemonError> {
    match response {
        RelayPeerResponse::ManagedContextImportStatus { status } => Ok(status),
        RelayPeerResponse::ManagedContextImportFailed { code, retryable } => {
            Err(DaemonError::ManagedContext {
                code: "target_import_failed",
                operation: "send managed context",
                message: format!("target kernel rejected managed context with `{code}`"),
                retryable,
            })
        }
        response => Err(unexpected_response(operation, response)),
    }
}

fn validate_outbound_request(
    request: &ManagedContextOutboundTransferRequest,
) -> Result<(), DaemonError> {
    if request.plan != request.package.plan
        || request.target_environment_id.trim().is_empty()
        || request.target_kernel_id.trim().is_empty()
        || request.target_key_thumbprint.len() != 64
        || !request
            .target_key_thumbprint
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        || request.package.package_size_bytes == 0
        || request.package.package_path.as_os_str().is_empty()
    {
        return Err(outbound_error(
            "managed-context package and target binding are invalid",
            false,
        ));
    }
    Ok(())
}

fn open_verified_package(package: &ManagedContextPackageExportResult) -> Result<File, DaemonError> {
    let mut options = OpenOptions::new();
    options.read(true);
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
    let mut file = options
        .open(&package.package_path)
        .map_err(|error| outbound_io_error("open managed-context package", error))?;
    let metadata = file
        .metadata()
        .map_err(|error| outbound_io_error("inspect managed-context package", error))?;
    if !metadata.is_file() || metadata.len() != package.package_size_bytes {
        return Err(outbound_error(
            "managed-context package size changed before upload",
            false,
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(outbound_error(
                "managed-context package must not be hard-linked",
                false,
            ));
        }
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| outbound_io_error("hash managed-context package", error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    if format!("{:x}", hasher.finalize()) != package.package_sha256 {
        return Err(outbound_error(
            "managed-context package digest changed before upload",
            false,
        ));
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|error| outbound_io_error("rewind managed-context package", error))?;
    Ok(file)
}

fn read_package_chunk(file: &mut File, offset: u64, size: usize) -> Result<Vec<u8>, DaemonError> {
    file.seek(SeekFrom::Start(offset))
        .map_err(|error| outbound_io_error("seek managed-context package", error))?;
    let mut bytes = vec![0_u8; size];
    file.read_exact(&mut bytes)
        .map_err(|error| outbound_io_error("read managed-context package", error))?;
    Ok(bytes)
}

fn validate_status(
    status: &RelayManagedContextTransferStatus,
    transfer_id: &str,
    package_size_bytes: u64,
) -> Result<(), DaemonError> {
    if status.transfer_id != transfer_id
        || status.archive_size_bytes != package_size_bytes
        || status.accepted_bytes > package_size_bytes
    {
        return Err(outbound_error(
            "target kernel returned a conflicting managed-context transfer status",
            false,
        ));
    }
    Ok(())
}

fn valid_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn unexpected_response(operation: &'static str, _response: RelayPeerResponse) -> DaemonError {
    outbound_error(
        format!("target kernel returned an unexpected response to {operation}"),
        false,
    )
}

fn outbound_error(message: impl Into<String>, retryable: bool) -> DaemonError {
    DaemonError::ManagedContext {
        code: "managed_context_transfer_failed",
        operation: "send managed context",
        message: message.into(),
        retryable,
    }
}

fn outbound_io_error(operation: &'static str, error: std::io::Error) -> DaemonError {
    DaemonError::ManagedContext {
        code: "managed_context_source_unavailable",
        operation,
        message: error.to_string(),
        retryable: true,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_context::package::{
        ManagedContextDevelopmentSelection, ManagedContextGitCredentialSelection,
        ManagedContextKernelSelection, ManagedContextPackageDevelopment,
        ManagedContextPackageKernel, ManagedContextProviderAccountSelection,
    };
    use crate::transport::relay_peer::{
        RelayManagedDevelopmentContextImportReceipt, RelayManagedKernelContextImportReceipt,
    };
    use std::collections::VecDeque;
    use std::sync::Mutex;

    struct FakeTransport {
        responses: Mutex<VecDeque<Result<RelayPeerResponse, DaemonError>>>,
        requests: Mutex<Vec<RelayPeerRequest>>,
    }

    impl FakeTransport {
        fn new(responses: Vec<Result<RelayPeerResponse, DaemonError>>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<RelayPeerRequest> {
            self.requests.lock().expect("request lock").clone()
        }
    }

    impl ManagedContextPeerTransport for FakeTransport {
        fn send(
            &self,
            request: RelayPeerRequest,
        ) -> Pin<Box<dyn Future<Output = Result<RelayPeerResponse, DaemonError>> + Send + '_>>
        {
            self.requests.lock().expect("request lock").push(request);
            let response = self
                .responses
                .lock()
                .expect("response lock")
                .pop_front()
                .expect("fake response");
            Box::pin(async move { response })
        }
    }

    #[tokio::test]
    async fn streams_package_in_bounded_chunks_and_returns_receipt() {
        let fixture = outbound_fixture(700_000);
        let fixture_root = fixture.root.clone();
        let capability = RelayManagedContextCapability::new("capability".to_string());
        let receipt = receipt(&fixture.package.package_sha256, &fixture.plan.plan_digest);
        let transport = FakeTransport::new(vec![
            Ok(armed(&capability, 300_000)),
            Ok(status(
                RelayManagedContextTransferPhase::Armed,
                0,
                None,
                700_000,
            )),
            Ok(status(
                RelayManagedContextTransferPhase::Receiving,
                300_000,
                None,
                700_000,
            )),
            Ok(status(
                RelayManagedContextTransferPhase::Receiving,
                600_000,
                None,
                700_000,
            )),
            Ok(status(
                RelayManagedContextTransferPhase::ReadyToImport,
                700_000,
                None,
                700_000,
            )),
            Ok(status(
                RelayManagedContextTransferPhase::Consumed,
                700_000,
                Some(receipt.clone()),
                700_000,
            )),
        ]);
        let result = transfer_managed_context_package(
            &transport,
            ManagedContextOutboundTransferRequest {
                plan: fixture.plan,
                target_environment_id: "environment-1".to_string(),
                target_kernel_id: "target-kernel".to_string(),
                target_key_thumbprint: "a".repeat(64),
                package: fixture.package,
                capability,
            },
            |_| {},
        )
        .await
        .expect("transfer should complete");
        assert_eq!(result.receipt, receipt);
        let requests = transport.requests();
        let uploads = requests
            .iter()
            .filter_map(|request| match request {
                RelayPeerRequest::UploadManagedContextChunk {
                    offset,
                    data_base64,
                    ..
                } => Some((*offset, data_base64.clone().into_inner())),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(uploads.len(), 3);
        assert_eq!(uploads[0].0, 0);
        assert_eq!(uploads[1].0, 300_000);
        assert_eq!(uploads[2].0, 600_000);
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(&uploads[2].1)
                .expect("chunk base64")
                .len(),
            100_000
        );
        std::fs::remove_dir_all(fixture_root).expect("fixture cleanup");
    }

    #[tokio::test]
    async fn resumes_from_target_offset_and_recovers_lost_finalize_response() {
        let fixture = outbound_fixture(600_000);
        let fixture_root = fixture.root.clone();
        let capability = RelayManagedContextCapability::new("capability".to_string());
        let receipt = receipt(&fixture.package.package_sha256, &fixture.plan.plan_digest);
        let transport = FakeTransport::new(vec![
            Ok(armed(&capability, 400_000)),
            Ok(status(
                RelayManagedContextTransferPhase::Receiving,
                400_000,
                None,
                600_000,
            )),
            Ok(status(
                RelayManagedContextTransferPhase::ReadyToImport,
                600_000,
                None,
                600_000,
            )),
            Err(DaemonError::LocalTransport {
                operation: "test",
                message: "lost response".to_string(),
            }),
            Ok(status(
                RelayManagedContextTransferPhase::Importing,
                600_000,
                None,
                600_000,
            )),
            Ok(status(
                RelayManagedContextTransferPhase::Consumed,
                600_000,
                Some(receipt.clone()),
                600_000,
            )),
        ]);
        let result = transfer_managed_context_package(
            &transport,
            ManagedContextOutboundTransferRequest {
                plan: fixture.plan,
                target_environment_id: "environment-1".to_string(),
                target_kernel_id: "target-kernel".to_string(),
                target_key_thumbprint: "a".repeat(64),
                package: fixture.package,
                capability,
            },
            |_| {},
        )
        .await
        .expect("transfer should recover");
        assert_eq!(result.receipt, receipt);
        let requests = transport.requests();
        let upload = requests
            .iter()
            .find_map(|request| match request {
                RelayPeerRequest::UploadManagedContextChunk { offset, .. } => Some(*offset),
                _ => None,
            })
            .expect("upload request");
        assert_eq!(upload, 400_000);
        assert!(requests.iter().any(|request| matches!(
            request,
            RelayPeerRequest::GetManagedContextImportStatus { .. }
        )));
        std::fs::remove_dir_all(fixture_root).expect("fixture cleanup");
    }

    #[tokio::test]
    async fn rejects_conflicting_receipt() {
        let fixture = outbound_fixture(10);
        let fixture_root = fixture.root.clone();
        let capability = RelayManagedContextCapability::new("capability".to_string());
        let transport = FakeTransport::new(vec![
            Ok(armed(&capability, 512)),
            Ok(status(RelayManagedContextTransferPhase::Armed, 0, None, 10)),
            Ok(status(
                RelayManagedContextTransferPhase::ReadyToImport,
                10,
                None,
                10,
            )),
            Ok(status(
                RelayManagedContextTransferPhase::Consumed,
                10,
                Some(receipt(&"0".repeat(64), &fixture.plan.plan_digest)),
                10,
            )),
        ]);
        let error = transfer_managed_context_package(
            &transport,
            ManagedContextOutboundTransferRequest {
                plan: fixture.plan,
                target_environment_id: "environment-1".to_string(),
                target_kernel_id: "target-kernel".to_string(),
                target_key_thumbprint: "a".repeat(64),
                package: fixture.package,
                capability,
            },
            |_| {},
        )
        .await
        .expect_err("conflicting receipt should fail");
        assert!(error
            .to_string()
            .contains("conflicting managed-context receipt"));
        std::fs::remove_dir_all(fixture_root).expect("fixture cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_hard_linked_package_before_contacting_the_target() {
        let fixture = outbound_fixture(10);
        std::fs::hard_link(
            &fixture.package.package_path,
            fixture.root.join("package-hard-link"),
        )
        .expect("hard link package");
        let error = open_verified_package(&fixture.package).expect_err("hard link must fail");
        assert!(error.to_string().contains("must not be hard-linked"));
        std::fs::remove_dir_all(fixture.root).expect("fixture cleanup");
    }

    struct OutboundFixture {
        root: PathBuf,
        plan: ManagedContextPlanBinding,
        package: ManagedContextPackageExportResult,
    }

    fn outbound_fixture(size: usize) -> OutboundFixture {
        let root = std::env::temp_dir().join(format!(
            "chariox-managed-context-outbound-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&root).expect("fixture root");
        let package_path = root.join("context.pkg");
        let bytes = vec![7_u8; size];
        std::fs::write(&package_path, &bytes).expect("fixture package");
        let plan = ManagedContextPlanBinding {
            context_id: "context-1".to_string(),
            plan_digest: format!("sha256:{}", "1".repeat(64)),
            kernel_context: ManagedContextKernelSelection::Empty,
            development: ManagedContextDevelopmentSelection::Empty,
            provider_accounts: ManagedContextProviderAccountSelection::None,
            git_credentials: ManagedContextGitCredentialSelection::None,
        };
        OutboundFixture {
            root,
            plan: plan.clone(),
            package: ManagedContextPackageExportResult {
                plan,
                package_path,
                package_sha256: sha256_hex(&bytes),
                package_size_bytes: size as u64,
                development_archive_sha256: None,
                kernel_context_snapshot_sha256: None,
            },
        }
    }

    fn armed(
        capability: &RelayManagedContextCapability,
        max_chunk_bytes: usize,
    ) -> RelayPeerResponse {
        RelayPeerResponse::ManagedContextImportArmed {
            transfer_id: "transfer-1".to_string(),
            capability: capability.clone(),
            expires_at_ms: u64::MAX,
            max_chunk_bytes,
            relay_peer_protocol_version: RELAY_PEER_PROTOCOL_VERSION,
        }
    }

    fn status(
        phase: RelayManagedContextTransferPhase,
        accepted_bytes: u64,
        receipt: Option<RelayManagedContextImportReceipt>,
        archive_size_bytes: u64,
    ) -> RelayPeerResponse {
        RelayPeerResponse::ManagedContextImportStatus {
            status: RelayManagedContextTransferStatus {
                transfer_id: "transfer-1".to_string(),
                phase,
                accepted_bytes,
                archive_size_bytes,
                expires_at_ms: u64::MAX,
                receipt,
            },
        }
    }

    fn receipt(package_sha256: &str, plan_digest: &str) -> RelayManagedContextImportReceipt {
        RelayManagedContextImportReceipt {
            transfer_id: "transfer-1".to_string(),
            archive_sha256: package_sha256.to_string(),
            plan_digest: plan_digest.to_string(),
            development: RelayManagedDevelopmentContextImportReceipt::Empty,
            kernel_context: RelayManagedKernelContextImportReceipt::Empty,
            receipt_sha256: "b".repeat(64),
        }
    }

    #[allow(dead_code)]
    fn _package_kinds_compile() -> (
        ManagedContextPackageDevelopment,
        ManagedContextPackageKernel,
    ) {
        (
            ManagedContextPackageDevelopment::Empty,
            ManagedContextPackageKernel::Empty,
        )
    }
}
