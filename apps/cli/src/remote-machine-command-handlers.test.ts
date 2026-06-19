import assert from "node:assert/strict"
import test from "node:test"

import {
  handleRemoteMachineSlashCommand,
  type RemoteMachineCommandHandlerDeps,
  type RemoteMachineSummary,
} from "./remote-machine-command-handlers.js"

test("remote machine command renders recovery hints", async () => {
  const harness = remoteMachineHarness()

  await handleRemoteMachineSlashCommand(harness.deps, command("list"))
  await handleRemoteMachineSlashCommand(harness.deps, command("kernels", "machine-1"))

  assert.match(harness.notices.at(0) ?? "", /cold id=machine-2 status=pending/)
  assert.match(harness.notices.at(0) ?? "", /next: approve with \/machine approve machine-2/)
  assert.match(harness.notices.at(1) ?? "", /cold-kernel id=kernel-2/)
  assert.match(harness.notices.at(1) ?? "", /readiness=blocked/)
  assert.match(harness.notices.at(1) ?? "", /accepting_remote_leases=false/)
  assert.match(harness.notices.at(1) ?? "", /next: run \/machine kernels builder; enable remote leases on cold-kernel or choose another worker/)
  assert.equal(harness.footers.at(0)?.message, "listed 2 live remote machine(s)")
  assert.equal(harness.footers.at(1)?.message, "listed 1 live kernel(s) for machine-1")
})

test("remote machine command renders unknown lease state with refresh recovery", async () => {
  const harness = remoteMachineHarness({
    accepting_remote_leases: undefined,
    available_providers: ["codex"],
  })

  await handleRemoteMachineSlashCommand(harness.deps, command("kernels", "machine-1"))

  assert.match(harness.notices.at(0) ?? "", /accepting_remote_leases=unknown/)
  assert.match(harness.notices.at(0) ?? "", /readiness=unknown/)
  assert.doesNotMatch(harness.notices.at(0) ?? "", /enable remote leases/)
  assert.match(harness.notices.at(0) ?? "", /next: run \/machine kernels builder; refresh cold-kernel readiness or reconnect that worker before launching remote agents/)
})

test("remote machine command prioritizes unknown readiness over empty providers", async () => {
  const harness = remoteMachineHarness({
    accepting_remote_leases: undefined,
    available_providers: [],
  })

  await handleRemoteMachineSlashCommand(harness.deps, command("kernels", "machine-1"))

  assert.match(harness.notices.at(0) ?? "", /readiness=unknown/)
  assert.match(harness.notices.at(0) ?? "", /providers=-/)
  assert.match(harness.notices.at(0) ?? "", /next: run \/machine kernels builder; refresh cold-kernel readiness or reconnect that worker before launching remote agents/)
  assert.doesNotMatch(harness.notices.at(0) ?? "", /configure provider CLIs/)
})

test("remote machine mutation commands patch cached machines without refreshing inventory", async () => {
  const harness = remoteMachineHarness()

  await handleRemoteMachineSlashCommand(harness.deps, command("approve", "machine-2"))
  await handleRemoteMachineSlashCommand(harness.deps, command("rename", "machine-2", "New", "Name"))
  await handleRemoteMachineSlashCommand(harness.deps, command("forget", "machine-2"))

  assert.deepEqual(harness.machines.map((machine) => [
    machine.machine_id,
    machine.display_name,
    machine.trust_status,
  ]), [
    ["machine-1", "Builder", "approved"],
  ])
  assert.equal(harness.reconcileCount, 3)
  assert.equal(harness.footers.at(0)?.message, "approved remote machine cold")
  assert.equal(harness.footers.at(1)?.message, "renamed remote machine machine-2 to New Name")
  assert.equal(harness.footers.at(2)?.message, "forgot remote machine New Name")
})

function command(...args: string[]) {
  return { kind: "machine" as const, args, raw: `/machine ${args.join(" ")}` }
}

function remoteMachineHarness(kernelOverrides: Record<string, unknown> = {}) {
  const notices: string[] = []
  const footers: Array<{ message: string; tone: "info" | "error" }> = []
  let reconcileCount = 0
  const machines: RemoteMachineSummary[] = [{
    machine_id: "machine-1",
    machine_alias: "builder",
    registry_alias: null,
    display_name: "Builder",
    trust_status: "approved" as const,
    online: true,
    pending: false,
    kernel_count: 1,
    available_providers: ["codex"],
  }, {
    machine_id: "machine-2",
    machine_alias: "cold",
    registry_alias: null,
    display_name: "cold",
    trust_status: "pending" as const,
    online: true,
    pending: true,
    kernel_count: 0,
    available_providers: [],
  }]
  const deps: RemoteMachineCommandHandlerDeps = {
    flashFooter: (message, tone) => { footers.push({ message, tone }) },
    appendNotice: (message) => { notices.push(message) },
    refreshWaitingRoomData: async () => {
      throw new Error("should not refresh waiting-room inventory after remote machine mutation")
    },
    getRemoteMachines: () => machines,
    setRemoteMachines: (next) => {
      machines.splice(0, machines.length, ...next)
    },
    reconcileWaitingRoom: () => {
      reconcileCount += 1
    },
    listRemoteMachines: async () => machines,
    approveRemoteMachine: async (machineRef) => ({
      machine_id: machineRef,
      display_name: "cold",
      trust_status: "approved",
      online: true,
    }),
    forgetRemoteMachine: async (machineRef) => ({
      machine_id: machineRef,
      display_name: "New Name",
      trust_status: "forgotten",
      online: false,
    }),
    renameRemoteMachine: async (machineRef, alias) => ({
      machine_id: machineRef,
      display_name: alias,
      trust_status: "approved",
      online: true,
    }),
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
  return {
    deps,
    notices,
    footers,
    machines,
    get reconcileCount() {
      return reconcileCount
    },
  }
}
