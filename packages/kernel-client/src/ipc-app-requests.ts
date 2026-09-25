export function listAppInstallationsRequest(options: { after?: string; limit?: number } = {}) {
  return { ListAppInstallations: { after: options.after ?? null, limit: options.limit ?? null } }
}

export function beginAppPublisherEnrollmentRequest(options: {
  sessionId: string; requestId: string; publisherId: string; keyId: string;
  publicKeyBase64: string; expectedRevision: string;
}) {
  return { BeginAppPublisherEnrollment: {
    session_id: options.sessionId, request_id: options.requestId,
    publisher_id: options.publisherId, key_id: options.keyId,
    public_key_base64: options.publicKeyBase64, expected_revision: options.expectedRevision,
  } }
}

export function getAppPublisherEnrollmentRequest(requestId: string) {
  return { GetAppPublisherEnrollment: { request_id: requestId } }
}

export function cancelAppPublisherEnrollmentRequest(requestId: string) {
  return { CancelAppPublisherEnrollment: { request_id: requestId } }
}

export function getAppInstallationRequest(installationId: string) {
  return { GetAppInstallation: { installation_id: installationId } }
}

export function getAppInstallationJournalRequest(installationId: string) {
  return { GetAppInstallationJournal: { installation_id: installationId } }
}

export function beginAppPackageUploadRequest(options: { requestId: string; expectedSize: number; sha256: string }) {
  return { BeginAppPackageUpload: { request_id: options.requestId, expected_size: options.expectedSize, sha256: options.sha256 } }
}

export function putAppPackageUploadChunkRequest(options: { handle: string; offset: number; dataBase64: string; chunkSha256: string }) {
  return { PutAppPackageUploadChunk: { handle: options.handle, offset: options.offset, data_base64: options.dataBase64, chunk_sha256: options.chunkSha256 } }
}

export function getAppPackageUploadRequest(handle: string) {
  return { GetAppPackageUpload: { handle } }
}

export function abortAppPackageUploadRequest(handle: string) {
  return { AbortAppPackageUpload: { handle } }
}

/** The upload is owner-bound by the kernel; this request supplies no trust key or approval. */
export function beginAppInstallRequest(options: { sessionId: string; requestId: string; uploadHandle: string; expectedPackageDigest: string }) {
  return { BeginAppInstall: { session_id: options.sessionId, request_id: options.requestId, upload_handle: options.uploadHandle, expected_package_digest: options.expectedPackageDigest } }
}

export function getAppInstallOperationRequest(requestId: string) {
  return { GetAppInstallOperation: { request_id: requestId } }
}

export function cancelAppInstallOperationRequest(requestId: string) {
  return { CancelAppInstallOperation: { request_id: requestId } }
}

export function getAppLogsRequest(installationId: string, afterSequence?: string, limit?: number) {
  return { GetAppLogs: { installation_id: installationId,
    ...(afterSequence ? { after_sequence: afterSequence } : {}), ...(limit ? { limit } : {}) } }
}

export function uninstallAppRequest(installationId: string, expectedGeneration: string) {
  return { UninstallApp: { installation_id: installationId, expected_generation: expectedGeneration } }
}

export function openAppViewRequest(sessionId: string, installationId: string) {
  return { OpenAppView: { session_id: sessionId, installation_id: installationId } }
}

export function getAppWorkerRequest(installationId: string) {
  return { GetAppWorker: { installation_id: installationId } }
}

export function controlAppWorkerRequest(installationId: string, action: "start" | "stop" | "restart") {
  return { ControlAppWorker: { installation_id: installationId, action } }
}

export function listAppAutomationsRequest(installationId: string) {
  return { ListAppAutomations: { installation_id: installationId } }
}

/** Expected revision zero creates the automation; replacements name the current revision. */
export function configureAppAutomationRequest(options: {
  installationId: string; automationId: string; expectedRevision: number; eventName: string;
  sessionId: string; publicationRef: string; queueRef?: string; scheduled?: boolean;
}) {
  return { ConfigureAppAutomation: {
    installation_id: options.installationId, automation_id: options.automationId,
    expected_revision: options.expectedRevision, event_name: options.eventName,
    session_id: options.sessionId, publication_ref: options.publicationRef,
    queue_ref: options.queueRef ?? null, scheduled: options.scheduled ?? false,
  } }
}

export function disableAppAutomationRequest(installationId: string, automationId: string, expectedRevision: number) {
  return { DisableAppAutomation: { installation_id: installationId, automation_id: automationId, expected_revision: expectedRevision } }
}
