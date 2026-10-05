import assert from "node:assert/strict"
import test from "node:test"
import { kernelBrowserMinimumProtocolVersion, kernelBrowserRequest } from "./ipc-kernel-browser-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"

test("MD-2: protocol 417 browser requests carry no session or claimed user", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 425)
  assert.equal(kernelBrowserMinimumProtocolVersion, 417)
  assert.deepEqual(kernelBrowserRequest({ op: "open", url: "https://example.com" }), {
    KernelBrowser: { command: { op: "open", url: "https://example.com" } },
  })
  assert.deepEqual(kernelBrowserRequest({ op: "input", tab_id: "host-tab-1", generation: 2, input: { kind: "text", text: "fixture" } }), {
    KernelBrowser: { command: { op: "input", tab_id: "host-tab-1", generation: 2, input: { kind: "text", text: "fixture" } } },
  })
})
