export * from "@chariox/kernel-client/ipc"
export type * from "@chariox/kernel-client/ipc"

import { LocalIpcClient as KernelClient } from "@chariox/kernel-client/ipc"
import { kernelFeatureMinimum, requireKernelFeatureProtocol } from "./kernel-feature-minimum.js"
import { loadLocalKernelPresences, localKernelEndpoint, localKernelPresenceFreshnessMs } from "./local-kernel-presence.js"

type AdvertisedProtocol = { version: number | undefined; expiresAtMs: number }

export class LocalIpcClient extends KernelClient {
  private advertisedProtocol: AdvertisedProtocol | undefined
  private protocolLookup: Promise<AdvertisedProtocol> | undefined

  protected override onControlConnectionChanged(): void {
    this.clearAdvertisedProtocol()
  }

  override async send<TResponse>(request: unknown): Promise<TResponse> {
    if (kernelFeatureMinimum(request)) {
      requireKernelFeatureProtocol(request, (await this.kernelProtocol()).version)
    }
    return super.send<TResponse>(request)
  }

  override async close(): Promise<void> {
    this.clearAdvertisedProtocol()
    await super.close()
  }

  override destroy(): void {
    this.clearAdvertisedProtocol()
    super.destroy()
  }

  private clearAdvertisedProtocol(): void {
    this.advertisedProtocol = undefined
    this.protocolLookup = undefined
  }

  private async kernelProtocol(): Promise<AdvertisedProtocol> {
    if (this.advertisedProtocol && this.advertisedProtocol.expiresAtMs > Date.now()) return this.advertisedProtocol
    if (this.protocolLookup) return this.protocolLookup
    const lookup = this.resolveAdvertisedProtocol()
    this.protocolLookup = lookup
    try {
      const result = await lookup
      if (this.protocolLookup === lookup) this.advertisedProtocol = result
      return result
    } finally {
      if (this.protocolLookup === lookup) this.protocolLookup = undefined
    }
  }

  private async resolveAdvertisedProtocol(): Promise<AdvertisedProtocol> {
    const now = Date.now()
    // Unix admission is process-bound and may precede the access popup.
    // Let the kernel's decode/authorization diagnostics handle unknown versions.
    if (this.socketPath.startsWith("ws+unix://")) {
      return { version: undefined, expiresAtMs: now + localKernelPresenceFreshnessMs }
    }
    const presences = loadLocalKernelPresences(undefined, now)
    const presence = presences.find(value => localKernelEndpoint(value) === this.socketPath)
    return {
      version: presence?.protocolVersion,
      expiresAtMs: Math.min(now + localKernelPresenceFreshnessMs,
        (presence?.heartbeatAtMs ?? now) + localKernelPresenceFreshnessMs),
    }
  }
}
