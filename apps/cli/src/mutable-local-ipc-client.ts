import type { KernelEvent } from "@arroba/kernel-client/kernel-events"

import type { LocalIpcClient } from "./ipc.js"

type KernelEventHandler = (event: KernelEvent) => void

export type MutableLocalIpcClient = LocalIpcClient & {
  currentClient: () => LocalIpcClient
  replaceClient: (nextClient: LocalIpcClient) => Promise<void>
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
      const previousClient = currentClient
      for (const dispose of handlers.values()) {
        dispose()
      }
      currentClient = nextClient
      for (const handler of handlers.keys()) {
        handlers.set(handler, bindHandler(handler))
      }
      await previousClient.close()
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
