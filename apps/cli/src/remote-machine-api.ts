import type { RemoteMachineRecord as KernelRemoteMachineRecord } from "@chariox/kernel-client/kernel-types"
import type { LocalIpcClient } from "./ipc.js"
import type {
  WaitingRoomRemoteKernelView,
  WaitingRoomRemoteMachineView,
} from "./cli-types.js"
import {
  issueOwnMachineEnrollmentRequest,
  approveRemoteMachineRequest,
  forgetRemoteMachineRequest,
  listRemoteMachineKernelsRequest,
  listRemoteMachinesRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

export function remoteMachineView(record: KernelRemoteMachineRecord): WaitingRoomRemoteMachineView {
  const { presence, ...rest } = record
  return { ...rest, online: presence === "online" }
}

export async function listRemoteMachines(client: LocalIpcClient): Promise<WaitingRoomRemoteMachineView[]> {
  const response = await client.send<Record<string, unknown>>(listRemoteMachinesRequest())
  const payload = expectVariant<{
    machines: KernelRemoteMachineRecord[]
  }>(response, "RemoteMachinesListed")
  return payload.machines.map(remoteMachineView)
}

export async function approveRemoteMachine(
  client: LocalIpcClient,
  machineRef: string,
): Promise<WaitingRoomRemoteMachineView> {
  const response = await client.send<Record<string, unknown>>(approveRemoteMachineRequest(machineRef))
  return remoteMachineView(expectVariant<{ machine: KernelRemoteMachineRecord }>(response, "RemoteMachineApproved").machine)
}

export async function forgetRemoteMachine(
  client: LocalIpcClient,
  machineRef: string,
): Promise<{ machine_id: string; display_name: string }> {
  const response = await client.send<Record<string, unknown>>(forgetRemoteMachineRequest(machineRef))
  return expectVariant<{ machine_id: string; display_name: string }>(response, "RemoteMachineForgotten")
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

export type OwnMachineEnrollment = { command: string; expires_at: string }
export async function issueOwnMachineEnrollment(client: LocalIpcClient): Promise<OwnMachineEnrollment> {
  const response = await client.send<Record<string, unknown>>(issueOwnMachineEnrollmentRequest())
  return expectVariant<{ enrollment: OwnMachineEnrollment }>(response, "OwnMachineEnrollment").enrollment
}
