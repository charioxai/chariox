export * from "@chariox/kernel-client/ipc"
export type * from "@chariox/kernel-client/ipc"

import { LocalIpcClient as KernelClient } from "@chariox/kernel-client/ipc"
import { kernelFeatureMinimum, requireKernelFeatureProtocol } from "./kernel-feature-minimum.js"
import { loadLocalKernelPresences, localKernelEndpoint } from "./local-kernel-presence.js"

export class LocalIpcClient extends KernelClient {
  override async send<TResponse>(request: unknown): Promise<TResponse> {
    if (kernelFeatureMinimum(request)) {
      const presences = loadLocalKernelPresences()
      let version = presences.find(value => localKernelEndpoint(value) === this.socketPath)?.protocolVersion
      if (version === undefined && !/^wss?:\/\//i.test(this.socketPath)
        && presences.some(value => value.protocolVersion !== undefined)) {
        const reply = await super.send<{ RelayStatus?: { status?: { daemon_id?: string } } }>({ RelayStatus: null })
        version = presences.find(value => value.kernelId === reply.RelayStatus?.status?.daemon_id)?.protocolVersion
      }
      requireKernelFeatureProtocol(request, version)
    }
    return super.send<TResponse>(request)
  }
}
