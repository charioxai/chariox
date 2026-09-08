import assert from "node:assert/strict"
import { test } from "node:test"
import { abortAppPackageUploadRequest, beginAppPackageUploadRequest, getAppPackageUploadRequest, putAppPackageUploadChunkRequest, getAppInstallationJournalRequest, getAppInstallationRequest, listAppInstallationsRequest } from "./ipc-app-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"

test("App inspection shares protocol 292 without client owner or host paths", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 292)
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
