import assert from "node:assert/strict"
import { test } from "node:test"
import { abortAppPackageUploadRequest, beginAppPackageUploadRequest, getAppPackageUploadRequest, putAppPackageUploadChunkRequest, getAppInstallationJournalRequest, getAppInstallationRequest, listAppInstallationsRequest } from "./ipc-app-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"
import { beginAppInstallRequest, beginAppUpdateRequest, getAppInstallOperationRequest, cancelAppInstallOperationRequest } from "./ipc-app-requests.js"
import { beginAppPublisherEnrollmentRequest, getAppPublisherEnrollmentRequest, cancelAppPublisherEnrollmentRequest } from "./ipc-app-requests.js"
import { createAppInboxRouteRequest, listAppInboxRoutesRequest, removeAppInboxRouteRequest, testAppInboxRouteRequest } from "./ipc-app-requests.js"
import { grantAppFileRequest, saveAppFileExportRequest } from "./ipc-app-requests.js"

test("App inspection shares protocol 297 without client owner or host paths", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 356)
  assert.deepEqual(listAppInstallationsRequest(), { ListAppInstallations: { after: null, limit: null } })
  assert.deepEqual(listAppInstallationsRequest({ after: "todo", limit: 1 }), { ListAppInstallations: { after: "todo", limit: 1 } })
  assert.deepEqual(getAppInstallationRequest("todo"), { GetAppInstallation: { installation_id: "todo" } })
  assert.deepEqual(getAppInstallationJournalRequest("todo"), { GetAppInstallationJournal: { installation_id: "todo" } })
})

test("App package transport uses opaque handles without owner, path or expiry authority", () => {
  assert.deepEqual(beginAppPackageUploadRequest({ requestId: "retry", expectedSize: 4, sha256: "sha256:digest" }), { BeginAppPackageUpload: { request_id: "retry", expected_size: 4, sha256: "sha256:digest" } })
  assert.deepEqual(putAppPackageUploadChunkRequest({ handle: "opaque", offset: 0, dataBase64: "dGVzdA==", chunkSha256: "sha256:chunk" }), { PutAppPackageUploadChunk: { handle: "opaque", offset: 0, data_base64: "dGVzdA==", chunk_sha256: "sha256:chunk" } })
  assert.deepEqual(getAppPackageUploadRequest("opaque"), { GetAppPackageUpload: { handle: "opaque" } })
  assert.deepEqual(abortAppPackageUploadRequest("opaque"), { AbortAppPackageUpload: { handle: "opaque" } })
})

test("install operation retry and cancellation share kernel ownership without client consent", () => {
  const options = { sessionId: "session", requestId: "retry", uploadHandle: `upload_${"a".repeat(64)}`, expectedPackageDigest: `sha256:${"b".repeat(64)}` }
  assert.deepEqual(beginAppInstallRequest(options), { BeginAppInstall: { session_id: "session", request_id: "retry", upload_handle: options.uploadHandle, expected_package_digest: options.expectedPackageDigest } })
  assert.deepEqual(getAppInstallOperationRequest("retry"), { GetAppInstallOperation: { request_id: "retry" } })
  assert.deepEqual(cancelAppInstallOperationRequest("retry"), { CancelAppInstallOperation: { request_id: "retry" } })
  assert.deepEqual(Object.keys(beginAppInstallRequest(options).BeginAppInstall).sort(), ["expected_package_digest", "request_id", "session_id", "upload_handle"])
})

test("update is fenced on the reviewed generation and otherwise matches install", () => {
  const options = { sessionId: "session", requestId: "retry", installationId: "todo", expectedGeneration: "9007199254740993", uploadHandle: `upload_${"a".repeat(64)}`, expectedPackageDigest: `sha256:${"b".repeat(64)}` }
  assert.deepEqual(beginAppUpdateRequest(options), { BeginAppUpdate: { session_id: "session", request_id: "retry", installation_id: "todo", expected_generation: "9007199254740993", upload_handle: options.uploadHandle, expected_package_digest: options.expectedPackageDigest } })
  assert.deepEqual(Object.keys(beginAppUpdateRequest(options).BeginAppUpdate).sort(), ["expected_generation", "expected_package_digest", "installation_id", "request_id", "session_id", "upload_handle"])
})

test("publisher enrollment sends review material and exact revisions without a client trust decision", () => {
  const options = { sessionId: "session", requestId: "retry", publisherId: "com.example", keyId: "developer",
    publicKeyBase64: Buffer.alloc(32, 7).toString("base64"), expectedRevision: "9007199254740993" }
  const request = beginAppPublisherEnrollmentRequest(options)
  assert.deepEqual(request, { BeginAppPublisherEnrollment: {
    session_id: "session", request_id: "retry", publisher_id: "com.example", key_id: "developer",
    public_key_base64: options.publicKeyBase64, expected_revision: "9007199254740993",
  } })
  assert.deepEqual(getAppPublisherEnrollmentRequest("retry"), { GetAppPublisherEnrollment: { request_id: "retry" } })
  assert.deepEqual(cancelAppPublisherEnrollmentRequest("retry"), { CancelAppPublisherEnrollment: { request_id: "retry" } })
})

test("inbox routes name an installation and its signed incoming event only", () => {
  assert.deepEqual(createAppInboxRouteRequest({ installationId: "todo", routeId: "mail", eventName: "todo_requested",
    sourceEventType: "dev.chariox.dummy/dummy.test", sourceEventVersion: 1 }), { CreateAppInboxRoute: {
    installation_id: "todo", route_id: "mail", event_name: "todo_requested",
    source_event_type: "dev.chariox.dummy/dummy.test", source_event_version: 1 } })
  assert.deepEqual(listAppInboxRoutesRequest("todo"), { ListAppInboxRoutes: { installation_id: "todo" } })
  assert.deepEqual(removeAppInboxRouteRequest("todo", "mail"), { RemoveAppInboxRoute: { installation_id: "todo", route_id: "mail" } })
  assert.deepEqual(testAppInboxRouteRequest("todo", "mail", "occ-1", { title: "x" }),
    { TestAppInboxRoute: { installation_id: "todo", route_id: "mail", occurrence_id: "occ-1", payload: { title: "x" } } })
})

test("file grants carry names and bytes for one pending request, never a path", () => {
  assert.deepEqual(grantAppFileRequest("s", "file-pick-1", [{ name: "notes.md", contentsBase64: "IyBO" }]), {
    GrantAppFile: { session_id: "s", operation_id: "file-pick-1", files: [{ name: "notes.md", contents_base64: "IyBO" }] },
  })
})

test("saving an offered App file names only the session and the offer", () => {
  assert.deepEqual(saveAppFileExportRequest("s", "file-export-1"), { SaveAppFileExport: { session_id: "s", operation_id: "file-export-1" } })
})
