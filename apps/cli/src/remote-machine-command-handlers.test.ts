import assert from "node:assert/strict"
import test from "node:test"
import { deriveWaitingRoomControlActivationDecision } from "./waiting-room-controller.js"
import { parseSlashCommand } from "./commands.js"

import {
  handleRemoteMachineSlashCommand,
  type RemoteMachineCommandHandlerDeps,
  type RemoteMachineSummary,
} from "./remote-machine-command-handlers.js"

test("remote machine command renders recovery hints", async () => {
  const harness = remoteMachineHarness()

  await handleRemoteMachineSlashCommand(harness.deps, command("list"))
  await handleRemoteMachineSlashCommand(harness.deps, command("kernels", "machine-1"))

  assert.match(harness.footers.at(0)?.message ?? "", /cold id=machine-2 status=pending/)
  assert.match(harness.footers.at(0)?.message ?? "", /next: approve with \/machine approve machine-2/)
  assert.match(harness.footers.at(1)?.message ?? "", /cold-kernel id=kernel-2/)
  assert.match(harness.footers.at(1)?.message ?? "", /readiness=blocked/)
  assert.match(harness.footers.at(1)?.message ?? "", /accepting_remote_leases=false/)
  assert.match(harness.footers.at(1)?.message ?? "", /next: run \/machine kernels machine-1; enable remote leases on cold-kernel or choose another worker/)
  assert.deepEqual(harness.notices, [])
  assert.deepEqual(harness.notices, [])
})

test("remote machine command renders unknown lease state with refresh recovery", async () => {
  const harness = remoteMachineHarness({
    accepting_remote_leases: undefined,
    available_providers: ["codex"],
  })

  await handleRemoteMachineSlashCommand(harness.deps, command("kernels", "machine-1"))

  assert.match(harness.footers.at(0)?.message ?? "", /accepting_remote_leases=unknown/)
  assert.match(harness.footers.at(0)?.message ?? "", /readiness=unknown/)
  assert.doesNotMatch(harness.footers.at(0)?.message ?? "", /enable remote leases/)
  assert.match(harness.footers.at(0)?.message ?? "", /next: run \/machine kernels machine-1; refresh cold-kernel readiness or reconnect that worker before launching remote agents/)
})

test("remote machine command prioritizes unknown readiness over empty providers", async () => {
  const harness = remoteMachineHarness({
    accepting_remote_leases: undefined,
    available_providers: [],
  })

  await handleRemoteMachineSlashCommand(harness.deps, command("kernels", "machine-1"))

  assert.match(harness.footers.at(0)?.message ?? "", /readiness=unknown/)
  assert.match(harness.footers.at(0)?.message ?? "", /providers=-/)
  assert.match(harness.footers.at(0)?.message ?? "", /next: run \/machine kernels machine-1; refresh cold-kernel readiness or reconnect that worker before launching remote agents/)
  assert.doesNotMatch(harness.footers.at(0)?.message ?? "", /configure provider CLIs/)
})

