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

/** Replaces the release of an existing installation, fenced on the generation the caller read. */
export function beginAppUpdateRequest(options: { sessionId: string; requestId: string; installationId: string; expectedGeneration: string; uploadHandle: string; expectedPackageDigest: string }) {
  return { BeginAppUpdate: { session_id: options.sessionId, request_id: options.requestId, installation_id: options.installationId, expected_generation: options.expectedGeneration, upload_handle: options.uploadHandle, expected_package_digest: options.expectedPackageDigest } }
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

export function uninstallAppRequest(installationId: string, expectedGeneration: string, deleteData = false) {
  return { UninstallApp: { installation_id: installationId, expected_generation: expectedGeneration,
    ...(deleteData ? { delete_data: true } : {}) } }
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

/** An event generator connection an inbox route subscribes to (protocol 358). */
export type AppInboxConnection = { generatorId: string; connectionId: string; connectionScope: string; filter?: unknown }

/** Protocol 353: route one external event type to an App's declared incoming
 * event; with a connection (358) the kernel subscribes to it at the generator. */
export function createAppInboxRouteRequest(options: {
  installationId: string; routeId: string; eventName: string; sourceEventType: string; sourceEventVersion: number;
  connection?: AppInboxConnection;
}) {
  const connection = options.connection
  return { CreateAppInboxRoute: {
    installation_id: options.installationId, route_id: options.routeId, event_name: options.eventName,
    source_event_type: options.sourceEventType, source_event_version: options.sourceEventVersion,
    ...(connection ? { connection: {
      generator_id: connection.generatorId, connection_id: connection.connectionId,
      connection_scope: connection.connectionScope, ...(connection.filter == null ? {} : { filter: connection.filter }),
    } } : {}),
  } }
}

/** Protocol 361: the caller's App set (releases, approved capabilities, configuration). */
export function getAppSetRequest() {
  return { GetAppSet: {} }
}

/** Protocol 360: a workflow event binding becomes an App inbox route on the
 * same connection; its actions become a grant and, optionally, the App's
 * outgoing event goes to the binding's workflow. The kernel undoes it on failure. */
export function moveEventBindingToAppRequest(options: {
  sessionId: string; bindingId: string; installationId: string; routeId: string; eventName: string;
  automation?: { automationId: string; eventName: string };
}) {
  return { MoveEventBindingToApp: {
    session_id: options.sessionId, binding_id: options.bindingId, installation_id: options.installationId,
    route_id: options.routeId, event_name: options.eventName,
    ...(options.automation ? { automation: {
      automation_id: options.automation.automationId, event_name: options.automation.eventName,
    } } : {}),
  } }
}

/** Protocol 359: let an App act through one of the owner's generator connections. */
export function grantAppConnectionRequest(installationId: string, generatorId: string, connectionId: string) {
  return { GrantAppConnection: { installation_id: installationId, generator_id: generatorId, connection_id: connectionId } }
}

export function revokeAppConnectionRequest(installationId: string, connectionId: string) {
  return { RevokeAppConnection: { installation_id: installationId, connection_id: connectionId } }
}

export function listAppConnectionsRequest(installationId: string) {
  return { ListAppConnections: { installation_id: installationId } }
}

export function removeAppInboxRouteRequest(installationId: string, routeId: string) {
  return { RemoveAppInboxRoute: { installation_id: installationId, route_id: routeId } }
}

export function listAppInboxRoutesRequest(installationId: string) {
  return { ListAppInboxRoutes: { installation_id: installationId } }
}

/** Accepts one occurrence as a source would; the payload must match the App's signed schema. */
export function testAppInboxRouteRequest(installationId: string, routeId: string, occurrenceId: string, payload: unknown) {
  return { TestAppInboxRoute: { installation_id: installationId, route_id: routeId, occurrence_id: occurrenceId, payload } }
}

/** Protocol 354: the owner answers an App's file request with the chosen files' names and bytes. */
export function grantAppFileRequest(sessionId: string, operationId: string, files: { name: string; contentsBase64: string }[]) {
  return { GrantAppFile: { session_id: sessionId, operation_id: operationId,
    files: files.map(file => ({ name: file.name, contents_base64: file.contentsBase64 })) } }
}

/** Protocol 355: the owner takes a copy of a file an App offered with `files.export`. */
export function saveAppFileExportRequest(sessionId: string, operationId: string) {
  return { SaveAppFileExport: { session_id: sessionId, operation_id: operationId } }
}
