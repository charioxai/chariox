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
  readonly environmentId: string
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

export function startManagedContextTransferRequest(ticket: ManagedContextTransferTicket) {
  return { StartManagedContextTransfer: { ticket } } as const
}

export function getManagedContextTransferStatusRequest(contextId: string) {
  return { GetManagedContextTransferStatus: { contextId } } as const
}

export function getManagedContextLaunchTargetRequest(contextId: string, planDigest: string) {
  return { GetManagedContextLaunchTarget: { contextId, planDigest } } as const
}
