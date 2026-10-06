import type { ManagedContextTransferTicket } from "./ipc-managed-environment-requests.js"

export type ManagedContextTransferPhase =
  | "preparing"
  | "uploading"
  | "importing"
  | "completed"
  | "failed"

export type ManagedContextTransferStatus = {
  readonly contextId: string
  readonly planDigest: string
  readonly phase: ManagedContextTransferPhase
  readonly acceptedBytes: number
  readonly packageSizeBytes: number
  readonly receipt?: unknown
  readonly failureCode?: string
  readonly failureMessage?: string
  readonly retryable: boolean
  readonly updatedAtMs: number
}

export type ManagedContextLaunchTarget = {
  readonly environmentId?: string
  readonly destination?: OwnerManagedDestination
  readonly kernelId: string
  readonly contextId: string
  readonly planDigest: string
  readonly development:
    | { readonly kind: "empty"; readonly workspacePath: string }
    | {
        readonly kind: "from_source"
        readonly projectId: string
        readonly destinationRoot: string
        readonly primaryRepositoryId: string
        readonly repositories: readonly {
          readonly repositoryId: string
          readonly role: "primary" | "supporting"
          readonly targetDirectory: string
          readonly workspacePath: string
          readonly headSha: string
        }[]
      }
}

// MP-08 / MP-11: clients select inventory only; the kernel pins source/owner/plan.
export const OWNER_MANAGED_CONTEXT_CAPABILITY = "owner_managed_context_transfer_v1"
export const OWNER_MANAGED_CONTEXT_MINIMUM_PROTOCOL_VERSION = 445
export const OWNER_MANAGED_CONTEXT_MINIMUM_RELAY_PROTOCOL_VERSION = 88

export type OwnerManagedDestination = {
  readonly kind: "owner_managed_machine"
  readonly machineId: string
  readonly kernelId: string
}

export type OwnerManagedContextTransfer = {
  readonly target: {
    readonly relayRealmId: string
    readonly machineId: string
    readonly kernelId: string
    readonly relayPublicKey: string
    readonly keyThumbprint: string
  }
  readonly contextSelection: {
    readonly kernelContext: "empty" | "source_kernel_without_credentials"
    readonly developmentSetup:
      | { readonly kind: "empty" }
      | {
          readonly kind: "source_project"
          readonly projectId: string
          readonly repositories: readonly {
            readonly role: "primary" | "supporting"
            readonly workspaceId: string
            readonly worktreeId: string | null
          }[]
        }
  }
}

export function startOwnerManagedContextTransferRequest(ownerManaged: OwnerManagedContextTransfer) {
  return { StartManagedContextTransfer: { interactive: true, ownerManaged } } as const
}

export function startManagedContextTransferRequest(ticket: ManagedContextTransferTicket, interactive = false) {
  return { StartManagedContextTransfer: { ticket, ...(interactive ? { interactive: true } : {}) } } as const
}

export function getManagedContextTransferStatusRequest(contextId: string) {
  return { GetManagedContextTransferStatus: { contextId } } as const
}

export function getManagedContextLaunchTargetRequest(contextId: string, planDigest: string) {
  return { GetManagedContextLaunchTarget: { contextId, planDigest } } as const
}
