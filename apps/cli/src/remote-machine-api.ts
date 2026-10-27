import type { LocalIpcClient } from "./ipc.js"
import type {
  WaitingRoomRemoteKernelView,
  WaitingRoomRemoteMachineView,
} from "./cli-types.js"
import {
  addSshMachineRequest,
  removeSshMachineRequest,
  approveRemoteMachineRequest,
  forgetRemoteMachineRequest,
  listRemoteMachineKernelsRequest,
  listRemoteMachinesRequest,
  renameRemoteMachineRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

export async function listRemoteMachines(client: LocalIpcClient): Promise<WaitingRoomRemoteMachineView[]> {
  const response = await client.send<Record<string, unknown>>(listRemoteMachinesRequest())
  const payload = expectVariant<{
    machines: WaitingRoomRemoteMachineView[]
  }>(response, "RemoteMachinesListed")
  return payload.machines
}

export async function approveRemoteMachine(
  client: LocalIpcClient,
  machineRef: string,
): Promise<WaitingRoomRemoteMachineView> {
  const response = await client.send<Record<string, unknown>>(approveRemoteMachineRequest(machineRef))
  return expectVariant<{ machine: WaitingRoomRemoteMachineView }>(response, "RemoteMachineApproved").machine
}

export async function forgetRemoteMachine(
  client: LocalIpcClient,
  machineRef: string,
): Promise<WaitingRoomRemoteMachineView> {
  const response = await client.send<Record<string, unknown>>(forgetRemoteMachineRequest(machineRef))
  return expectVariant<{ machine: WaitingRoomRemoteMachineView }>(response, "RemoteMachineForgotten").machine
}

export async function renameRemoteMachine(
  client: LocalIpcClient,
  machineRef: string,
  alias: string,
): Promise<WaitingRoomRemoteMachineView> {
  const response = await client.send<Record<string, unknown>>(renameRemoteMachineRequest(machineRef, alias))
  return expectVariant<{ machine: WaitingRoomRemoteMachineView }>(response, "RemoteMachineRenamed").machine
}

export async function listRemoteMachineKernels(
  client: LocalIpcClient,
  machineRef: string,
): Promise<WaitingRoomRemoteKernelView[]> {
  const response = await client.send<Record<string, unknown>>(listRemoteMachineKernelsRequest(machineRef))
  const payload = expectVariant<{
    kernels: WaitingRoomRemoteKernelView[]
  }>(response, "RemoteMachineKernelsListed")
  return payload.kernels
}

// MP-07 / MP-08 / MP-11: the kernel owns SSH, release admission and enrollment.
export type SshMachineResult = {
  install_id: string; status: string; kernel_id: string | null; machine_id: string | null
  release_digest: string; state_retained: boolean
}
export async function addSshMachine(client: LocalIpcClient, host: string, options: { install_id?: string; port?: number; release?: string }): Promise<SshMachineResult> {
  const response = await client.send<Record<string, unknown>>(addSshMachineRequest(host, options))
  return expectVariant<{ machine: SshMachineResult }>(response, "SshMachine").machine
}
export async function removeSshMachine(client: LocalIpcClient, installId: string): Promise<SshMachineResult> {
  const response = await client.send<Record<string, unknown>>(removeSshMachineRequest(installId))
  return expectVariant<{ machine: SshMachineResult }>(response, "SshMachine").machine
}
