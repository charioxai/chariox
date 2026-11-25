import assert from "node:assert/strict"
import test from "node:test"
import { kernelBrowserAgentTabsMinimumProtocolVersion, kernelBrowserRoomDownloadsMinimumProtocolVersion, kernelBrowserMinimumProtocolVersion, kernelBrowserRequest, kernelDisplayResumeMinimumProtocolVersion, kernelTabDisplayMinimumProtocolVersion } from "./ipc-kernel-browser-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"

test("MD-2: protocol 417 browser requests carry no session or claimed user", () => {

  assert.equal(kernelBrowserMinimumProtocolVersion, 443)
  assert.deepEqual(kernelBrowserRequest({ op: "open", url: "https://example.com" }), {
    KernelBrowser: { command: { op: "open", url: "https://example.com" } },
  })
  assert.deepEqual(kernelBrowserRequest({ op: "input", tab_id: "host-tab-1", generation: 2, input: { kind: "text", text: "fixture" } }), {
    KernelBrowser: { command: { op: "input", tab_id: "host-tab-1", generation: 2, input: { kind: "text", text: "fixture" } } },
  })
})

test("a Room's browser input, downloads wait and reveal are typed commands", () => {
  assert.equal(kernelBrowserRoomDownloadsMinimumProtocolVersion, 1015)
  assert(LOCAL_DAEMON_PROTOCOL_VERSION >= kernelBrowserRoomDownloadsMinimumProtocolVersion)
  assert.deepEqual(kernelBrowserRequest({ op: "room", session_id: "room-1", command: { op: "navigate", tab_id: "t", generation: 1, url: "https://example.com/a.zip" } }), {
    KernelBrowser: { command: { op: "room", session_id: "room-1", command: { op: "navigate", tab_id: "t", generation: 1, url: "https://example.com/a.zip" } } },
  })
  assert.deepEqual(kernelBrowserRequest({ op: "downloads_wait", session_id: "room-1", wait_ms: 0 }), {
    KernelBrowser: { command: { op: "downloads_wait", session_id: "room-1", wait_ms: 0 } },
  })
  assert.deepEqual(kernelBrowserRequest({ op: "reveal_download", session_id: "room-1", download_id: "d1" }), {
    KernelBrowser: { command: { op: "reveal_download", session_id: "room-1", download_id: "d1" } },
  })
  // @ts-expect-error a Room scope carries navigation or input only
  kernelBrowserRequest({ op: "room", session_id: "room-1", command: { op: "stop" } })
})

// MP-08/MP-11: the agent tab projection reaches the merged line at 507.
test("MP-08 agent tab projection retains protocol 486", () => {
  assert.equal(kernelBrowserAgentTabsMinimumProtocolVersion, 486)
  assert(LOCAL_DAEMON_PROTOCOL_VERSION >= kernelBrowserAgentTabsMinimumProtocolVersion)
})

test("MP-08/MP-10/MP-11 shared display resumption pins allocated protocol 525", () => {
  assert.equal(kernelDisplayResumeMinimumProtocolVersion, 525)
  assert(LOCAL_DAEMON_PROTOCOL_VERSION >= kernelDisplayResumeMinimumProtocolVersion)
  for (const op of ["display_suspend", "display_resume"] as const) {
    assert.deepEqual(kernelBrowserRequest({ op, subscription_id: "prior-delivery", generation: 7 }), {
      KernelBrowser: { command: { op, subscription_id: "prior-delivery", generation: 7 } },
    })
  }
})

test("MP-08/MP-10 Tab display requests pin allocated protocol 720", () => {
  assert.equal(kernelTabDisplayMinimumProtocolVersion, 720)
  assert(LOCAL_DAEMON_PROTOCOL_VERSION >= kernelTabDisplayMinimumProtocolVersion)
  assert.deepEqual(kernelBrowserRequest({ op: "display_park", subscription_id: "d", generation: 7, parked: true }), {
    KernelBrowser: { command: { op: "display_park", subscription_id: "d", generation: 7, parked: true } },
  })
  assert.deepEqual(kernelBrowserRequest({ op: "display_subscribe", tab_id: "t", generation: 7, codecs: ["png", "avc1.420033"], bitrate: 8_000_000, device_scale_factor: 1, tab_display: true }), {
    KernelBrowser: { command: { op: "display_subscribe", tab_id: "t", generation: 7, codecs: ["png", "avc1.420033"], bitrate: 8_000_000, device_scale_factor: 1, tab_display: true } },
  })
})

// MP-08/MP-10: protocol 629 capabilities already exist on the Rust wire.
test("viewport-first and visited-link subscriptions retain their opt-in fields", () => {
  assert.deepEqual(kernelBrowserRequest({ op: "mirror_subscribe", tab_id: "t", generation: 1, device_scale_factor: 1, wire: 2, viewport_first: true, visited_links: true }), {
    KernelBrowser: { command: { op: "mirror_subscribe", tab_id: "t", generation: 1, device_scale_factor: 1, wire: 2, viewport_first: true, visited_links: true } },
  })
})