test("remote machine mutation commands patch cached machines without refreshing inventory", async () => {
  const harness = remoteMachineHarness()

  await handleRemoteMachineSlashCommand(harness.deps, command("approve", "machine-2"))
  await handleRemoteMachineSlashCommand(harness.deps, command("forget", "machine-2"))

  assert.deepEqual(harness.machines.map((machine) => [
    machine.machine_id,
    machine.display_name,
    machine.trust_status,
  ]), [
    ["machine-1", "Builder", "approved"],
  ])
  assert.equal(harness.reconcileCount, 2)
  assert.equal(harness.footers.at(0)?.message, "approved remote machine cold")
  assert.equal(harness.footers.at(1)?.message, "forgot remote machine New Name")
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
    display_name: "Builder",
    trust_status: "approved" as const,
    online: true,
    pending: false,
    kernel_count: 1,
    available_providers: ["codex"],
  }, {
    machine_id: "machine-2",
    machine_alias: "cold",
    display_name: "cold",
    trust_status: "pending" as const,
    online: true,
    pending: true,
    kernel_count: 0,
    available_providers: [],
  }]
  const deps: RemoteMachineCommandHandlerDeps & { appendNotice: (message: string) => void } = {
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

test("machine add shows the single-use enrollment command in a toast, readable and pasteable whole", async () => {
  let asked = 0
  const footers: { message: string; tone: string; display: string | undefined }[] = []
  const notices: string[] = []
  const command = " (setup_script=$(curl -fsSL https://dl.example/install.sh) && printf '%s\\n' ket_abc | sh -c \"$setup_script\" sh --enroll --api-url https://example.test)"
  const deps = { flashFooter: (message: string, tone: string, display?: string) => footers.push({ message, tone, display }), appendNotice: (m: string) => notices.push(m),
    issueOwnMachineEnrollment: async () => { asked++; return { command, expires_at: "2026-11-03T13:10:00Z" } },
  }
  await handleRemoteMachineSlashCommand(deps, { kind: "machine", raw: "/machine add", args: ["add"] })
  await handleRemoteMachineSlashCommand(deps, { kind: "machine", raw: "/machine add", args: ["add"] })
  assert.equal(asked, 2, "the machine names itself: no name is asked for")
  assert.equal(footers.length, 2)
  // The footer line would clip the command; a toast shows all of it.
  assert.deepEqual(footers.map(f => f.display), ["toast", "toast"])
  const shown = footers[0]!.message.split("\n").slice(1)
  assert.ok(shown.length > 1 && shown.slice(0, -1).every(line => line.endsWith(" \\")), "each broken line ends in a shell continuation")
  assert.equal(shown.join("\n").replaceAll(" \\\n", " "), command.trim())
  assert.deepEqual(notices, [])
})

test("machine add says why the command could not be prepared", async () => {
  const footers: { message: string; tone: string }[] = []
  const deps = { flashFooter: (message: string, tone: string) => footers.push({ message, tone }), appendNotice: () => {},
    issueOwnMachineEnrollment: async () => { throw new Error("this kernel is not signed in to Chariox; sign in before enrolling another machine") },
  }
  await handleRemoteMachineSlashCommand(deps, { kind: "machine", raw: "/machine add", args: ["add"] })
  assert.deepEqual(footers, [{ message: "this kernel is not signed in to Chariox; sign in before enrolling another machine", tone: "error" }])
})

test("MP-08/MP-11 machine results never enter the agent transcript", async () => {
  const harness = remoteMachineHarness()
  await handleRemoteMachineSlashCommand(harness.deps, command("list"))
  await handleRemoteMachineSlashCommand(harness.deps, command("kernels", "machine-1"))
  assert.deepEqual(harness.notices, [])
  assert.ok(harness.footers.some(f => f.message.includes("cold id=machine-2")))
  assert.ok(harness.footers.some(f => f.message.includes("cold-kernel id=kernel-2")))
})

test("a machine whose name has spaces is listed through the command the machine row stages", async () => {
  const decision = deriveWaitingRoomControlActivationDecision({
    state: { focus: "machine", machineIndex: 0 } as never,
    workspacePath: "/workspace",
    worktreePath: "/workspace",
    remote: {
      machines: [{
        machine_id: "machine-3c9f",
        machine_alias: "Miguel's Mac Studio",
        display_name: "Miguel's Mac Studio",
        trust_status: "approved",
        online: true,
        pending: false,
        kernel_count: 2,
        available_providers: [],
      }],
    },
  })
  assert.equal(decision.action, "stage-command")
  const parsed = parseSlashCommand((decision as { command: string }).command)
  assert.equal(parsed?.kind, "machine")
  const { deps } = remoteMachineHarness()
  const asked: Array<string | undefined> = []
  const listed = deps.listRemoteMachineKernels!
  deps.listRemoteMachineKernels = async (machineRef) => { asked.push(machineRef); return listed(machineRef) }
  await handleRemoteMachineSlashCommand(deps, parsed as ReturnType<typeof command>)
  assert.deepEqual(asked, ["machine-3c9f"])
})
