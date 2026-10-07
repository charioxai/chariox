import type { CloudClient } from "./cloud-client.js"
import { createCliRelayIdentityStore } from "./cli-relay-identity-store.js"
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

async function openWaitingRoomKernelClient(controlClient: LocalIpcClient, target: KernelClientTarget, cloud?: { client?: CloudClient | undefined; kernelConnected?: (() => boolean) | undefined }) {
  const issuingClient = "currentClient" in controlClient && typeof controlClient.currentClient === "function"
    ? controlClient.currentClient() as LocalIpcClient : controlClient
  const localPresence = loadLocalKernelPresences().find(presence => presence.kernelId === target.kernelRef)
  const useCloudClient = !localPresence && cloud?.client && !cloud.kernelConnected?.() && await cloud.client.profile()
  const cloudTargetClient = useCloudClient ? await cloud!.client!.connect(target.kernelRef) : null
  const connection = localPresence || cloudTargetClient ? null : await resolveKernelClientConnection(issuingClient, target)
  if (!target.isActive()) { await cloudTargetClient?.close(); return null }
  const client = cloudTargetClient ?? (localPresence
    ? new LocalIpcClient(localKernelEndpoint(localPresence))
    : new LocalIpcClient(connection!.relayUrl, {
        relayAuthToken: connection!.relayToken,
        relayIdentity: createCliRelayIdentityStore().getOrCreate(),
        targetDaemonId: connection!.targetDaemonId ?? undefined,
        targetDaemonAlias: connection!.targetDaemonAlias ?? undefined,
      }))
  if (connection?.tokenExpiresAtMs) {
    const release = issuingClient.retainForRelayRenewal()
    client.startRelayAuthRenewal(connection.tokenExpiresAtMs, async () => {
      const fresh = await resolveKernelClientConnection(issuingClient, {
        kernelRef: connection.kernelId ?? connection.targetDaemonId ?? target.kernelRef,
        machineRef: connection.machineId ?? target.machineRef,
        clientId: target.clientId,
      })
      return { token: fresh.relayToken, expiresAtMs: fresh.tokenExpiresAtMs! }
    }, release)
  }
  return { client, label: localPresence?.kernelAlias ?? connection?.targetDaemonAlias ?? connection?.kernelId ?? target.kernelRef,
    machineId: localPresence?.machineId ?? connection?.machineId, kernelId: localPresence?.kernelId ?? connection?.kernelId }
}

export async function withWaitingRoomWorkspaceClient<T>(
  controlClient: LocalIpcClient,
  target: KernelClientTarget,
  read: (client: LocalIpcClient) => Promise<T>,
): Promise<T | undefined> {
  const connection = await openWaitingRoomKernelClient(controlClient, target)
  if (!connection) return undefined
  try {
    if (connection.kernelId !== target.kernelRef || target.machineRef && connection.machineId !== target.machineRef) {
      throw new Error("workspace connection identity does not match the selected kernel")
    }
    return await read(connection.client)
  } finally {
    await connection.client.close()
  }
}

export async function browseWaitingRoomKernelWorkspace(
  controlClient: LocalIpcClient,
  target: KernelClientTarget,
  applyInventory: (inventory: WaitingRoomInventory, client: LocalIpcClient) => Promise<void>,
  getInventory: (client: LocalIpcClient) => Promise<WaitingRoomInventory> = getWaitingRoomInventory,
) {
  const connection = await openWaitingRoomKernelClient(controlClient, target)
  if (!connection) return
  try {
    const inventory = await getInventory(connection.client)
    if (!target.isActive()) return
    if (inventory.kernelId !== target.kernelRef || inventory.machineId !== target.machineRef) {
      throw new Error("workspace inventory identity does not match the selected managed machine")
    }
    await applyInventory(inventory, connection.client)
  } catch (error) {
    if (target.isActive()) throw error
  } finally {
    await connection.client.close()
  }
}

export function createWaitingRoomKernelConnectionController(deps: {
  client: MutableLocalIpcClient
  cloudClient?: CloudClient | undefined
  kernelConnected?: () => boolean
  getInventory?: (client: LocalIpcClient) => Promise<WaitingRoomInventory>
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
  let connectedMachineId: string | null = null
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
    const sourceMachineId = connectedMachineId
    if (!targetKernelRef || targetKernelRef === "local" || targetKernelRef === currentKernelId) {
      if (connected && targetKernelRef && targetKernelRef !== "local") {
        const inventory = await (deps.getInventory ?? getWaitingRoomInventory)(deps.client)
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
    }, { client: deps.cloudClient, kernelConnected: deps.kernelConnected })
    if (!connection) return false
    const nextClient = connection.client
    if (typeof deps.client.replaceClient !== "function") {
      await nextClient.close()
      throw new Error("kernel client pivot is unavailable in this build")
    }
    let targetInventory: WaitingRoomInventory
    try {
      targetInventory = await (deps.getInventory ?? getWaitingRoomInventory)(nextClient)
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
        connectedMachineId = targetInventory.machineId
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
              connectedMachineId = sourceMachineId
              directTargetKernelId = sourceTargetKernelId
              deps.invalidateInventory()
            }
          },
        })
      } catch (error) {
        try {
          await pivot.rollback()
        } finally {
          connectedMachineId = sourceMachineId
          directTargetKernelId = sourceTargetKernelId
        }
        throw error
      }
    } else {
      await deps.client.replaceClient(nextClient)
      deps.clearProjectSetup()
      connectedMachineId = targetInventory.machineId
      directTargetKernelId = targetInventory.kernelId === deps.homeKernelId() ? homeDirectTargetKernelId : targetInventory.kernelId
      connected?.(targetInventory)
    }
    deps.invalidateInventory()
    deps.connected(connection.label)
    return isActive()
  }

  return {
    connect,
    readWorkspace: async (target: { machineId: string; kernelId: string; isActive(): boolean }, read: (client: LocalIpcClient) => Promise<void>) => {
      if (!target.machineId || !target.kernelId) throw new Error("Choose a connected kernel before reading its workspace.")
      if (!target.isActive()) return
      const currentKernelId = directTargetKernelId || deps.homeKernelId() || deps.currentKernelId()
      if (target.kernelId === currentKernelId && target.machineId === (connectedMachineId || deps.homeMachineId())) {
        await read(deps.client.currentClient())
      } else {
        await withWaitingRoomWorkspaceClient(deps.client, {
          kernelRef: target.kernelId, machineRef: target.machineId, clientId: deps.clientId, isActive: target.isActive,
        }, read)
      }
    },
    getDirectTargetKernelId: () => directTargetKernelId,
    assertLocalReimageAuthority: () => {
      if (directTargetKernelId) {
        throw new Error("Return to the local kernel before controlling a managed-machine reimage.")
      }
    },
  }
}
