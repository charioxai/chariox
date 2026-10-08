import assert from "node:assert/strict"
import { test } from "node:test"
import { abortAppPackageUploadRequest, beginAppPackageUploadRequest, getAppPackageUploadRequest, putAppPackageUploadChunkRequest, getAppInstallationJournalRequest, getAppInstallationRequest, listAppInstallationsRequest } from "./ipc-app-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"
import { beginAppInstallRequest, beginAppUpdateRequest, getAppInstallOperationRequest, cancelAppInstallOperationRequest } from "./ipc-app-requests.js"
import { beginAppPublisherEnrollmentRequest, getAppPublisherEnrollmentRequest, cancelAppPublisherEnrollmentRequest } from "./ipc-app-requests.js"
import { createAppInboxRouteRequest, listAppInboxRoutesRequest, removeAppInboxRouteRequest, testAppInboxRouteRequest } from "./ipc-app-requests.js"
import { grantAppFileRequest, saveAppFileExportRequest } from "./ipc-app-requests.js"
import { prepareDeploymentAppsRequest, previewDeploymentAppsRequest } from "./ipc-app-requests.js"

test("App inspection shares protocol 297 without client owner or host paths", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 473)
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

test("an owner revokes an installation's file requests and unused grants (protocol 394)", async () => {
  const { revokeAppFileGrantsRequest } = await import("./ipc-app-requests.js")
  assert.deepEqual(revokeAppFileGrantsRequest("docs"), { RevokeAppFileGrants: { installation_id: "docs" } })
  assert.deepEqual(revokeAppFileGrantsRequest("docs", "file-pick-1"),
    { RevokeAppFileGrants: { installation_id: "docs", operation_id: "file-pick-1" } })
  const { executeAppCommand } = await import("./shell-app-command.js")
  const sent: unknown[] = []
  const none = await executeAppCommand(["file", "revoke", "docs"], {
    send: async (request) => {
      sent.push(request)
      return { AppFileGrantsRevoked: { installation_id: "docs", requests: 0, files: 0 } }
    },
  })
  assert.deepEqual(sent, [{ RevokeAppFileGrants: { installation_id: "docs" } }])
  assert.equal(none.message, "No open file requests or unused grants for docs.")
  const missing = await executeAppCommand(["file", "revoke", "docs", "file-pick-9"], {
    send: async () => ({ AppRequestFailed: { code: "not_found" } }),
  })
  assert.deepEqual([missing.ok, missing.message], [false, "Not found: check the App installation and the file request id."])
  assert.equal((await executeAppCommand(["file", "revoke"], { send: async () => ({}) })).ok, false)
  assert.equal((await executeAppCommand(["file", "revoke", "docs", "a", "b"], { send: async () => ({}) })).ok, false)
})

test("saving an offered App file names only the session and the offer", () => {
  assert.deepEqual(saveAppFileExportRequest("s", "file-export-1"), { SaveAppFileExport: { session_id: "s", operation_id: "file-export-1" } })
})

test("an inbox route can subscribe to an event generator connection (protocol 358)", async () => {
  assert.deepEqual(createAppInboxRouteRequest({ installationId: "slack", routeId: "mentions", eventName: "mentioned",
    sourceEventType: "app.mentioned", sourceEventVersion: 1,
    connection: { generatorId: "dev.chariox.slack", connectionId: "connection-1", connectionScope: "team:T1" } }),
  { CreateAppInboxRoute: { installation_id: "slack", route_id: "mentions", event_name: "mentioned",
    source_event_type: "app.mentioned", source_event_version: 1,
    connection: { generator_id: "dev.chariox.slack", connection_id: "connection-1", connection_scope: "team:T1" } } })
  const { inboxRequest } = await import("./shell-app-command.js")
  assert.deepEqual(inboxRequest(["add", "slack", "mentions", "mentioned", "app.mentioned",
    "--connection", "dev.chariox.slack/connection-1/team:T1"]),
  { CreateAppInboxRoute: { installation_id: "slack", route_id: "mentions", event_name: "mentioned",
    source_event_type: "app.mentioned", source_event_version: 1,
    connection: { generator_id: "dev.chariox.slack", connection_id: "connection-1", connection_scope: "team:T1" } } })
  assert.equal(inboxRequest(["add", "slack", "r", "e", "t", "--connection", "only-generator"]), null)
})

