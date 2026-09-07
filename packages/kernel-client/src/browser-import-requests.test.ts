import assert from "node:assert/strict"
import test from "node:test"
import { prepareBrowserImportRequest, approveBrowserImportRequest, cancelBrowserImportRequest,
  browserImportConsentMinimumProtocolVersion } from "./browser-import-requests.js"

test("import consent serializes only explicit metadata and requires protocol 313", () => {
  const expected = { session_id: "room-1", attachment_id: "attachment-1", environment_id: "environment-1",
    runtime_generation: 2, tab_id: "tab-1", document_revision: 3, source_store_id: "0",
    domains: ["example.com"], partition_sites: [], overwrite: false }
  const source = { ...expected, cookies: [{ name: "private", value: "must-not-travel" }] }
  assert.equal(browserImportConsentMinimumProtocolVersion, 313)
  const prepared = prepareBrowserImportRequest(source)
  assert.deepEqual(prepared, { PrepareBrowserImport: { selection: expected } })
  assert.deepEqual(approveBrowserImportRequest("request-1", source), {
    ApproveBrowserImport: { request_id: "request-1", selection: expected },
  })
  assert.deepEqual(cancelBrowserImportRequest("room-1", "attachment-1", "request-1"), {
    CancelBrowserImport: { session_id: "room-1", attachment_id: "attachment-1", request_id: "request-1" },
  })
  source.domains.push("unapproved.example")
  assert.deepEqual(prepared.PrepareBrowserImport.selection.domains, ["example.com"])
})
