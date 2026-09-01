import type { KernelEvent } from "@chariox/kernel-client/kernel-events"

import type { LocalIpcClient } from "./ipc.js"

type KernelEventHandler = (event: KernelEvent) => void

export type MutableLocalIpcClient = LocalIpcClient & {
  currentClient: () => LocalIpcClient
  replaceClient: (nextClient: LocalIpcClient) => Promise<void>
  swapClient: (nextClient: LocalIpcClient) => LocalIpcClient
}

export type MutableLocalIpcClientPivot = {
  commit: () => Promise<void>
  rollback: () => Promise<void>
}

export function beginMutableLocalIpcClientPivot(
  client: MutableLocalIpcClient,
  nextClient: LocalIpcClient,
): MutableLocalIpcClientPivot {
  const previousClient = client.swapClient(nextClient)
  let settled = false
  return {
    commit: async () => {
      if (settled) return
      settled = true
      if (client.currentClient() !== previousClient) {
        await previousClient.close()
      }
    },
    rollback: async () => {
      if (settled) return
      settled = true
      const currentClient = client.currentClient()
      if (currentClient === nextClient) {
        const replacedClient = client.swapClient(previousClient)
        await replacedClient.close()
      } else if (currentClient !== previousClient) {
        await previousClient.close()
      }
    },
  }
}

export function createMutableLocalIpcClient(initialClient: LocalIpcClient): MutableLocalIpcClient {
  let currentClient = initialClient
  const handlers = new Map<KernelEventHandler, () => void>()

  const bindHandler = (handler: KernelEventHandler) => currentClient.onKernelEvent(handler)

  const proxy = {
    get socketPath() {
      return currentClient.socketPath
    },
    currentClient: () => currentClient,
    async replaceClient(nextClient: LocalIpcClient) {
      if (nextClient === currentClient) {
        return
      }
      const previousClient = proxy.swapClient(nextClient)
      await previousClient.close()
    },
    swapClient(nextClient: LocalIpcClient) {
      if (nextClient === currentClient) {
        return currentClient
      }
      const previousClient = currentClient
      for (const dispose of handlers.values()) {
        dispose()
      }
      currentClient = nextClient
      for (const handler of handlers.keys()) {
        handlers.set(handler, bindHandler(handler))
      }
      return previousClient
    },
    supportsKernelEvents() {
      return currentClient.supportsKernelEvents()
    },
    send<TResponse>(request: unknown): Promise<TResponse> {
      return currentClient.send<TResponse>(request)
    },
    subscribeToKernelEvents(sessionId: string, attachmentId: string): Promise<void> {
      return currentClient.subscribeToKernelEvents(sessionId, attachmentId)
    },
    subscribeToWaitingRoomInventory(): Promise<void> {
      return currentClient.subscribeToWaitingRoomInventory()
    },
    unsubscribeFromKernelEvents(): Promise<void> {
      return currentClient.unsubscribeFromKernelEvents()
    },
    restartKernelEventStream(): Promise<void> {
      return currentClient.restartKernelEventStream()
    },
    onKernelEvent(handler: KernelEventHandler) {
      const dispose = bindHandler(handler)
      handlers.set(handler, dispose)
      return () => {
        const currentDispose = handlers.get(handler)
        handlers.delete(handler)
        currentDispose?.()
      }
    },
    close(): Promise<void> {
      return currentClient.close()
    },
    destroy(): void {
      currentClient.destroy()
    },
  }

  return proxy as MutableLocalIpcClient
}
