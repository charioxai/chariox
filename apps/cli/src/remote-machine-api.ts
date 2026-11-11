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
