import assert from "node:assert/strict"
import test from "node:test"

import {
  exportWorkflowPublicationPackageRequest,
} from "./ipc-workflow-requests.js"

test("export workflow publication package request matches kernel shape", () => {
  assert.deepEqual(exportWorkflowPublicationPackageRequest("session-1", "publication-1", {
    kernelUrl: "ws://127.0.0.1:43118",
  }), {
    ExportWorkflowPublicationPackage: {
      session_id: "session-1",
      publication_ref: "publication-1",
      kernel_url: "ws://127.0.0.1:43118",
    },
  })
})
