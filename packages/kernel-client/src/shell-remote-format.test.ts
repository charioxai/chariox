import assert from "node:assert/strict"
import test from "node:test"

import {
  formatRemoteKernels,
  formatRemoteMachines,
  remoteKernelReadiness,
  remoteKernelReadinessCounts,
} from "./shell-remote-format.js"

test("remote machine formatter shows provider account identities", () => {
  const output = formatRemoteMachines([{
    machine_id: "machine-1",
    machine_alias: "mini",
    display_name: "mini",
    trust_status: "approved",
    online: true,
    pending: false,
    kernel_count: 2,
    available_providers: ["codex", "opencode"],
    provider_accounts: [{
      provider: "codex",
      state: "authenticated",
      alias: "daily",
      email: "dev@example.com",
    }, {
      provider: "opencode",
      state: "not_configured",
      auth_type: "api",
    }],
  }, {
    machine_id: "machine-2",
    display_name: "offline-box",
    trust_status: "approved",
    online: false,
    pending: false,
    kernel_count: 0,
    available_providers: [],
    provider_accounts: [],
  }])

  assert.match(output, /mini id=machine-1 status=approved kernels=2 providers=codex,opencode accounts=codex=daily \(dev@example.com\),opencode=api\/state=not_configured next: run \/machine kernels mini; configure\/import or refresh provider accounts before spawning remote agents/)
  assert.match(output, /offline-box id=machine-2 status=approved,offline kernels=0 providers=- accounts=none next: connect or restart the remote kernel on this machine/)
})

test("remote kernel formatter summarizes worker readiness", () => {
  const output = formatRemoteKernels([
    {
      kernel_id: "kernel-ready",
      machine_id: "machine-1",
      machine_alias: "mini",
      relay_alias: "ready-kernel",
      available_providers: ["codex"],
      provider_accounts: [{
        provider: "codex",
        state: "not_configured",
        alias: "daily",
      }],
      accepting_remote_leases: true,
      leased_agent_count: 1,
      local_session_count: 2,
    },
    {
      kernel_id: "kernel-blocked",
      machine_id: "machine-1",
      machine_alias: "mini",
      relay_alias: "blocked-kernel",
      available_providers: ["opencode"],
      provider_accounts: [{
        provider: "opencode",
        state: "not_configured",
        auth_type: "api",
      }],
      accepting_remote_leases: false,
    },
    {
      kernel_id: "kernel-provider",
      machine_id: "machine-1",
      machine_alias: "mini",
      relay_alias: "provider-kernel",
      available_providers: [],
      accepting_remote_leases: true,
    },
    {
      kernel_id: "kernel-unknown",
      machine_id: "machine-1",
      machine_alias: "mini",
      relay_alias: "unknown-kernel",
      available_providers: ["claude"],
    },
  ], "machine-1")

  assert.match(output, /^machine machine-1 worker readiness: 0\/4 ready, 1 needs provider, 1 needs account, 1 blocked, 1 unknown; next: fix the listed kernel readiness issue or choose another machine/)
  assert.match(output, /ready-kernel id=kernel-ready machine=mini readiness=needs-account providers=codex accounts=codex=daily \(auth missing\)\/state=not_configured accepting_remote_leases=true leased_agents=1 local_sessions=2 next: configure\/import or refresh provider accounts on ready-kernel before spawning remote agents/)
  assert.match(output, /blocked-kernel id=kernel-blocked machine=mini readiness=blocked providers=opencode accounts=opencode=api\/state=not_configured accepting_remote_leases=false leased_agents=0 local_sessions=0 next: run \/machine kernels mini; enable remote leases on blocked-kernel or choose another worker/)
  assert.match(output, /provider-kernel id=kernel-provider machine=mini readiness=needs-provider providers=- accounts=none accepting_remote_leases=true leased_agents=0 local_sessions=0 next: run \/machine kernels mini; configure provider CLIs on provider-kernel/)
  assert.match(output, /unknown-kernel id=kernel-unknown machine=mini readiness=unknown providers=claude accounts=none accepting_remote_leases=unknown leased_agents=0 local_sessions=0 next: run \/machine kernels mini; refresh unknown-kernel readiness or reconnect that worker before launching remote agents/)
})

test("remote kernel formatter makes empty machine recovery actionable", () => {
  assert.equal(
    formatRemoteKernels([], "machine-1"),
    "no live kernels found for machine machine-1; next: reconnect that machine or choose another worker",
  )
})

test("remote kernel readiness helpers classify worker state", () => {
  const kernels = [
    { kernel_id: "ready", machine_id: "machine", available_providers: ["codex"], provider_accounts: [{ provider: "codex", state: "authenticated" }], accepting_remote_leases: true },
    { kernel_id: "blocked", machine_id: "machine", available_providers: ["codex"], accepting_remote_leases: false },
    { kernel_id: "provider", machine_id: "machine", available_providers: [], accepting_remote_leases: true },
    { kernel_id: "account", machine_id: "machine", available_providers: ["codex"], provider_accounts: [], accepting_remote_leases: true },
    { kernel_id: "unknown", machine_id: "machine", available_providers: ["codex"] },
  ]

  assert.deepEqual(kernels.map((kernel) => remoteKernelReadiness(kernel)), [
    "ready",
    "blocked",
    "needs-provider",
    "needs-account",
    "unknown",
  ])
  assert.deepEqual(remoteKernelReadinessCounts(kernels), {
    ready: 1,
    blocked: 1,
    "needs-provider": 1,
    "needs-account": 1,
    unknown: 1,
  })
})
