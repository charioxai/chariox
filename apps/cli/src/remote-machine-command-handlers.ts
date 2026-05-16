import type { ParsedSlashCommand } from "./commands.js"

type FooterTone = "info" | "error"

type RemoteMachineSummary = {
  machine_id: string
  machine_alias?: string | null
  registry_alias?: string | null
  display_name?: string
  trust_status?: "approved" | "pending" | "forgotten"
  online?: boolean
  pending?: boolean
  kernel_count: number
  available_providers?: string[]
}

type RemoteMachineRegistration = {
  machine_id: string
  display_name?: string
  trust_status?: "approved" | "pending" | "forgotten"
  online?: boolean
}

type RemoteMachineKernelSummary = {
  kernel_id: string
  machine_id: string
  machine_alias?: string | null
  relay_alias?: string | null
  kernel_alias?: string | null
  available_providers?: string[]
  capabilities?: string[]
  accepting_remote_leases?: boolean
  leased_agent_count?: number
  local_session_count?: number
}

export type RemoteMachineCommandHandlerDeps = {
  flashFooter: (message: string, tone: FooterTone) => void
  appendNotice: (message: string) => void
  refreshWaitingRoomData?: () => Promise<void>
  listRemoteMachines?: () => Promise<RemoteMachineSummary[]>
  approveRemoteMachine?: (machineRef: string) => Promise<RemoteMachineRegistration>
  forgetRemoteMachine?: (machineRef: string) => Promise<RemoteMachineRegistration>
  renameRemoteMachine?: (machineRef: string, alias: string) => Promise<RemoteMachineRegistration>
  listRemoteMachineKernels?: (machineRef: string) => Promise<RemoteMachineKernelSummary[]>
}

export async function handleRemoteMachineSlashCommand(
  deps: RemoteMachineCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "machine" }>,
): Promise<void> {
  const args = command.args
  const subcommand = args[0]
  if (subcommand === "list") {
    await listRemoteMachines(deps)
    return
  }
  if (subcommand === "kernels") {
    await listRemoteMachineKernels(deps, args[1])
    return
  }
  if (subcommand === "approve") {
    await approveRemoteMachine(deps, args[1])
    return
  }
  if (subcommand === "forget") {
    await forgetRemoteMachine(deps, args[1])
    return
  }
  if (subcommand === "rename") {
    await renameRemoteMachine(deps, args[1], args.slice(2).join(" ").trim())
    return
  }
  deps.flashFooter("usage: /machine list | /machine kernels <machine-ref> | /machine approve <machine-ref> | /machine forget <machine-ref> | /machine rename <machine-ref> <alias>", "error")
}

async function listRemoteMachines(deps: RemoteMachineCommandHandlerDeps): Promise<void> {
  if (!deps.listRemoteMachines) {
    deps.flashFooter("remote machine discovery is unavailable in this build", "error")
    return
  }
  const machines = await deps.listRemoteMachines()
  if (machines.length === 0) {
    deps.flashFooter("no live remote machines available through relay", "info")
    return
  }
  deps.appendNotice(machines.map(formatRemoteMachineSummary).join("\n"))
  deps.flashFooter(`listed ${machines.length} live remote machine(s)`, "info")
}

function formatRemoteMachineSummary(machine: RemoteMachineSummary): string {
  return `${machine.display_name ?? machine.machine_alias ?? "-"} id=${machine.machine_id} status=${machine.trust_status ?? "pending"}${machine.online === false ? ",offline" : ""} kernels=${machine.kernel_count} providers=${(machine.available_providers ?? []).join(",") || "-"}`
}

async function listRemoteMachineKernels(
  deps: RemoteMachineCommandHandlerDeps,
  machineRef: string | undefined,
): Promise<void> {
  if (!deps.listRemoteMachineKernels) {
    deps.flashFooter("remote machine discovery is unavailable in this build", "error")
    return
  }
  if (!machineRef) {
    deps.flashFooter("usage: /machine kernels <machine-ref>", "error")
    return
  }
  const kernels = await deps.listRemoteMachineKernels(machineRef)
  if (kernels.length === 0) {
    deps.flashFooter(`no live kernels found for machine ${machineRef}`, "info")
    return
  }
  deps.appendNotice(kernels.map(formatRemoteMachineKernelSummary).join("\n"))
  deps.flashFooter(`listed ${kernels.length} live kernel(s) for ${machineRef}`, "info")
}

function formatRemoteMachineKernelSummary(kernel: RemoteMachineKernelSummary): string {
  const displayName = kernel.relay_alias ?? kernel.kernel_alias ?? "-"
  const kernelAlias =
    kernel.kernel_alias && kernel.kernel_alias !== displayName
      ? ` kernel_alias=${kernel.kernel_alias}`
      : ""
  return `${displayName} id=${kernel.kernel_id}${kernelAlias} machine=${kernel.machine_alias ?? kernel.machine_id} providers=${(kernel.available_providers ?? []).join(",") || "-"} accepting_remote_leases=${String(kernel.accepting_remote_leases ?? false)} leased_agents=${kernel.leased_agent_count ?? 0} local_sessions=${kernel.local_session_count ?? 0}`
}

async function approveRemoteMachine(
  deps: RemoteMachineCommandHandlerDeps,
  machineRef: string | undefined,
): Promise<void> {
  if (!deps.approveRemoteMachine) {
    deps.flashFooter("remote machine registration is unavailable in this build", "error")
    return
  }
  if (!machineRef) {
    deps.flashFooter("usage: /machine approve <machine-ref>", "error")
    return
  }
  const machine = await deps.approveRemoteMachine(machineRef)
  await deps.refreshWaitingRoomData?.()
  deps.flashFooter(`approved remote machine ${machine.display_name ?? machine.machine_id}`, "info")
}

async function forgetRemoteMachine(
  deps: RemoteMachineCommandHandlerDeps,
  machineRef: string | undefined,
): Promise<void> {
  if (!deps.forgetRemoteMachine) {
    deps.flashFooter("remote machine registration is unavailable in this build", "error")
    return
  }
  if (!machineRef) {
    deps.flashFooter("usage: /machine forget <machine-ref>", "error")
    return
  }
  const machine = await deps.forgetRemoteMachine(machineRef)
  await deps.refreshWaitingRoomData?.()
  deps.flashFooter(`forgot remote machine ${machine.display_name ?? machine.machine_id}`, "info")
}

async function renameRemoteMachine(
  deps: RemoteMachineCommandHandlerDeps,
  machineRef: string | undefined,
  alias: string,
): Promise<void> {
  if (!deps.renameRemoteMachine) {
    deps.flashFooter("remote machine registration is unavailable in this build", "error")
    return
  }
  if (!machineRef || !alias) {
    deps.flashFooter("usage: /machine rename <machine-ref> <alias>", "error")
    return
  }
  const machine = await deps.renameRemoteMachine(machineRef, alias)
  await deps.refreshWaitingRoomData?.()
  deps.flashFooter(`renamed remote machine ${machine.machine_id} to ${machine.display_name ?? alias}`, "info")
}
