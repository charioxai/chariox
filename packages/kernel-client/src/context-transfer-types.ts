export type ContextRepositorySelection = {
  readonly role: "primary" | "supporting"
  readonly workspaceId: string
  readonly worktreeId: string | null
}

export type ContextDevelopmentSetup =
  | { readonly kind: "empty" }
  | {
      readonly kind: "source_project"
      readonly projectId: string
      readonly repositories: readonly ContextRepositorySelection[]
    }

export type ProviderAccountTransferSelection =
  | { readonly kind: "none" }
  | {
      readonly kind: "selected"
      readonly accounts: readonly {
        readonly provider: string
        readonly accountProfile: string
      }[]
    }

export type GitCredentialTransferSelection =
  | { readonly kind: "none" }
  | { readonly kind: "selected"; readonly credentialIds: readonly string[] }

export type ContextTransferPlanInput = {
  readonly sourceTargetId: string | null
  readonly kernelContext: "empty" | "source_kernel"
  readonly developmentSetup: ContextDevelopmentSetup
  readonly providerAccounts: ProviderAccountTransferSelection
  readonly gitCredentials: GitCredentialTransferSelection
}

export type ContextTransferPlan = Omit<ContextTransferPlanInput, "sourceTargetId"> & {
  readonly schemaVersion: 1
  readonly contextId: string
  readonly planDigest: string
  readonly source: {
    readonly sourceTargetId: string
    readonly relayRealmId: string
    readonly machineId: string
    readonly kernelId: string
    readonly keyThumbprint: string
  } | null
}

export type ManagedContextTransferTicket = {
  readonly environmentId: string
  readonly contextPlan: ContextTransferPlan
  readonly target: {
    readonly relayRealmId: string
    readonly machineId: string
    readonly kernelId: string
    readonly relayPublicKey: string
    readonly keyThumbprint: string
  }
}
