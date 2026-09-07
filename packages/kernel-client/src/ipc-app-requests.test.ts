import assert from "node:assert/strict"
import { test } from "node:test"
import { getAppInstallationJournalRequest, getAppInstallationRequest, listAppInstallationsRequest } from "./ipc-app-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"

test("App inspection shares protocol 288 without client owner or host paths", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 288)
  assert.deepEqual(listAppInstallationsRequest(), { ListAppInstallations: { after: null, limit: null } })
  assert.deepEqual(listAppInstallationsRequest({ after: "todo", limit: 1 }), { ListAppInstallations: { after: "todo", limit: 1 } })
  assert.deepEqual(getAppInstallationRequest("todo"), { GetAppInstallation: { installation_id: "todo" } })
  assert.deepEqual(getAppInstallationJournalRequest("todo"), { GetAppInstallationJournal: { installation_id: "todo" } })
})