test("an owner grants, lists and revokes an App's generator connections (protocol 359)", async () => {
  const { grantAppConnectionRequest, revokeAppConnectionRequest, listAppConnectionsRequest } = await import("./ipc-app-requests.js")
  assert.deepEqual(grantAppConnectionRequest("slack", "dev.chariox.slack", "connection-1"),
    { GrantAppConnection: { installation_id: "slack", generator_id: "dev.chariox.slack", connection_id: "connection-1" } })
  assert.deepEqual(revokeAppConnectionRequest("slack", "connection-1"),
    { RevokeAppConnection: { installation_id: "slack", connection_id: "connection-1" } })
  assert.deepEqual(listAppConnectionsRequest("slack"), { ListAppConnections: { installation_id: "slack" } })
  const { executeAppCommand } = await import("./shell-app-command.js")
  const sent: unknown[] = []
  const result = await executeAppCommand(["connection", "grant", "slack", "dev.chariox.slack/connection-1"], {
    send: async (request) => {
      sent.push(request)
      return { AppConnections: { installation_id: "slack", connections: [
        { generator_id: "dev.chariox.slack", connection_id: "connection-1", granted_at_ms: 1, actions: ["notification.reply"] }] } }
    },
  })
  assert.deepEqual(sent, [grantAppConnectionRequest("slack", "dev.chariox.slack", "connection-1")])
  assert.equal(result.message, "connection-1 · dev.chariox.slack · actions: notification.reply")
  assert.equal((await executeAppCommand(["connection", "grant", "slack", "no-slash"], { send: async () => ({}) })).ok, false)
  const undeclared = await executeAppCommand(["connection", "grant", "slack", "dev.chariox.other/connection-1"], {
    send: async () => ({ AppRequestFailed: { code: "invalid_request" } }),
  })
  assert.match(undeclared.message ?? "", /must declare this generator under capabilities\.connections/)
  // Only a grant checks the manifest; list and revoke refusals stay neutral.
  const listed = await executeAppCommand(["connection", "list", "slack"], {
    send: async () => ({ AppRequestFailed: { code: "invalid_request" } }),
  })
  assert.equal(listed.message, "Invalid App request.")
})

test("app set shows each active installation's release and configuration (protocol 361)", async () => {
  const { executeAppCommand } = await import("./shell-app-command.js")
  const sent: unknown[] = []
  const result = await executeAppCommand(["set"], { send: async (request) => {
    sent.push(request)
    return { AppSet: { schema: "chariox.app-set.v1", installations: [{
      installation_id: "slack", app_id: "com.chariox.slack", release: { version: "1.1.0" }, capabilities: {},
      automations: [{}], inbox_routes: [{}, {}], connections: [] }] } }
  } })
  assert.deepEqual(sent, [{ GetAppSet: {} }])
  assert.equal(result.message, "App set (chariox.app-set.v1): 1 active installation\nslack · com.chariox.slack v1.1.0 · 1 automation, 2 inbox routes, 0 connections")
  assert.equal((await executeAppCommand(["set", "extra"], { send: async () => ({}) })).ok, false)
})


test("deployment App consent names the release and carries no approval", () => {
  const request = prepareDeploymentAppsRequest({
    sessionId: "session", requestId: "deploy-apps-1", publicationRef: "publication-1",
    deploymentId: "deployment-1", releaseId: "release-1", packageDigest: `sha256:${"a".repeat(64)}`,
  })
  assert.deepEqual(request, { PrepareDeploymentApps: {
    session_id: "session", request_id: "deploy-apps-1", publication_ref: "publication-1",
    deployment_id: "deployment-1", release_id: "release-1", package_digest: `sha256:${"a".repeat(64)}`,
  } })
  assert.deepEqual(Object.keys(request.PrepareDeploymentApps).sort(), ["deployment_id", "package_digest", "publication_ref", "release_id", "request_id", "session_id"])
})

test("deployment App preview names the publication and optionally a release", () => {
  assert.deepEqual(previewDeploymentAppsRequest("session", "publication-1"), {
    PreviewDeploymentApps: { session_id: "session", publication_ref: "publication-1" },
  })
  assert.deepEqual(previewDeploymentAppsRequest("session", "publication-1", "sha256:release"), {
    PreviewDeploymentApps: { session_id: "session", publication_ref: "publication-1", package_digest: "sha256:release" },
  })
})

test("App host acceptance names only the session and operation, never a client payload or owner", async () => {
  const { acceptAppHostActionRequest } = await import("./ipc-app-requests.js")
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 473)
  assert.deepEqual(acceptAppHostActionRequest("s", "offer"), { AcceptAppHostAction: { session_id: "s", operation_id: "offer" } })
})

import { restoreAppDataSnapshotRequest } from "./ipc-app-requests.js"
test("snapshot restore binds installation, generation and saved identity without authority", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 473)
  assert.deepEqual(restoreAppDataSnapshotRequest("todo", "3", "snapshot-1"), {
    RestoreAppDataSnapshot: { installation_id: "todo", expected_generation: "3", snapshot_id: "snapshot-1" },
  })
})
