import type { GatewayKernelRecord } from "@chariox/kernel-client/kernel-types"
import type { LocalIpcClient } from "./ipc.js"
import { forgetGatewayKernelRequest, joinKernelLinkRequest, listGatewayKernelsRequest } from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

export async function joinKernelLink(client: LocalIpcClient, link: string, relayToken: string | null): Promise<GatewayKernelRecord> {
  const response = await client.send<Record<string, unknown>>(joinKernelLinkRequest(link, relayToken))
  return expectVariant<{ kernel: GatewayKernelRecord }>(response, "GatewayKernelJoined").kernel
}

export async function listGatewayKernels(client: LocalIpcClient): Promise<GatewayKernelRecord[]> {
  const response = await client.send<Record<string, unknown>>(listGatewayKernelsRequest())
  return expectVariant<{ kernels: GatewayKernelRecord[] }>(response, "GatewayKernelsListed").kernels
}

export async function forgetGatewayKernel(client: LocalIpcClient, name: string): Promise<void> {
  const response = await client.send<Record<string, unknown>>(forgetGatewayKernelRequest(name))
  expectVariant<{ kernel_id: string }>(response, "GatewayKernelForgotten")
}
