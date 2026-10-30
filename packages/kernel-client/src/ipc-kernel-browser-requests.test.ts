import assert from "node:assert/strict"
import test from "node:test"
import { kernelBrowserAgentTabsMinimumProtocolVersion, kernelBrowserMinimumProtocolVersion, kernelBrowserRequest, kernelDisplayResumeMinimumProtocolVersion, kernelTabDisplayMinimumProtocolVersion, userDomainAccessMinimumProtocolVersion } from "./ipc-kernel-browser-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"
import { userDomainWindowBadge } from "./index.js"

test("MD-2: protocol 417 browser requests carry no session or claimed user", () => {

  assert.equal(kernelBrowserMinimumProtocolVersion, 443)
  assert.deepEqual(kernelBrowserRequest({ op: "open", url: "https://example.com" }), {
    KernelBrowser: { command: { op: "open", url: "https://example.com" } },
  })
  assert.deepEqual(kernelBrowserRequest({ op: "input", tab_id: "host-tab-1", generation: 2, input: { kind: "text", text: "fixture" } }), {
    KernelBrowser: { command: { op: "input", tab_id: "host-tab-1", generation: 2, input: { kind: "text", text: "fixture" } } },
  })
})

test("MP-08/MP-11: grant requests and cross-kernel badge retain protocol 443", () => {
  assert.equal(userDomainAccessMinimumProtocolVersion, 443)
  assert.deepEqual(kernelBrowserRequest({ op: "revoke_grants", agent_id: null }), { KernelBrowser: { command: { op: "revoke_grants", agent_id: null } } })
  assert.deepEqual(kernelBrowserRequest({ op: "subscribe_grants", after: 3, wait_ms: 25000 }), { KernelBrowser: { command: { op: "subscribe_grants", after: 3, wait_ms: 25000 } } })
  const access = { kernel_id: "home", kernel_name: "home", focused_agent_kernel_id: "worker", reachable_by_focused_agent: false }
  assert.equal(userDomainWindowBadge(access), "Your focused agent can't control this window — focus an agent on kernel home")
  assert.equal(userDomainWindowBadge({ ...access, focused_agent_kernel_id: "home", reachable_by_focused_agent: true }), null)
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
