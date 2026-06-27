export type RemoteKernelReadiness = "ready" | "blocked" | "needs-provider" | "needs-account" | "unknown"

export type RemoteKernelProviderAccount = {
  provider: string
  state?: string | null
}

export type RemoteKernelReadinessInput = {
  accepting_remote_leases?: boolean
  available_providers?: string[]
  provider_accounts?: RemoteKernelProviderAccount[]
}

export function remoteKernelReadiness(kernel: RemoteKernelReadinessInput): RemoteKernelReadiness {
  if (kernel.accepting_remote_leases === false) return "blocked"
  const providers = kernel.available_providers ?? []
  if (providers.length === 0) {
    return kernel.accepting_remote_leases === undefined ? "unknown" : "needs-provider"
  }
  if (kernel.accepting_remote_leases === undefined) return "unknown"
  if (
    "provider_accounts" in kernel
    && providerAccountsNeedRecovery(providers, kernel.provider_accounts ?? [])
  ) {
    return "needs-account"
  }
  return "ready"
}

export function remoteKernelReadinessCounts(kernels: readonly RemoteKernelReadinessInput[]): Record<RemoteKernelReadiness, number> {
  return kernels.reduce<Record<RemoteKernelReadiness, number>>((counts, kernel) => {
    counts[remoteKernelReadiness(kernel)] += 1
    return counts
  }, { ready: 0, blocked: 0, "needs-provider": 0, "needs-account": 0, unknown: 0 })
}

function providerAccountsNeedRecovery(
  providers: readonly string[],
  accounts: readonly RemoteKernelProviderAccount[],
): boolean {
  return providers.some((provider) => {
    const account = accounts.find((entry) => entry.provider === provider)
    return !account || account.state !== "authenticated"
  })
}
