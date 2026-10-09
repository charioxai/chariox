import assert from "node:assert/strict"
import test from "node:test"
import { kernelBrowserAgentTabsMinimumProtocolVersion, kernelBrowserMinimumProtocolVersion, kernelBrowserRequest, userDomainAccessMinimumProtocolVersion } from "./ipc-kernel-browser-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"
import { userDomainWindowBadge } from "./index.js"

test("MD-2: protocol 417 browser requests carry no session or claimed user", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 481)
  assert.equal(kernelBrowserMinimumProtocolVersion, 443)
  assert.deepEqual(kernelBrowserRequest({ op: "open", url: "https://example.com" }), {
    KernelBrowser: { command: { op: "open", url: "https://example.com" } },
  })
  assert.deepEqual(kernelBrowserRequest({ op: "input", tab_id: "host-tab-1", generation: 2, input: { kind: "text", text: "fixture" } }), {
    KernelBrowser: { command: { op: "input", tab_id: "host-tab-1", generation: 2, input: { kind: "text", text: "fixture" } } },
  })
})

test("MP-08/MP-11: grant requests and cross-kernel badge share protocol 443", () => {
  assert.equal(userDomainAccessMinimumProtocolVersion, 443)
  assert.deepEqual(kernelBrowserRequest({ op: "revoke_grants", agent_id: null }), { KernelBrowser: { command: { op: "revoke_grants", agent_id: null } } })
  assert.deepEqual(kernelBrowserRequest({ op: "subscribe_grants", after: 3, wait_ms: 25000 }), { KernelBrowser: { command: { op: "subscribe_grants", after: 3, wait_ms: 25000 } } })
  const access = { kernel_id: "home", kernel_name: "home", focused_agent_kernel_id: "worker", reachable_by_focused_agent: false }
  assert.equal(userDomainWindowBadge(access), "Your focused agent can't control this window — focus an agent on kernel home")
  assert.equal(userDomainWindowBadge({ ...access, focused_agent_kernel_id: "home", reachable_by_focused_agent: true }), null)
})

// MP-08/MP-11: PR937 already published481. Keep this RED until the coordinator
// assigns the added projection a fresh version and both clients pin it.
test("MP-08 agent tab projection requires a fresh protocol after published PR937", () => {
  assert(kernelBrowserAgentTabsMinimumProtocolVersion > 481, "Coordinator must allocate the new tab projection protocol before landing")
  assert(LOCAL_DAEMON_PROTOCOL_VERSION >= kernelBrowserAgentTabsMinimumProtocolVersion)
})
