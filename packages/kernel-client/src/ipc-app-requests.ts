export function listAppInstallationsRequest(options: { after?: string; limit?: number } = {}) {
  return { ListAppInstallations: { after: options.after ?? null, limit: options.limit ?? null } }
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
