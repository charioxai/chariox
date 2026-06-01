import assert from "node:assert/strict"
import test from "node:test"

import { handleRemoteMachineSlashCommand, type RemoteMachineCommandHandlerDeps } from "./remote-machine-command-handlers.js"

test("remote machine command renders recovery hints", async () => {
  const harness = remoteMachineHarness()

  await handleRemoteMachineSlashCommand(harness.deps, command("list"))
  await handleRemoteMachineSlashCommand(harness.deps, command("kernels", "machine-1"))

  assert.match(harness.notices.at(0) ?? "", /cold id=machine-2 status=pending/)
  assert.match(harness.notices.at(0) ?? "", /next: approve with \/machine approve machine-2/)
  assert.match(harness.notices.at(1) ?? "", /cold-kernel id=kernel-2/)
  assert.match(harness.notices.at(1) ?? "", /accepting_remote_leases=false/)
  assert.match(harness.notices.at(1) ?? "", /next: enable remote leases on cold-kernel or choose another worker/)
  assert.equal(harness.footers.at(0)?.message, "listed 2 live remote machine(s)")
  assert.equal(harness.footers.at(1)?.message, "listed 1 live kernel(s) for machine-1")
})

test("remote machine command renders unknown lease state without a false recovery hint", async () => {
  const harness = remoteMachineHarness({
    accepting_remote_leases: undefined,
    available_providers: ["codex"],
  })

  await handleRemoteMachineSlashCommand(harness.deps, command("kernels", "machine-1"))

  assert.match(harness.notices.at(0) ?? "", /accepting_remote_leases=unknown/)
  assert.doesNotMatch(harness.notices.at(0) ?? "", /enable remote leases/)
})

function command(...args: string[]) {
  return { kind: "machine" as const, args, raw: `/machine ${args.join(" ")}` }
}

function remoteMachineHarness(kernelOverrides: Record<string, unknown> = {}) {
  const notices: string[] = []
  const footers: Array<{ message: string; tone: "info" | "error" }> = []
  const deps: RemoteMachineCommandHandlerDeps = {
    flashFooter: (message, tone) => { footers.push({ message, tone }) },
    appendNotice: (message) => { notices.push(message) },
    listRemoteMachines: async () => [{
      machine_id: "machine-1",
      machine_alias: "builder",
      registry_alias: null,
      display_name: "Builder",
      trust_status: "approved",
      online: true,
      pending: false,
      kernel_count: 1,
      available_providers: ["codex"],
    }, {
      machine_id: "machine-2",
      machine_alias: "cold",
      registry_alias: null,
      display_name: "cold",
      trust_status: "pending",
      online: true,
      pending: true,
      kernel_count: 0,
      available_providers: [],
    }],
    listRemoteMachineKernels: async () => [{
      kernel_id: "kernel-2",
      machine_id: "machine-1",
      machine_alias: "builder",
      relay_alias: "cold-kernel",
      kernel_alias: null,
      accepting_remote_leases: false,
      leased_agent_count: 0,
      local_session_count: 0,
      available_providers: [],
      ...kernelOverrides,
    }],
  }
  return { deps, notices, footers }
}
