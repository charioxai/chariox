import assert from "node:assert/strict"
import test from "node:test"

import { formatRemoteKernels } from "./shell-remote-format.js"

test("remote kernel formatter summarizes worker readiness", () => {
  const output = formatRemoteKernels([
    {
      kernel_id: "kernel-ready",
      machine_id: "machine-1",
      machine_alias: "mini",
      relay_alias: "ready-kernel",
      available_providers: ["codex"],
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

  assert.match(output, /^machine machine-1 worker readiness: 1\/4 ready, 1 needs provider, 1 blocked, 1 unknown; next: spawn a remote agent with a ready worker kernel/)
  assert.match(output, /ready-kernel id=kernel-ready machine=mini readiness=ready providers=codex accepting_remote_leases=true leased_agents=1 local_sessions=2/)
  assert.match(output, /blocked-kernel id=kernel-blocked machine=mini readiness=blocked providers=opencode accepting_remote_leases=false leased_agents=0 local_sessions=0 next: run \/machine kernels mini; enable remote leases on blocked-kernel or choose another worker/)
  assert.match(output, /provider-kernel id=kernel-provider machine=mini readiness=needs-provider providers=- accepting_remote_leases=true leased_agents=0 local_sessions=0 next: run \/machine kernels mini; configure provider CLIs on provider-kernel/)
  assert.match(output, /unknown-kernel id=kernel-unknown machine=mini readiness=unknown providers=claude accepting_remote_leases=unknown leased_agents=0 local_sessions=0 next: run \/machine kernels mini; refresh unknown-kernel readiness or reconnect that worker before launching remote agents/)
})

test("remote kernel formatter makes empty machine recovery actionable", () => {
  assert.equal(
    formatRemoteKernels([], "machine-1"),
    "no live kernels found for machine machine-1; next: reconnect that machine or choose another worker",
  )
})
