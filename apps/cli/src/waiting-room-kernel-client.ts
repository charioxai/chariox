import { LocalIpcClient } from "./ipc.js"
import { loadLocalKernelPresences, localKernelEndpoint } from "./local-kernel-presence.js"
import { resolveKernelClientConnection } from "./relay-api.js"
import { beginMutableLocalIpcClientPivot, type MutableLocalIpcClient, type MutableLocalIpcClientPivot } from "./mutable-local-ipc-client.js"
import { getWaitingRoomInventory, type WaitingRoomInventory } from "./waiting-room-inventory-api.js"

type KernelClientTarget = {
  kernelRef: string
  machineRef: string | null
  clientId: string
  isActive(): boolean
}

async function openWaitingRoomKernelClient(controlClient: LocalIpcClient, target: KernelClientTarget) {
  const localPresence = loadLocalKernelPresences().find(presence => presence.kernelId === target.kernelRef)
  const connection = localPresence ? null : await resolveKernelClientConnection(controlClient, target)
  if (!target.isActive()) return null
  const client = localPresence
    ? new LocalIpcClient(localKernelEndpoint(localPresence))
    : new LocalIpcClient(connection!.relayUrl, {
        relayAuthToken: connection!.relayToken,
        targetDaemonId: connection!.targetDaemonId ?? undefined,
        targetDaemonAlias: connection!.targetDaemonAlias ?? undefined,
      })
  return { client, label: localPresence?.kernelAlias ?? connection?.targetDaemonAlias ?? connection?.kernelId ?? target.kernelRef }
}

export async function browseWaitingRoomKernelWorkspace(
  controlClient: LocalIpcClient,
  target: KernelClientTarget,
  applyInventory: (inventory: WaitingRoomInventory, client: LocalIpcClient) => Promise<void>,
) {
  const connection = await openWaitingRoomKernelClient(controlClient, target)
  if (!connection) return
  try {
    const inventory = await getWaitingRoomInventory(connection.client)
    if (!target.isActive()) return
    if (inventory.kernelId !== target.kernelRef || inventory.machineId !== target.machineRef) {
      throw new Error("workspace inventory identity does not match the selected managed machine")
    }
    await applyInventory(inventory, connection.client)
  } finally {
    await connection.client.close()
  }
}

export function createWaitingRoomKernelConnectionController(deps: {
  client: MutableLocalIpcClient
  clientId: string
  initialTargetKernelId?: string | null
  homeKernelId(): string | null
  homeMachineId(): string | null
  currentKernelId(): string | null | undefined
  invalidateInventory(): void
  clearProjectSetup(): void
  connected(label: string): void
}) {
  const homeDirectTargetKernelId = deps.initialTargetKernelId?.trim() || null
  let directTargetKernelId = homeDirectTargetKernelId
  const connect = async (
    kernelRef: string | null | undefined,
    machineRef: string | null | undefined,
    isActive: () => boolean = () => true,
    connected?: (inventory: WaitingRoomInventory) => void,
    retainPrevious?: (pivot: MutableLocalIpcClientPivot) => void,
  ): Promise<boolean> => {
    const targetKernelRef = kernelRef?.trim() === "local" ? deps.homeKernelId() : kernelRef?.trim()
    const currentKernelId = directTargetKernelId || deps.homeKernelId() || deps.currentKernelId()
    const sourceTargetKernelId = directTargetKernelId
    if (!targetKernelRef || targetKernelRef === "local" || targetKernelRef === currentKernelId) {
      if (connected && targetKernelRef && targetKernelRef !== "local") {
        const inventory = await getWaitingRoomInventory(deps.client)
        if (!isActive()) return false
        connected(inventory)
      }
      if (isActive() && targetKernelRef === deps.homeKernelId()) directTargetKernelId = homeDirectTargetKernelId
      return isActive()
    }
    const connection = await openWaitingRoomKernelClient(deps.client, {
      kernelRef: targetKernelRef,
      machineRef: machineRef === "local" ? deps.homeMachineId() : machineRef ?? null,
      clientId: deps.clientId,
      isActive,
    })
    if (!connection) return false
    const nextClient = connection.client
    if (typeof deps.client.replaceClient !== "function") {
      await nextClient.close()
      throw new Error("kernel client pivot is unavailable in this build")
    }
    let targetInventory: WaitingRoomInventory
    try {
      targetInventory = await getWaitingRoomInventory(nextClient)
    } catch (error) {
      await nextClient.close()
      throw error
    }
    if (!isActive()) {
      await nextClient.close()
      return false
    }
    if (retainPrevious) {
      if (typeof deps.client.swapClient !== "function") {
        await nextClient.close()
        throw new Error("transactional kernel client pivot is unavailable in this build")
      }
      const pivot = beginMutableLocalIpcClientPivot(deps.client, nextClient)
      try {
        directTargetKernelId = targetInventory.kernelId === deps.homeKernelId() ? homeDirectTargetKernelId : targetInventory.kernelId
        connected?.(targetInventory)
        retainPrevious({
          commit: async () => {
            try {
              await pivot.commit()
            } finally {
              deps.clearProjectSetup()
            }
          },
          rollback: async () => {
            try {
              await pivot.rollback()
            } finally {
              directTargetKernelId = sourceTargetKernelId
              deps.invalidateInventory()
            }
          },
        })
      } catch (error) {
        try {
          await pivot.rollback()
        } finally {
          directTargetKernelId = sourceTargetKernelId
        }
        throw error
      }
    } else {
      await deps.client.replaceClient(nextClient)
      deps.clearProjectSetup()
      directTargetKernelId = targetInventory.kernelId === deps.homeKernelId() ? homeDirectTargetKernelId : targetInventory.kernelId
      connected?.(targetInventory)
    }
    deps.invalidateInventory()
    deps.connected(connection.label)
    return isActive()
  }

  return {
    connect,
    getDirectTargetKernelId: () => directTargetKernelId,
    assertLocalReimageAuthority: () => {
      if (directTargetKernelId) {
        throw new Error("Return to the local kernel before controlling a managed-machine reimage.")
      }
    },
  }
}
